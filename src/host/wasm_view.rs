// WasmView — wasmtime-backed runtime for Plexi v2 WASM component plugins.
//
// One `Store` per pane. The component exports the `lifecycle` interface
// (init/update/view) and imports host capabilities (host-log, host-state,
// pipes). Effects returned by init/update are executed by the host, which
// feeds results back as the next `input-event` — a synchronous Elm-style loop,
// no async runtime.
//
// Rendering (UiTree -> egui) and pane-tree wiring land in M3; this module is
// the runtime core validated by the G3 (effect roundtrip) and G5 (state
// persistence) gates against the sysmon POC, with no subprocess involved.

use std::collections::HashMap;

use crate::host::guest_text;
use std::path::{Path, PathBuf};

use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

// The crate itself is named `plexi`; bindgen also generates a top-level `plexi`
// module from the `plexi:platform` package. Nest the generated code so the two
// never collide at a use-path.
pub mod bindings {
    wasmtime::component::bindgen!({
        path: "wit/plexi.wit",
        world: "plexi-full-plugin",
    });
}

use bindings::exports::plexi::platform::audio_rt_process::{
    Guest as AudioProcessGuest, GuestIndices as AudioProcessIndices,
};
use bindings::exports::plexi::platform::lifecycle::{
    Guest as LifecycleGuest, GuestIndices as LifecycleIndices,
};
use bindings::plexi::platform::{audio_rt_control, gpu, host_log, host_state, pipes, types};

pub use types::{
    Alignment, BadgeColor, ButtonStyle, Color, Effect, IndexedNode, InputEvent, Justify, KeyEvent,
    Modifiers, MouseButton, MouseEvent, Point, StateSnapshot, SurfaceEvent, SystemStats,
    UiNodeData, UiTree,
};

impl UiTree {
    /// Every human-readable label in the tree — text, button, and badge nodes —
    /// newline-joined in arena order. The single text surface every content
    /// assertion reads, so a substring search matches the same content
    /// wherever it runs. Production reads rendered content through
    /// `SemanticPaneState`, which keeps node identity; this flattening exists
    /// for assertions, so it is test-only.
    #[cfg(test)]
    pub fn visible_text(&self) -> String {
        self.nodes
            .iter()
            .filter_map(|n| match &n.data {
                UiNodeData::Text(t) => Some(t.text.clone()),
                UiNodeData::Button(b) => Some(b.label.clone()),
                UiNodeData::Badge(b) => Some(b.text.clone()),
                UiNodeData::Table(t) if !t.columns.is_empty() => Some(t.columns.join("\n")),
                UiNodeData::TableRow(r) if !r.cells.is_empty() => Some(r.cells.join("\n")),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// The node ids `data` references as children, in paint order. The one edge
/// list every tree walk reads (decode validation, reachability, plugin check,
/// pane state), so a new container kind is wired everywhere by one match.
/// Container arms come first; the leaf arm opens with `Empty` (the SDK's
/// `test_container_table_covers_host_decoder` reads that boundary).
pub(crate) fn children(data: &UiNodeData) -> &[u32] {
    match data {
        UiNodeData::Row(row) => &row.children,
        UiNodeData::Column(column) => &column.children,
        UiNodeData::ListView(list) => &list.items,
        UiNodeData::Table(table) => &table.rows,
        UiNodeData::ListItem(item) => std::slice::from_ref(&item.child),
        UiNodeData::Scroll(scroll) => std::slice::from_ref(&scroll.child),
        UiNodeData::Padding(padding) => std::slice::from_ref(&padding.child),
        UiNodeData::Pinned(pinned) => std::slice::from_ref(&pinned.child),
        UiNodeData::Empty
        | UiNodeData::Text(_)
        | UiNodeData::Button(_)
        | UiNodeData::TextInput(_)
        | UiNodeData::ProgressBar(_)
        | UiNodeData::Badge(_)
        | UiNodeData::Canvas(_)
        | UiNodeData::Divider
        | UiNodeData::Space(_)
        | UiNodeData::Surface(_)
        | UiNodeData::SurfaceHeader(_)
        | UiNodeData::FooterKeys(_)
        | UiNodeData::Spinner(_)
        | UiNodeData::TableRow(_) => &[],
    }
}

// ─── Capability grants ─────────────────────────────────────────────────────
//
// host-log is always linked (logging is unconditionally safe). Every other
// host interface is linked only when granted. An plugin whose component imports an
// ungranted interface fails to instantiate — link-time gating, fail-fast.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Grants {
    pub state: bool,
    pub pipes: bool,
    pub gpu: bool,
    pub audio: bool,
}

impl Grants {
    pub fn capability_ids(&self) -> Vec<String> {
        let mut ids = Vec::new();
        if self.state {
            ids.push("state:read-write".to_string());
        }
        if self.pipes {
            ids.push("pipe.open".to_string());
        }
        if self.gpu {
            ids.push("gpu.render".to_string());
        }
        if self.audio {
            ids.push("audio.playback".to_string());
        }
        ids
    }
}

// ─── State store (G5) ──────────────────────────────────────────────────────
//
// Host-owned key/value store backing the `host-state` import. Persistent
// stores write the full map to a per-namespace JSON file on every mutation;
// ephemeral stores (e.g. `plexi run`) keep the map in memory only.

pub struct StateStore {
    data: HashMap<String, Vec<u8>>,
    path: Option<PathBuf>,
}

impl StateStore {
    pub fn ephemeral() -> Self {
        StateStore {
            data: HashMap::new(),
            path: None,
        }
    }

    /// Open (or create) a persistent store at `path`. Existing contents are
    /// loaded; a missing file starts empty.
    pub fn persistent(path: PathBuf) -> std::io::Result<Self> {
        let data = if path.exists() {
            let bytes = std::fs::read(&path)?;
            let parsed: HashMap<String, Vec<u8>> =
                serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
            log::info!(
                "{}",
                Self::format_load_trace(&path, Some((bytes.len(), parsed.len())))
            );
            parsed
        } else {
            log::info!("{}", Self::format_load_trace(&path, None));
            HashMap::new()
        };
        Ok(StateStore {
            data,
            path: Some(path),
        })
    }

    pub(crate) fn format_load_trace(path: &Path, hit: Option<(usize, usize)>) -> String {
        match hit {
            Some((bytes, entries)) => format!(
                "wasm_state: loaded state scope=context bytes={bytes} entries={entries} from {}",
                path.display()
            ),
            None => format!(
                "wasm_state: no state found for scope=context at {} (fallback=none)",
                path.display()
            ),
        }
    }

    pub(crate) fn format_persist_trace(path: &Path, bytes: usize, replaced: bool) -> String {
        format!(
            "wasm_state: persisted state scope=context bytes={bytes} replaced={replaced} to {}",
            path.display()
        )
    }

    fn persist(&self) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let replaced = path.exists();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let bytes = serde_json::to_vec(&self.data).map_err(|e| e.to_string())?;
        std::fs::write(path, &bytes).map_err(|e| e.to_string())?;
        log::info!(
            "{}",
            Self::format_persist_trace(path, bytes.len(), replaced)
        );
        Ok(())
    }

    pub fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.data.get(key).cloned()
    }

    pub fn set(&mut self, key: String, value: Vec<u8>) -> Result<(), String> {
        self.data.insert(key, value);
        self.persist()
    }

    pub fn delete(&mut self, key: &str) -> Result<(), String> {
        self.data.remove(key);
        self.persist()
    }

    pub fn list_prefix(&self, prefix: &str) -> Vec<String> {
        self.data
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect()
    }

    pub fn snapshot(&self) -> StateSnapshot {
        StateSnapshot {
            entries: self
                .data
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        }
    }
}

// ─── Host context (store data) ─────────────────────────────────────────────

/// A pipe handle the guest holds. Holds the registry id and kind so sends
/// route to the right backing: a binary ring, or the directed connection the
/// handle carries.
struct PipeBinding {
    id: String,
    binary: bool,
    /// The directed pipe connection this handle carries, for its whole
    /// lifetime (stint 1681). `None` for a pipe the guest opened itself.
    generation: Option<u64>,
}

pub struct HostCtx {
    plugin_id: String,
    state: StateStore,
    // Per-plugin typed-pipe registry backing the `pipes` import (G13). Binary
    // pipes get a unix socket + drain thread; the registry's Drop closes them
    // when the pane (and its Store) is dropped.
    pipes: crate::host::typed_pipes::TypedPipeRegistry,
    pipe_handles: HashMap<u32, PipeBinding>,
    next_pipe_handle: u32,
    // Directed pipe opens awaiting the host's answer, by host request id,
    // with the request number `open-directed` returned (stint 1681).
    pipe_requests: HashMap<String, u32>,
    next_pipe_request: u32,
    // Directed pipe opens, sends and closes the guest made through the
    // `pipes` import, for the host to drain; each names its connection.
    pipe_commands: Vec<crate::app::view_trait::ViewCommand>,
    // Open audio streams (G12). Backs the `audio-rt-control` import; sample
    // pulls happen through the guest's `process-output` export, not here.
    audio_streams: AudioStreams,
    // wgpu backend for the `gpu` import (G7/G11). Some only when the gpu
    // capability is granted; surface textures, buffers, pipelines, and bind
    // groups live in its handle registries.
    gpu: Option<crate::host::wasm_gpu::GpuDevice>,
    // Baseline WASI 0.2: clocks + random only. Rust-compiled components import
    // wasi:cli/io/clocks/random from std even when unused; a default ctx grants
    // no env, filesystem, or network access — the real platform capabilities
    // are the Plexi host-* interfaces, gated separately.
    wasi: WasiCtx,
    table: ResourceTable,
}

impl WasiView for HostCtx {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl HostCtx {
    /// A guest log message, escaped and capped where it arrives (stint 1344).
    fn guest_log(&self, msg: &str) -> String {
        crate::host::guest_text::guest_line(
            &self.plugin_id,
            "log message",
            msg,
            crate::host::guest_text::MAX_LINE_CHARS,
        )
    }
}

impl host_log::Host for HostCtx {
    fn debug(&mut self, msg: String) {
        log::debug!("plugin::{}: {}", self.plugin_id, self.guest_log(&msg));
    }
    fn info(&mut self, msg: String) {
        log::info!("plugin::{}: {}", self.plugin_id, self.guest_log(&msg));
    }
    fn warn(&mut self, msg: String) {
        log::warn!("plugin::{}: {}", self.plugin_id, self.guest_log(&msg));
    }
    fn error(&mut self, msg: String) {
        log::error!("plugin::{}: {}", self.plugin_id, self.guest_log(&msg));
    }
}

impl host_state::Host for HostCtx {
    fn get(&mut self, key: String) -> Option<Vec<u8>> {
        self.state.get(&key)
    }
    fn set(&mut self, key: String, value: Vec<u8>) -> Result<(), String> {
        self.state.set(key, value)
    }
    fn delete(&mut self, key: String) -> Result<(), String> {
        self.state.delete(&key)
    }
    fn list_prefix(&mut self, prefix: String) -> Vec<String> {
        self.state.list_prefix(&prefix)
    }
    fn snapshot(&mut self) -> StateSnapshot {
        self.state.snapshot()
    }
}

impl HostCtx {
    fn pipe_id(&self, handle: u32) -> Result<&PipeBinding, String> {
        self.pipe_handles
            .get(&handle)
            .ok_or_else(|| format!("unknown pipe handle {handle}"))
    }

    /// Register directed pipe connection `generation` under a fresh guest
    /// handle. A binary pipe already open under the same name refuses it: the
    /// two would share one id.
    pub fn register_directed_pipe(&mut self, pipe_id: &str, generation: u64) -> Result<(), String> {
        if self.pipes.is_binary(pipe_id) {
            return Err(format!(
                "'{}' is already open as a binary pipe on plugin {}",
                guest_text::shown(pipe_id),
                self.plugin_id
            ));
        }
        // A pipe the guest opened itself under this name would share the
        // registry entry, and undoing this registration would close it.
        if self
            .pipe_handles
            .values()
            .any(|binding| binding.generation.is_none() && binding.id == pipe_id)
        {
            return Err(format!(
                "'{}' is already open as a guest pipe on plugin {}",
                guest_text::shown(pipe_id),
                self.plugin_id
            ));
        }
        match self.pipes.open_json(
            pipe_id.to_string(),
            crate::host::typed_pipes::PipeDirection::Duplex,
        ) {
            Ok(()) | Err(crate::host::typed_pipes::PipeError::AlreadyOpen(_)) => {}
            Err(e) => {
                return Err(format!(
                    "registering directed pipe '{}' on plugin {}: {e}",
                    guest_text::shown(pipe_id),
                    self.plugin_id
                ));
            }
        }
        let handle = self.next_pipe_handle;
        self.next_pipe_handle += 1;
        self.pipe_handles.insert(
            handle,
            PipeBinding {
                id: pipe_id.to_string(),
                binary: false,
                generation: Some(generation),
            },
        );
        log::info!(
            "plugin::{}: registered directed pipe '{pipe_id}' connection {generation} as handle {handle}",
            self.plugin_id,
            pipe_id = guest_text::shown(pipe_id)
        );
        Ok(())
    }

    #[cfg(test)]
    pub fn has_pipe_reader(&self, pipe_id: &str) -> bool {
        self.pipes.has_reader(pipe_id)
    }

    /// Drop the handle carrying retired connection `generation`, and the JSON
    /// registry entry once no JSON handle uses the name. A binary pipe of the
    /// same name is never touched.
    pub fn close_directed_pipe(&mut self, pipe_id: &str, generation: u64) {
        self.pipe_handles
            .retain(|_, binding| binding.generation != Some(generation));
        if !self
            .pipe_handles
            .values()
            .any(|binding| !binding.binary && binding.id == pipe_id)
        {
            self.pipes.close_json(pipe_id);
        }
    }

    /// The guest handle carrying directed pipe connection `generation`.
    pub fn directed_pipe_handle(&self, generation: u64) -> Option<u32> {
        self.pipe_handles.iter().find_map(|(&handle, binding)| {
            (binding.generation == Some(generation)).then_some(handle)
        })
    }

    /// The request number `open-directed` returned for host request
    /// `request_id`, now answered. `None` for a request this guest never made.
    pub fn take_pipe_request(&mut self, request_id: &str) -> Option<u32> {
        self.pipe_requests.remove(request_id)
    }

    /// The directed pipe opens, sends and closes the guest made since the
    /// last call.
    pub fn take_pipe_commands(&mut self) -> Vec<crate::app::view_trait::ViewCommand> {
        std::mem::take(&mut self.pipe_commands)
    }
}

impl pipes::Host for HostCtx {
    fn open(
        &mut self,
        id: String,
        kind: pipes::PipeType,
        direction: pipes::PipeDirection,
    ) -> Result<u32, String> {
        use crate::host::typed_pipes::PipeDirection as Dir;
        let dir = match direction {
            pipes::PipeDirection::In => Dir::In,
            pipes::PipeDirection::Out => Dir::Out,
            pipes::PipeDirection::Duplex => Dir::Duplex,
        };
        let binary = matches!(kind, pipes::PipeType::Binary);
        if binary {
            self.pipes
                .open_binary(id.clone(), dir)
                .map_err(|e| e.to_string())?;
        } else {
            self.pipes
                .open_json(id.clone(), dir)
                .map_err(|e| e.to_string())?;
        }
        let handle = self.next_pipe_handle;
        self.next_pipe_handle += 1;
        self.pipe_handles.insert(
            handle,
            PipeBinding {
                id: id.clone(),
                binary,
                generation: None,
            },
        );
        log::info!(
            "plugin::{}: opened {} pipe '{id}' -> handle {handle}",
            self.plugin_id,
            if binary { "binary" } else { "json" },
            id = guest_text::shown(&id)
        );
        Ok(handle)
    }

    fn send_binary(&mut self, handle: u32, payload: Vec<u8>) -> Result<(), String> {
        let binding = self.pipe_id(handle)?;
        if !binding.binary {
            return Err(format!("pipe handle {handle} is a json pipe"));
        }
        let ring = self
            .pipes
            .binary_ring(&binding.id)
            .ok_or_else(|| format!("pipe '{}' has no binary ring", binding.id))?;
        // Lock-free push; a full ring means the peer is behind — drop the frame
        // (RT-safe, best-effort) and report overrun to the guest.
        ring.push(payload)
            .map_err(|_| "pipe ring full (overrun)".to_string())
    }

    fn open_directed(&mut self, id: String, target_pane: u64) -> Result<u32, String> {
        if id.is_empty() {
            return Err("directed pipe id is empty".to_string());
        }
        let request = self.next_pipe_request;
        self.next_pipe_request += 1;
        let request_id = format!("wasm-open-{request}");
        log::info!(
            "plugin::{}: open-directed '{id}' to pane {target_pane} as request {request}",
            self.plugin_id,
            id = guest_text::shown(&id)
        );
        self.pipe_requests.insert(request_id.clone(), request);
        self.pipe_commands
            .push(crate::app::view_trait::ViewCommand::OpenDirectedPipe {
                sender_pane_id: 0,
                request_id,
                pipe_id: id,
                target_pane_id: target_pane,
                emitter: None,
                admission: None,
            });
        Ok(request)
    }

    /// Queue the send on the connection the handle carries, fixed here at
    /// the import call. A handle whose connection closed is unknown.
    fn send_json(&mut self, handle: u32, json: String) -> Result<(), String> {
        let binding = self.pipe_id(handle)?;
        if binding.binary {
            return Err(format!("pipe handle {handle} is a binary pipe"));
        }
        let Some(generation) = binding.generation else {
            return Err(format!(
                "pipe handle {handle} is not a directed connection; open one with open-directed"
            ));
        };
        let payload = serde_json::from_str::<serde_json::Value>(&json)
            .map_err(|e| format!("invalid json: {e}"))?;
        self.pipe_commands
            .push(crate::app::view_trait::ViewCommand::DeliverPipeMessage {
                sender_pane_id: 0,
                generation,
                payload,
                emitter: None,
            });
        Ok(())
    }

    fn close(&mut self, handle: u32) -> Result<(), String> {
        let binding = self
            .pipe_handles
            .remove(&handle)
            .ok_or_else(|| format!("unknown pipe handle {handle}"))?;
        let Some(generation) = binding.generation else {
            self.pipes.close(&binding.id);
            return Ok(());
        };
        if !self
            .pipe_handles
            .values()
            .any(|other| !other.binary && other.id == binding.id)
        {
            self.pipes.close_json(&binding.id);
        }
        log::info!(
            "plugin::{}: closed directed pipe '{id}' connection {generation} (handle {handle})",
            self.plugin_id,
            id = guest_text::shown(&binding.id)
        );
        self.pipe_commands
            .push(crate::app::view_trait::ViewCommand::CloseDirectedPipe {
                sender_pane_id: 0,
                generation,
                emitter: None,
            });
        Ok(())
    }

    fn is_connected(&mut self, handle: u32) -> bool {
        match self.pipe_handles.get(&handle) {
            Some(b) if b.binary => !self.pipes.drain_failed(&b.id),
            Some(b) => self.pipes.has_reader(&b.id),
            None => false,
        }
    }
}

// ─── Audio streams (G12) ────────────────────────────────────────────────────
//
// Backs the `audio-rt-control` import. Opening a stream registers its config;
// the host's RT thread (live path) and the synchronous G12 gate both pull
// samples by calling the guest's exported `process-output` via
// `WasmView::audio_process_output` — not through this registry.

#[derive(Default)]
struct AudioStreams {
    streams: HashMap<u32, audio_rt_control::AudioConfig>,
    next: u32,
}

impl audio_rt_control::Host for HostCtx {
    fn open_output(&mut self, config: audio_rt_control::AudioConfig) -> Result<u32, String> {
        let handle = self.audio_streams.next;
        self.audio_streams.next += 1;
        log::info!(
            "plugin::{}: audio open_output stream {handle} ({} Hz, {} ch, {} frames)",
            self.plugin_id,
            config.sample_rate,
            config.channels,
            config.buffer_frames
        );
        self.audio_streams.streams.insert(handle, config);
        Ok(handle)
    }

    fn open_input(&mut self, config: audio_rt_control::AudioConfig) -> Result<u32, String> {
        let handle = self.audio_streams.next;
        self.audio_streams.next += 1;
        log::info!(
            "plugin::{}: audio open_input stream {handle}",
            self.plugin_id
        );
        self.audio_streams.streams.insert(handle, config);
        Ok(handle)
    }

    fn stream_config(&mut self, handle: u32) -> Result<audio_rt_control::AudioConfig, String> {
        self.audio_streams
            .streams
            .get(&handle)
            .cloned()
            .ok_or_else(|| format!("unknown audio stream {handle}"))
    }

    fn pause(&mut self, handle: u32) -> Result<(), String> {
        if self.audio_streams.streams.contains_key(&handle) {
            Ok(())
        } else {
            Err(format!("unknown audio stream {handle}"))
        }
    }

    fn resume(&mut self, handle: u32) -> Result<(), String> {
        if self.audio_streams.streams.contains_key(&handle) {
            Ok(())
        } else {
            Err(format!("unknown audio stream {handle}"))
        }
    }

    fn close(&mut self, handle: u32) -> Result<(), String> {
        self.audio_streams.streams.remove(&handle);
        log::info!("plugin::{}: audio stream {handle} closed", self.plugin_id);
        Ok(())
    }
}

// ─── gpu import (G7/G11) ─────────────────────────────────────────────────────
//
// Delegates every WIT call to the per-plugin `GpuDevice`. The device is present
// only when the gpu capability was granted (link-time), so a missing device is
// an internal invariant violation rather than a plugin error.

impl HostCtx {
    fn gpu_mut(&mut self) -> Result<&mut crate::host::wasm_gpu::GpuDevice, String> {
        self.gpu
            .as_mut()
            .ok_or_else(|| "gpu capability not initialized".to_string())
    }
}

impl gpu::Host for HostCtx {
    fn create_surface_view(&mut self, handle: u64) -> Result<u64, String> {
        self.gpu_mut()?.create_surface_view(handle)
    }
    fn create_buffer(
        &mut self,
        label: String,
        size: u64,
        usage: gpu::BufferUsage,
    ) -> Result<u64, String> {
        Ok(self.gpu_mut()?.create_buffer(&label, size, usage))
    }
    fn write_buffer(&mut self, handle: u64, offset: u64, data: Vec<u8>) -> Result<(), String> {
        self.gpu_mut()?.write_buffer(handle, offset, &data)
    }
    fn read_buffer(&mut self, handle: u64, offset: u64, size: u64) -> Result<Vec<u8>, String> {
        self.gpu_mut()?.read_buffer(handle, offset, size)
    }
    fn destroy_buffer(&mut self, handle: u64) {
        if let Ok(g) = self.gpu_mut() {
            g.destroy_buffer(handle);
        }
    }
    fn create_texture(&mut self, label: String, desc: gpu::TextureDesc) -> Result<u64, String> {
        self.gpu_mut()?.create_texture(&label, desc)
    }
    fn destroy_texture(&mut self, handle: u64) {
        if let Ok(g) = self.gpu_mut() {
            g.destroy_texture(handle);
        }
    }
    fn create_render_pipeline(
        &mut self,
        label: String,
        wgsl: String,
        desc: gpu::RenderPipelineDesc,
    ) -> Result<u64, String> {
        self.gpu_mut()?.create_render_pipeline(&label, &wgsl, desc)
    }
    fn create_compute_pipeline(
        &mut self,
        label: String,
        wgsl: String,
        entry: String,
    ) -> Result<u64, String> {
        self.gpu_mut()?
            .create_compute_pipeline(&label, &wgsl, &entry)
    }
    fn destroy_pipeline(&mut self, handle: u64) {
        if let Ok(g) = self.gpu_mut() {
            g.destroy_pipeline(handle);
        }
    }
    fn create_bind_group(
        &mut self,
        pipeline: u64,
        bindings: Vec<gpu::BindingEntry>,
    ) -> Result<u64, String> {
        self.gpu_mut()?.create_bind_group(pipeline, &bindings)
    }
    fn destroy_bind_group(&mut self, handle: u64) {
        if let Ok(g) = self.gpu_mut() {
            g.destroy_bind_group(handle);
        }
    }
    fn submit_render_pass(&mut self, pass: gpu::RenderPassDesc) -> Result<(), String> {
        self.gpu_mut()?.submit_render_pass(pass)
    }
    fn submit_compute_pass(&mut self, pass: gpu::ComputePassDesc) -> Result<(), String> {
        self.gpu_mut()?.submit_compute_pass(pass)
    }
    fn copy_texture(&mut self, src: u64, dst: u64) -> Result<(), String> {
        self.gpu_mut()?.copy_texture(src, dst)
    }
}

// ─── WasmView ───────────────────────────────────────────────────────────────

pub struct WasmView {
    store: Store<HostCtx>,
    lifecycle: LifecycleGuest,
    // Some only when the component exports `audio-rt-process` (audio/full worlds).
    audio: Option<AudioProcessGuest>,
}

/// Derive the capability grants a component needs from the interfaces it
/// imports. Callers use this for pre-link review; instantiation still goes
/// through `load_with_grants` once the required imports have been approved.
fn grants_from_component(engine: &Engine, component: &Component) -> Grants {
    let mut grants = Grants::default();
    for (name, _) in component.component_type().imports(engine) {
        if name.contains("host-state") {
            grants.state = true;
        } else if name.contains("/pipes") {
            grants.pipes = true;
        } else if name.contains("audio-rt-control") {
            grants.audio = true;
        } else if name.contains("/gpu") {
            grants.gpu = true;
        }
    }
    grants
}

fn host_wit_package() -> &'static str {
    include_str!("../../wit/plexi.wit")
        .lines()
        .find_map(|line| line.strip_prefix("package "))
        .map(|package| package.trim_end_matches(';'))
        .unwrap_or("plexi:platform (version unavailable)")
}

/// The `plexi:platform` package a component was built against, when it is not
/// the host's. A WIT version bump renames every interface, so a component from
/// an older SDK would otherwise fail to link as if it lacked a grant.
fn foreign_sdk_package(engine: &Engine, component: &Component) -> Option<String> {
    let host = host_wit_package();
    let ty = component.component_type();
    let names: Vec<String> = ty
        .imports(engine)
        .map(|(name, _)| name.to_string())
        .chain(ty.exports(engine).map(|(name, _)| name.to_string()))
        .collect();
    names.into_iter().find_map(|name| {
        let rest = name.strip_prefix("plexi:platform/")?;
        let (_, version) = rest.split_once('@')?;
        let package = format!("plexi:platform@{version}");
        (package != host).then_some(package)
    })
}

fn component_load_error(plugin_id: &str, error: wasmtime::Error) -> wasmtime::Error {
    // Wasmtime does not expose a typed component signature-mismatch error.
    // Inspect its chain only at load boundaries, never errors from guest calls.
    if error
        .chain()
        .any(|cause| cause.to_string().contains("type mismatch"))
    {
        let package = host_wit_package();
        log::info!("plugin::{plugin_id}: incompatible SDK (host WIT {package}): {error:#}");
        error.context(format!(
            "plugin '{plugin_id}' was built against a different SDK; rebuild against the current SDK (host WIT package {package})"
        ))
    } else {
        error
    }
}

impl WasmView {
    fn engine_and_component(path: &Path) -> wasmtime::Result<(Engine, Component)> {
        let engine = super::wasm_engine::create_engine()?;
        let component = Component::from_file(&engine, path)?;
        Ok((engine, component))
    }

    /// Inspect a component's imported host interfaces without instantiating it.
    /// Raw `.wasm` launches use this to review link-time grants before loading.
    pub fn inspect_required_grants(path: &Path) -> wasmtime::Result<Grants> {
        let (engine, component) = Self::engine_and_component(path)?;
        Ok(grants_from_component(&engine, &component))
    }

    /// Load for tests and fixtures that intentionally bypass the launch review
    /// path. Production raw `.wasm` launches should inspect and review imports,
    /// then call `load_with_grants`.
    #[cfg(test)]
    pub fn load_ephemeral_run(
        plugin_id: impl Into<String>,
        path: &Path,
        state: StateStore,
    ) -> wasmtime::Result<Self> {
        let plugin_id = plugin_id.into();
        let (engine, component) = Self::engine_and_component(path)?;
        let grants = grants_from_component(&engine, &component);
        log::info!("plugin::{plugin_id}: ephemeral run grants {grants:?}");
        Self::instantiate(plugin_id, &engine, &component, &grants, state)
    }

    /// Load an installed/manifest-backed component with explicit host-interface
    /// grants derived from the manifest and remembered user decisions.
    ///
    /// Only an [`AdmittedPlugin`](crate::host::plugin_admission::AdmittedPlugin)
    /// starts one, and the plugin id and component come from it alone (stint
    /// 1398). The permit is spent at instantiation.
    pub(crate) fn load_with_grants(
        admitted: crate::host::plugin_admission::AdmittedPlugin<'_>,
        state: StateStore,
        grants: Grants,
    ) -> wasmtime::Result<Self> {
        let plugin_id = admitted.owner().to_string();
        if admitted.kind() != crate::app::registry::ManifestType::Wasm {
            return Err(wasmtime::Error::msg(format!(
                "plugin '{plugin_id}' was admitted as a Python plugin, not a WASM component"
            )));
        }
        let (engine, component) = Self::engine_and_component(admitted.entry())?;
        log::info!("plugin::{plugin_id}: manifest wasm grants {grants:?}");
        let start = admitted.commit();
        let view = Self::instantiate(plugin_id, &engine, &component, &grants, state);
        drop(start);
        view
    }

    /// [`Self::load_with_grants`] straight from a fixture component, for tests
    /// of the runtime. Compiled out of non-test builds.
    #[cfg(test)]
    pub(crate) fn load_with_grants_for_test(
        plugin_id: impl Into<String>,
        path: &Path,
        state: StateStore,
        grants: Grants,
    ) -> wasmtime::Result<Self> {
        let plugin_id = plugin_id.into();
        let root = path.parent().unwrap_or(path);
        let admitted = crate::host::plugin_admission::AdmittedPlugin::for_test(
            &plugin_id,
            crate::app::registry::ManifestType::Wasm,
            root,
            path,
        );
        Self::load_with_grants(admitted, state, grants)
    }

    fn instantiate(
        plugin_id: String,
        engine: &Engine,
        component: &Component,
        grants: &Grants,
        state: StateStore,
    ) -> wasmtime::Result<Self> {
        if let Some(found) = foreign_sdk_package(engine, component) {
            let package = host_wit_package();
            log::info!("plugin::{plugin_id}: incompatible SDK {found} (host WIT {package})");
            return Err(wasmtime::Error::msg(format!(
                "plugin '{plugin_id}' was built against a different SDK ({found}); rebuild against the current SDK (host WIT package {package})"
            )));
        }
        let mut linker = Linker::<HostCtx>::new(engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
        host_log::add_to_linker::<_, HasSelf<HostCtx>>(&mut linker, |c| c)?;
        if grants.state {
            host_state::add_to_linker::<_, HasSelf<HostCtx>>(&mut linker, |c| c)?;
        }
        if grants.pipes {
            pipes::add_to_linker::<_, HasSelf<HostCtx>>(&mut linker, |c| c)?;
        }
        let gpu_device = if grants.gpu {
            gpu::add_to_linker::<_, HasSelf<HostCtx>>(&mut linker, |c| c)?;
            // Prefer eframe's shared device so the guest's surface lives where
            // egui renders (zero-copy compositing). Headless/test processes
            // have no shared state and fall back to a dedicated device; a
            // missing adapter must fail the load, not surface on first draw.
            Some(match crate::host::wasm_gpu::host_render_state() {
                Some(rs) => crate::host::wasm_gpu::GpuDevice::from_shared(
                    rs.device.clone(),
                    rs.queue.clone(),
                ),
                None => crate::host::wasm_gpu::GpuDevice::new()
                    .map_err(|e| wasmtime::Error::msg(format!("plugin::{plugin_id}: {e}")))?,
            })
        } else {
            None
        };
        if grants.audio {
            audio_rt_control::add_to_linker::<_, HasSelf<HostCtx>>(&mut linker, |c| c)?;
        }

        let ctx = HostCtx {
            plugin_id: plugin_id.clone(),
            state,
            pipes: crate::host::typed_pipes::TypedPipeRegistry::new(
                crate::config::config_dir().join("pipes"),
            ),
            pipe_handles: HashMap::new(),
            next_pipe_handle: 0,
            pipe_requests: HashMap::new(),
            next_pipe_request: 0,
            pipe_commands: Vec::new(),
            audio_streams: AudioStreams::default(),
            gpu: gpu_device,
            wasi: WasiCtxBuilder::new().build(),
            table: ResourceTable::new(),
        };

        // Instantiate raw, then build per-interface export accessors. The world
        // struct's instantiate would demand every export of `plexi-full-plugin`;
        // standard plugins only export `lifecycle`, so build that unconditionally
        // and probe for the optional `audio-rt-process` export.
        let load_error = |error| component_load_error(&plugin_id, error);
        let pre = linker.instantiate_pre(component).map_err(load_error)?;
        let lifecycle_idx = LifecycleIndices::new(&pre).map_err(load_error)?;
        let audio_idx = AudioProcessIndices::new(&pre).ok();

        let mut store = Store::new(engine, ctx);
        let instance = pre.instantiate(&mut store).map_err(load_error)?;
        let lifecycle = lifecycle_idx
            .load(&mut store, &instance)
            .map_err(load_error)?;
        let audio = match audio_idx {
            Some(idx) => Some(idx.load(&mut store, &instance).map_err(load_error)?),
            None => None,
        };

        log::info!(
            "plugin::{}: wasm component instantiated (audio={})",
            plugin_id,
            audio.is_some()
        );
        Ok(WasmView {
            store,
            lifecycle,
            audio,
        })
    }

    pub fn init(
        &mut self,
        snapshot: &StateSnapshot,
        size: (f32, f32),
        args: &[String],
    ) -> wasmtime::Result<Vec<Effect>> {
        self.lifecycle
            .call_init(&mut self.store, snapshot, size, args)
    }

    pub fn update(&mut self, event: &InputEvent) -> wasmtime::Result<Vec<Effect>> {
        self.lifecycle.call_update(&mut self.store, event)
    }

    pub fn plugin_id(&self) -> &str {
        &self.store.data().plugin_id
    }

    pub fn view(&mut self) -> wasmtime::Result<UiTree> {
        self.lifecycle.call_view(&mut self.store)
    }

    /// Pull one buffer of output samples from the guest's RT `process-output`
    /// export. Returns interleaved f32 samples (len = buffer_frames * channels)
    /// and the threaded `u64` state slot. Errors on a non-audio component.
    pub fn audio_process_output(
        &mut self,
        handle: u32,
        buffer_frames: u32,
        channels: u32,
        sample_rate: u32,
        state: u64,
    ) -> wasmtime::Result<(Vec<f32>, u64)> {
        let audio = self
            .audio
            .as_ref()
            .ok_or_else(|| wasmtime::Error::msg("audio_process_output on a non-audio plugin"))?;
        audio.call_process_output(
            &mut self.store,
            handle,
            buffer_frames,
            channels,
            sample_rate,
            state,
        )
    }

    /// Config of the first audio output stream the guest opened via
    /// `audio-rt-control`, as `(stream_handle, sample_rate, channels,
    /// buffer_frames)`. `None` until the guest opens one (or for non-audio
    /// plugins). The live pane uses this to start the host output stream.
    pub fn output_stream_config(&self) -> Option<(u32, u32, u32, u32)> {
        let streams = &self.store.data().audio_streams;
        streams
            .streams
            .iter()
            .min_by_key(|(handle, _)| **handle)
            .map(|(handle, cfg)| (*handle, cfg.sample_rate, cfg.channels, cfg.buffer_frames))
    }

    /// Socket path of a binary pipe the guest opened, for a peer to connect to.
    #[cfg(test)]
    pub fn pipe_socket_path(&self, pipe_id: &str) -> Option<String> {
        self.store.data().pipes.binary_socket_path(pipe_id)
    }

    /// Stamp the owning pane's host-established scope onto this instance's
    /// typed-pipe registry (stint 0724 Phase D 2/2). Called once from
    /// `WasmPane::set_context_id`, which runs at pane placement — after
    /// `pane_id` is already known — so every pipe this instance ever opens is
    /// attributable to its owning pane for Phase F's audit.
    pub fn set_pipe_owner(&mut self, owner: crate::host::scope::Scope) {
        self.store.data_mut().pipes.set_owner(owner);
    }

    /// Phase F's audit surface: the scope this instance's typed-pipe registry
    /// is owned at. `None` before pane placement stamps it.
    #[cfg(test)]
    pub fn pipe_owner(&self) -> Option<&crate::host::scope::Scope> {
        self.store.data().pipes.owner()
    }

    /// Register directed pipe connection `generation` on this WASM instance
    /// under a fresh guest handle.
    pub fn register_directed_pipe(&mut self, pipe_id: &str, generation: u64) -> Result<(), String> {
        self.store
            .data_mut()
            .register_directed_pipe(pipe_id, generation)
    }

    /// True when `pipe_id` is open on this WASM instance's typed-pipe registry.
    #[cfg(test)]
    pub fn has_pipe_reader(&self, pipe_id: &str) -> bool {
        self.store.data().has_pipe_reader(pipe_id)
    }

    /// Close a host-retired directed pipe connection on this WASM instance.
    pub fn close_directed_pipe(&mut self, pipe_id: &str, generation: u64) {
        self.store
            .data_mut()
            .close_directed_pipe(pipe_id, generation);
    }

    /// The guest handle carrying directed pipe connection `generation`.
    pub fn directed_pipe_handle(&self, generation: u64) -> Option<u32> {
        self.store.data().directed_pipe_handle(generation)
    }

    /// See [`HostCtx::take_pipe_request`].
    pub fn take_pipe_request(&mut self, request_id: &str) -> Option<u32> {
        self.store.data_mut().take_pipe_request(request_id)
    }

    /// See [`HostCtx::take_pipe_commands`].
    pub fn take_pipe_commands(&mut self) -> Vec<crate::app::view_trait::ViewCommand> {
        self.store.data_mut().take_pipe_commands()
    }

    /// Open a binary pipe as the guest would, for tests. The socket lives in
    /// a short `/tmp` directory: a temp config dir overflows `SUN_LEN`.
    #[cfg(test)]
    pub fn open_binary_pipe_for_test(&mut self, id: &str) -> Result<u32, String> {
        let dir = PathBuf::from("/tmp").join(format!("plwv{}", std::process::id()));
        self.store.data_mut().pipes = crate::host::typed_pipes::TypedPipeRegistry::new(dir);
        pipes::Host::open(
            self.store.data_mut(),
            id.to_string(),
            pipes::PipeType::Binary,
            pipes::PipeDirection::Out,
        )
    }

    /// Send a binary frame as the guest would, for tests.
    #[cfg(test)]
    pub fn send_binary_for_test(&mut self, handle: u32, frame: Vec<u8>) -> Result<(), String> {
        pipes::Host::send_binary(self.store.data_mut(), handle, frame)
    }

    /// Open a plain JSON pipe as the guest would, for tests.
    #[cfg(test)]
    pub fn open_json_pipe_for_test(&mut self, id: &str) -> Result<u32, String> {
        pipes::Host::open(
            self.store.data_mut(),
            id.to_string(),
            pipes::PipeType::Json,
            pipes::PipeDirection::Out,
        )
    }

    /// Call `open-directed` as the guest would, for tests.
    #[cfg(test)]
    pub fn open_directed_for_test(&mut self, id: &str, target_pane: u64) -> Result<u32, String> {
        pipes::Host::open_directed(self.store.data_mut(), id.to_string(), target_pane)
    }

    /// Close a pipe handle as the guest would, for tests.
    #[cfg(test)]
    pub fn close_pipe_for_test(&mut self, handle: u32) -> Result<(), String> {
        pipes::Host::close(self.store.data_mut(), handle)
    }

    /// How many handles carry a directed connection, for tests.
    #[cfg(test)]
    pub fn directed_handle_count_for_test(&self) -> usize {
        self.store
            .data()
            .pipe_handles
            .values()
            .filter(|binding| binding.generation.is_some())
            .count()
    }

    /// True when the gpu capability was granted (the plugin declares a surface).
    pub fn has_gpu(&self) -> bool {
        self.store.data().gpu.is_some()
    }

    /// Allocate a surface texture for a `surface-node`. Returns the opaque
    /// handle to deliver to the guest in a `surface-ready` event, or `None` if
    /// the plugin has no gpu capability.
    pub fn alloc_surface(&mut self, width: u32, height: u32) -> Option<u64> {
        self.store
            .data_mut()
            .gpu
            .as_mut()
            .map(|g| g.alloc_surface(width, height))
    }

    /// Drop a surface texture (display-scale reallocation, stint 0527).
    /// Unknown handles and missing gpu capability are no-ops.
    pub fn free_surface(&mut self, handle: u64) {
        if let Some(g) = self.store.data_mut().gpu.as_mut() {
            g.free_surface(handle);
        }
    }

    /// Read a surface texture back to an RGBA image for pixel assertions.
    /// Test-only: live composition never blocks the UI thread on a synchronous
    /// readback — see `request_surface_readback`/`take_surface_readback`.
    #[cfg(test)]
    pub fn read_surface(&self, handle: u64) -> Option<image::RgbaImage> {
        self.store
            .data()
            .gpu
            .as_ref()
            .and_then(|g| g.read_texture(handle).ok())
    }

    /// Start a nonblocking copy of a surface for the dedicated-device fallback.
    /// Live panes on the host render device use `surface_srgb_view` instead.
    pub fn request_surface_readback(&mut self, handle: u64) -> Result<(), String> {
        self.store
            .data_mut()
            .gpu
            .as_mut()
            .ok_or_else(|| "gpu capability not granted".to_string())?
            .request_surface_readback(handle)
    }

    /// Take the newest completed dedicated-device surface readback.
    pub fn take_surface_readback(&mut self) -> Option<Result<image::RgbaImage, String>> {
        self.store
            .data_mut()
            .gpu
            .as_mut()
            .and_then(|g| g.take_surface_readback())
    }
    /// sRGB view of the surface texture for zero-copy egui compositing.
    pub fn surface_srgb_view(&self, handle: u64) -> Option<wgpu::TextureView> {
        self.store
            .data()
            .gpu
            .as_ref()
            .and_then(|g| g.surface_srgb_view(handle))
    }
    /// Surface readbacks performed by this plugin's device (capture path only).
    pub fn surface_readbacks(&self) -> u64 {
        self.store
            .data()
            .gpu
            .as_ref()
            .map_or(0, |g| g.readback_count())
    }
}

// ─── Gate tests (G3 effect roundtrip, G5 state persistence) ────────────────
//
// These load the committed sysmon component fixture and drive the real
// wasmtime runtime — no subprocess, no rendering. Regenerate the fixture with
// `just wasm-fixtures` after changing the sysmon POC.
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn wit_snapshot_path(root: &Path, path: &Path) -> String {
        path.strip_prefix(root)
            .unwrap()
            .components()
            .map(|component| component.as_os_str().to_str().unwrap())
            .collect::<Vec<_>>()
            .join("/")
    }

    #[test]
    fn wit_snapshot_paths_use_forward_slashes() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let path = root.join("wit").join("deps").join("nested.wit");
        assert_eq!(wit_snapshot_path(&root, &path), "wit/deps/nested.wit");
    }

    #[test]
    fn incompatible_sdk_message_is_version_neutral() {
        let error = component_load_error(
            "com.plexi.incompatible",
            wasmtime::Error::msg("type mismatch"),
        );
        let message = error.to_string();
        assert!(message.contains("a different SDK"), "{message}");
        assert!(
            message.contains("rebuild against the current SDK"),
            "{message}"
        );
    }

    #[test]
    fn wit_sources_match_version_snapshot() {
        use sha2::{Digest, Sha256};

        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let wit = std::fs::read_to_string(root.join("wit/plexi.wit")).unwrap();
        let package = wit
            .lines()
            .find_map(|line| line.strip_prefix("package "))
            .unwrap();
        let version = package.trim_end_matches(';').split_once('@').unwrap().1;
        let mut pending = vec![root.join("wit")];
        let mut sources = Vec::new();
        while let Some(path) = pending.pop() {
            for entry in std::fs::read_dir(path).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_dir() {
                    pending.push(entry.path());
                } else if entry.path().extension().is_some_and(|ext| ext == "wit") {
                    sources.push(entry.path());
                }
            }
        }
        sources.sort_by_key(|path| wit_snapshot_path(&root, path));
        let actual: String = sources
            .iter()
            .map(|path| {
                format!(
                    "{:x}  {}\n",
                    Sha256::digest(std::fs::read(path).unwrap()),
                    wit_snapshot_path(&root, path)
                )
            })
            .collect();
        // Existing version snapshots are immutable. A WIT edit requires a package
        // version bump and a new snapshot, never rewriting an existing snapshot.
        let snapshot = root
            .join("tests/wit-versions")
            .join(format!("{version}.sha256"));
        let expected = std::fs::read_to_string(&snapshot).unwrap_or_else(|error| {
            panic!("add WIT version snapshot {}: {error}", snapshot.display())
        });
        assert_eq!(
            actual, expected,
            "WIT changed without a package version bump; bump the WIT version and add a new version snapshot"
        );
    }

    #[test]
    fn older_sdk_component_reports_rebuild_and_plugin_id() {
        let dir = tempfile::tempdir().unwrap();
        let _profile = crate::config::set_test_profile_dir(dir.path().to_path_buf());
        let capture = crate::testing::test_log::capture();
        // Frozen sysmon from d6c1a15e1^ (blob b4317b91fd8e0e885977b6e27112e7254a235cdd),
        // before Row/Column gained `justify`, built against plexi:platform@0.3.0.
        // Do not rebuild this negative fixture.
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/wasm-fixtures/sysmon-pre-justify.wasm");
        let error = WasmView::load_with_grants_for_test(
            "com.plexi.old-sysmon",
            &path,
            StateStore::ephemeral(),
            Grants {
                state: true,
                ..Grants::default()
            },
        )
        .err()
        .expect("previous record shape must be rejected");
        let message = error.to_string();
        assert!(message.contains("a different SDK"), "{message}");
        assert!(
            message.contains("rebuild against the current SDK"),
            "{error:#}"
        );
        assert!(message.contains("com.plexi.old-sysmon"), "{message}");
        assert!(message.contains(host_wit_package()), "{message}");
        assert!(message.contains("plexi:platform@0.3.0"), "{message}");
        assert!(
            capture.lines().iter().any(|line| {
                line.contains("com.plexi.old-sysmon") && line.contains("incompatible SDK")
            }),
            "the rejected SDK version must be logged"
        );
    }

    #[test]
    fn missing_capability_is_not_reported_as_an_older_sdk() {
        let dir = tempfile::tempdir().unwrap();
        let _profile = crate::config::set_test_profile_dir(dir.path().to_path_buf());
        let error = WasmView::load_with_grants_for_test(
            "com.plexi.ungranted-sysmon",
            &fixture(),
            StateStore::ephemeral(),
            Grants::default(),
        )
        .err()
        .expect("state import needs a grant");
        let message = format!("{error:#}");
        assert!(message.contains("host-state"), "{message}");
        assert!(
            !message.contains("rebuild against the current SDK"),
            "{message}"
        );
    }

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wasm-fixtures/sysmon.wasm")
    }

    fn audio_fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wasm-fixtures/audio-synth.wasm")
    }

    fn pong_fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wasm-fixtures/pong.wasm")
    }

    fn empty_snapshot() -> StateSnapshot {
        StateSnapshot { entries: vec![] }
    }

    /// Stint 1344: a WASM guest's `host-log` message is escaped and capped
    /// where it arrives, so it cannot forge a host line or recolor a terminal
    /// reading `plexi.log`.
    #[test]
    fn guest_log_messages_are_escaped_and_capped() {
        let capture = crate::testing::test_log::capture();
        let mut app =
            WasmView::load_ephemeral_run("guest-log-probe", &fixture(), StateStore::ephemeral())
                .expect("load");
        let hostile = format!(
            "LOGMARK\u{1b}]0;forged\u{7}\nplugin::other: forged\u{202e}{}",
            "A".repeat(5000)
        );
        host_log::Host::info(app.store.data_mut(), hostile);
        let line = capture
            .lines()
            .into_iter()
            .find(|line| line.contains("LOGMARK"))
            .expect("guest log line");
        assert!(
            !line
                .chars()
                .any(crate::host::guest_text::is_unsafe_to_print),
            "{:?}",
            line.chars().take(80).collect::<String>()
        );
        assert!(line.contains("\\u{1b}]0;forged\\u{7}\\nplugin::other"));
        assert!(line.ends_with(crate::host::guest_text::TRUNCATED));
        assert!(capture
            .lines()
            .iter()
            .any(|line| line.contains("guest-log-probe: guest log message truncated from")));
    }

    #[test]
    fn inspect_required_grants_maps_component_imports() -> wasmtime::Result<()> {
        let grants = WasmView::inspect_required_grants(&audio_fixture())?;

        assert_eq!(
            grants.capability_ids(),
            vec![
                "state:read-write".to_string(),
                "pipe.open".to_string(),
                "audio.playback".to_string()
            ]
        );
        Ok(())
    }

    // Stint 0386: pong is compiled against the `plexi-gpu-plugin` world, which
    // unconditionally imports `pipes` and `gpu`. Ground truth is the compiled
    // component's actual imports (dead-code elimination strips anything the
    // guest never calls, e.g. pong never touches host-state), not the WIT
    // world text.
    #[test]
    fn inspect_required_grants_maps_component_imports_for_pong() -> wasmtime::Result<()> {
        let grants = WasmView::inspect_required_grants(&pong_fixture())?;

        assert_eq!(
            grants.capability_ids(),
            vec!["pipe.open".to_string(), "gpu.render".to_string()]
        );
        Ok(())
    }

    // Stint 0386: the manifest-backed launch path (`open_installed_wasm_view_pane`)
    // derives `Grants` strictly from `plugins/wasm-poc/pong/manifest.toml`'s
    // declared `capabilities` list, then links the component against those
    // grants. Before the fix, `capabilities = []` meant `pipes`/`gpu` were
    // never linked, and wasmtime failed at instantiation with an unresolved
    // import. Parse the real manifest and reproduce that exact derivation to
    // prove the declared capabilities now satisfy the compiled binary's
    // actual required imports end to end.
    #[test]
    fn pong_manifest_capabilities_satisfy_compiled_component_imports() -> wasmtime::Result<()> {
        use crate::app::permissions::{parse_capability_strings, Capability};
        use crate::app::registry::PluginManifest;

        let manifest_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("plugins/wasm-poc/pong/manifest.toml");
        let manifest_text = std::fs::read_to_string(&manifest_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", manifest_path.display()));
        let manifest: PluginManifest =
            toml::from_str(&manifest_text).expect("pong manifest.toml parses");

        let declared_caps = parse_capability_strings(&manifest.plugin.capabilities.capabilities)
            .expect("pong manifest declares only known capabilities");

        let grants = Grants {
            state: true,
            pipes: declared_caps.contains(&Capability::PipeOpen),
            gpu: declared_caps.contains(&Capability::GpuRender),
            audio: declared_caps.contains(&Capability::AudioPlayback),
        };
        assert!(
            grants.pipes && grants.gpu,
            "pong manifest must declare pipe.open and gpu.render"
        );

        WasmView::load_with_grants_for_test(
            "com.plexi.pong",
            &pong_fixture(),
            StateStore::ephemeral(),
            grants,
        )
        .map(|_| ())
    }

    fn host_ctx_with_pipes() -> HostCtx {
        // Unix socket paths must be < ~104 bytes (SUN_LEN); the macOS
        // /var/folders temp dir + a UUID socket name overflows it, so use a
        // short /tmp path for the bind to succeed.
        let dir = PathBuf::from("/tmp").join(format!("plpipe{}", std::process::id()));
        HostCtx {
            plugin_id: "pipes-test".to_string(),
            state: StateStore::ephemeral(),
            pipes: crate::host::typed_pipes::TypedPipeRegistry::new(dir),
            pipe_handles: std::collections::HashMap::new(),
            next_pipe_handle: 0,
            pipe_requests: std::collections::HashMap::new(),
            next_pipe_request: 0,
            pipe_commands: Vec::new(),
            audio_streams: AudioStreams::default(),
            gpu: None,
            wasi: WasiCtxBuilder::new().build(),
            table: ResourceTable::new(),
        }
    }

    // G13 (host side): the pipes import is backed by TypedPipeRegistry. A binary
    // pipe opens, accepts frames into the lock-free ring, rejects json-on-binary,
    // and reports overrun (full ring) as an error instead of panicking/blocking.
    #[test]
    fn g13_pipes_open_send_overrun_close() {
        use pipes::{Host, PipeDirection, PipeType};
        let mut ctx = host_ctx_with_pipes();

        let h = ctx
            .open(
                "waveform-out".to_string(),
                PipeType::Binary,
                PipeDirection::Out,
            )
            .expect("open binary");
        assert!(ctx.is_connected(h), "freshly opened pipe is healthy");
        ctx.send_binary(h, vec![1, 2, 3, 4])
            .expect("first frame fits");

        // No peer is draining the ring, so it fills at capacity; further pushes
        // must return an overrun error — never panic, never block.
        let mut overran = false;
        for _ in 0..1024 {
            if ctx.send_binary(h, vec![0u8; 64]).is_err() {
                overran = true;
                break;
            }
        }
        assert!(overran, "a full ring must surface overrun as Err");

        // json send on a binary pipe is rejected.
        assert!(ctx.send_json(h, "{}".to_string()).is_err());
        ctx.close(h).expect("close");
        assert!(!ctx.is_connected(h), "closed handle is not connected");
        assert!(
            ctx.send_binary(h, vec![0]).is_err(),
            "send after close errors"
        );
    }

    // A json pipe round-trips through validation: valid json is accepted,
    // malformed json and binary-on-json are rejected.
    #[test]
    fn g13_json_pipe_validation() {
        use pipes::{Host, PipeDirection, PipeType};
        let mut ctx = host_ctx_with_pipes();
        let h = ctx
            .open("score".to_string(), PipeType::Json, PipeDirection::Out)
            .expect("open json");
        let error = ctx
            .send_json(h, r#"{"score":3}"#.to_string())
            .expect_err("a pipe the guest opened by name carries no connection");
        assert!(error.contains("not a directed connection"), "{error}");
        assert!(ctx.take_pipe_commands().is_empty());
        assert!(
            ctx.send_binary(h, vec![0]).is_err(),
            "binary on json pipe errors"
        );
        ctx.close(h).expect("close");
    }

    /// Stint 1681: a directed handle carries one connection for its life.
    /// The guest cannot claim it by name; a send names its connection when
    /// the import is called; an open is a numbered request; once the host
    /// retires the connection the handle is unknown, and a reopen of the
    /// name is a new handle whose close ends only its own connection.
    #[test]
    fn a_directed_handle_carries_one_connection_for_its_life() {
        use crate::app::view_trait::ViewCommand;
        use pipes::{Host, PipeDirection, PipeType};
        let mut ctx = host_ctx_with_pipes();
        assert_eq!(ctx.register_directed_pipe("directed.pipe", 4), Ok(()));
        assert!(ctx.has_pipe_reader("directed.pipe"));
        let old = ctx
            .directed_pipe_handle(4)
            .expect("host registered a handle for connection 4");
        assert!(
            ctx.open(
                "directed.pipe".to_string(),
                PipeType::Json,
                PipeDirection::Duplex
            )
            .is_err(),
            "a guest open by name never claims a connection"
        );

        ctx.send_json(old, r#"{"n":1}"#.to_string()).expect("send");
        assert!(ctx.send_json(old, "{not json".to_string()).is_err());
        let sent = ctx.take_pipe_commands();
        assert!(
            matches!(
                &sent[..],
                [ViewCommand::DeliverPipeMessage { generation: 4, payload, .. }]
                    if payload == &serde_json::json!({"n": 1})
            ),
            "{:?}",
            sent.iter().map(ViewCommand::verb).collect::<Vec<_>>()
        );

        assert_eq!(ctx.open_directed("other".to_string(), 9), Ok(0));
        assert_eq!(ctx.open_directed("other".to_string(), 9), Ok(1));
        assert!(ctx.open_directed(String::new(), 9).is_err());
        let opens = ctx.take_pipe_commands();
        assert!(matches!(
            &opens[..],
            [
                ViewCommand::OpenDirectedPipe { request_id: first, target_pane_id: 9, .. },
                ViewCommand::OpenDirectedPipe { request_id: second, .. },
            ] if first == "wasm-open-0" && second == "wasm-open-1"
        ));
        assert_eq!(ctx.take_pipe_request("wasm-open-1"), Some(1));
        assert_eq!(ctx.take_pipe_request("wasm-open-1"), None, "answered once");

        ctx.close_directed_pipe("directed.pipe", 4);
        let error = ctx
            .send_json(old, r#"{"late":true}"#.to_string())
            .expect_err("a retired connection's handle is unknown");
        assert!(error.contains("unknown pipe handle"), "{error}");
        assert_eq!(ctx.register_directed_pipe("directed.pipe", 5), Ok(()));
        let new = ctx
            .directed_pipe_handle(5)
            .expect("handle for connection 5");
        assert_ne!(new, old);
        assert!(ctx.send_json(old, "{}".to_string()).is_err());

        ctx.close(new).expect("guest closes its connection");
        assert!(!ctx.has_pipe_reader("directed.pipe"));
        assert!(matches!(
            &ctx.take_pipe_commands()[..],
            [ViewCommand::CloseDirectedPipe { generation: 5, .. }]
        ));
    }

    /// Stint 1681: a directed pipe never takes a binary pipe's id, and
    /// retiring a connection drops only its own handle: a binary pipe of the
    /// same name keeps taking frames after the refusal and after a JSON
    /// connection on another name is retired.
    #[test]
    fn directed_pipe_registration_and_retirement_leave_a_binary_pipe_alone() {
        use pipes::{Host, PipeDirection, PipeType};
        let mut ctx = host_ctx_with_pipes();
        let binary = ctx
            .open("frames".to_string(), PipeType::Binary, PipeDirection::Out)
            .expect("open binary");
        assert!(
            ctx.register_directed_pipe("frames", 1).is_err(),
            "a directed pipe may not take a binary pipe's id"
        );
        assert_eq!(ctx.directed_pipe_handle(1), None);
        ctx.close_directed_pipe("frames", 1);
        assert!(ctx.pipes.is_binary("frames"), "the binary pipe stays open");
        ctx.send_binary(binary, vec![1, 2, 3])
            .expect("the binary ring still takes frames after the refusal");

        assert_eq!(ctx.register_directed_pipe("directed", 2), Ok(()));
        let directed = ctx
            .directed_pipe_handle(2)
            .expect("handle for connection 2");
        assert_ne!(directed, binary);
        ctx.close_directed_pipe("directed", 2);
        assert!(!ctx.has_pipe_reader("directed"));
        assert_eq!(ctx.directed_pipe_handle(2), None);
        ctx.send_binary(binary, vec![4])
            .expect("the binary ring still takes frames after the retirement");
        ctx.close(binary).expect("close binary");
    }

    // G12 (host side): a plexi-audio-plugin component links audio-rt-control and
    // exports audio-rt-process. After the guest starts playing, the host pulls
    // a buffer via process-output and gets non-silent interleaved samples. The
    // whole loop runs synchronously through wasmtime — no OS audio thread, which
    // is exactly the spec's automated pass condition (RT-thread priority and
    // <10ms latency are measured separately by the live path).
    #[test]
    fn g12_audio_process_output_produces_sound() -> wasmtime::Result<()> {
        // Grants are derived from the component's imports; audio-synth imports
        // audio-rt-control, so the audio capability is linked.
        let mut app = WasmView::load_ephemeral_run(
            "audio-synth-g12",
            &audio_fixture(),
            StateStore::ephemeral(),
        )?;
        assert!(
            app.audio.is_some(),
            "audio-synth must export audio-rt-process"
        );

        app.init(&empty_snapshot(), (400.0, 300.0), &[])?;

        // Before play, the envelope is at zero — output is silent.
        let (silent, _) = app.audio_process_output(0, 512, 2, 48_000, 0)?;
        assert_eq!(
            silent.len(),
            512 * 2,
            "interleaved buffer = frames * channels"
        );
        assert!(
            silent.iter().all(|&s| s.abs() <= 0.01),
            "stopped synth must be silent"
        );

        // Space toggles play; pump several buffers so the amplitude ramp opens.
        app.update(&key("space"))?;
        let mut state = 0u64;
        let mut peak = 0.0f32;
        for _ in 0..32 {
            let (samples, next) = app.audio_process_output(0, 512, 2, 48_000, state)?;
            state = next;
            peak = peak.max(samples.iter().fold(0.0, |m, &s| m.max(s.abs())));
        }
        assert!(
            peak > 0.01,
            "playing synth must produce audible samples (peak={peak})"
        );
        Ok(())
    }

    // G13 (guest round-trip): the real audio-synth component, driven through
    // wasmtime, opens its `waveform-out` binary pipe in init and pushes a
    // decimated waveform preview from `process-output` on every RT buffer. A
    // peer connects to the unix socket as a client and must receive the frames
    // intact (u32-BE length prefix + payload) — proving the full guest→ring→
    // drain-thread→socket path, not just the host-side primitives.
    #[test]
    fn g13_guest_roundtrip_waveform_pipe() -> wasmtime::Result<()> {
        use std::io::Read;
        use std::os::unix::net::UnixStream;

        let mut app = WasmView::load_ephemeral_run(
            "audio-synth-g13",
            &audio_fixture(),
            StateStore::ephemeral(),
        )?;
        // init opens the audio output stream + the waveform binary pipe (Out).
        app.init(&empty_snapshot(), (400.0, 300.0), &[])?;

        let sock = app
            .pipe_socket_path("waveform-out")
            .expect("guest opened the waveform-out binary pipe in init");

        // A visualiser peer connects as the socket client and reads framed bytes.
        let mut client = UnixStream::connect(&sock).expect("connect to waveform socket");
        client
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .expect("set read timeout");

        // Play, then pump RT buffers. Each process-output pushes one decimated
        // 256-byte preview frame (1024 samples / 16 * 4 bytes) into the ring.
        app.update(&key("space"))?;
        let mut state = 0u64;
        for _ in 0..16 {
            let (_, next) = app.audio_process_output(0, 512, 2, 48_000, state)?;
            state = next;
        }

        // Read several frames off the socket; each must be a complete 256-byte
        // length-prefixed frame, and the playing synth must carry real samples.
        let mut any_nonzero = false;
        for _ in 0..4 {
            let mut len_buf = [0u8; 4];
            client.read_exact(&mut len_buf).expect("read frame length");
            let len = u32::from_be_bytes(len_buf) as usize;
            assert_eq!(len, 256, "decimated preview = 1024 samples / 16 * 4 bytes");
            let mut payload = vec![0u8; len];
            client.read_exact(&mut payload).expect("read frame payload");
            any_nonzero |= payload
                .chunks_exact(4)
                .any(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]).abs() > 0.0);
        }
        assert!(
            any_nonzero,
            "playing synth waveform frames must carry non-zero samples"
        );
        Ok(())
    }

    // G11 (host side): the gpu import compiles a WGSL pipeline, binds a uniform,
    // and executes a render pass on the real wgpu device. We clear a surface to
    // black, draw a fullscreen triangle whose color comes from a uniform buffer,
    // read the surface back, and assert the pixels are that color — proving the
    // full create-pipeline / create-bind-group / submit-render-pass / readback
    // path against an actual GPU. No pixel buffer crosses the WASM boundary.
    const UNIFORM_WGSL: &str = r#"
struct U { color: vec4<f32> }
@group(0) @binding(0) var<uniform> u: U;
@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    var p = array<vec2<f32>, 3>(vec2(-1.0, -3.0), vec2(-1.0, 1.0), vec2(3.0, 1.0));
    return vec4(p[vi], 0.0, 1.0);
}
@fragment
fn fs_main() -> @location(0) vec4<f32> { return u.color; }
"#;

    fn gpu_ctx() -> HostCtx {
        let device = crate::host::wasm_gpu::GpuDevice::new().expect("wgpu device");
        let mut ctx = host_ctx_with_pipes();
        ctx.gpu = Some(device);
        ctx
    }

    #[test]
    #[ignore = "perf-gate: run explicitly on a quiet machine"]
    fn perf_gate_gpu_render_pass_executes_on_device() -> Result<(), String> {
        use gpu::Host;
        let mut ctx = gpu_ctx();

        let surface = ctx.gpu.as_mut().unwrap().alloc_surface(64, 64);
        let view = ctx.create_surface_view(surface)?;

        // Green uniform color (RGBA f32).
        let ubuf = ctx.create_buffer(
            "color".to_string(),
            16,
            gpu::BufferUsage::UNIFORM | gpu::BufferUsage::COPY_DST,
        )?;
        let green: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
        let green_bytes: Vec<u8> = green.iter().flat_map(|f| f.to_le_bytes()).collect();
        ctx.write_buffer(ubuf, 0, green_bytes)?;

        let pipeline = ctx.create_render_pipeline(
            "uniform".to_string(),
            UNIFORM_WGSL.to_string(),
            gpu::RenderPipelineDesc {
                vs_entry: "vs_main".to_string(),
                fs_entry: "fs_main".to_string(),
                vertex_stride: 0,
                attrs: vec![],
                output_format: gpu::TextureFormat::Rgba8Unorm,
                blend_alpha: false,
            },
        )?;
        let bind = ctx.create_bind_group(
            pipeline,
            vec![gpu::BindingEntry {
                binding: 0,
                resource_ref: gpu::BindingResource::Buffer(ubuf),
            }],
        )?;

        let make_pass = || gpu::RenderPassDesc {
            target: view,
            clear_color: Some((0.0, 0.0, 0.0, 1.0)),
            pipeline,
            vertex_buffer: None,
            index_buffer: None,
            bind_groups: vec![(0, bind)],
            draws: vec![gpu::DrawCall {
                vertices: 3,
                instances: 1,
                first_vertex: 0,
                first_instance: 0,
            }],
        };
        // G11 frame-time budget: a steady-state encode+submit must stay cheap
        // relative to a frame. The first submit pays one-time Metal cost
        // (command-buffer alloc, PSO/driver warmup) that is not representative,
        // so warm up once, then time the second. The 5ms ceiling is a
        // regression guard with headroom for machine load — a warmed submit of
        // this trivial pass runs well under 1ms; this only trips on an
        // order-of-magnitude regression (e.g. an accidental synchronous stall).
        ctx.submit_render_pass(make_pass())?;
        let start = std::time::Instant::now();
        ctx.submit_render_pass(make_pass())?;
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_secs_f32() * 1000.0 < 5.0,
            "warmed render pass submit should be < 5ms, was {elapsed:?}"
        );

        let img = ctx.gpu.as_ref().unwrap().read_texture(surface)?;
        let px = img.get_pixel(32, 32);
        assert!(
            px[0] < 40 && px[1] > 200 && px[2] < 40,
            "surface should be the uniform green, got {px:?}"
        );
        Ok(())
    }

    #[test]
    fn g11_gpu_render_pass_renders_uniform_color() -> Result<(), String> {
        use gpu::Host;
        let mut ctx = gpu_ctx();

        let surface = ctx.gpu.as_mut().unwrap().alloc_surface(64, 64);
        let view = ctx.create_surface_view(surface)?;
        let ubuf = ctx.create_buffer(
            "color".to_string(),
            16,
            gpu::BufferUsage::UNIFORM | gpu::BufferUsage::COPY_DST,
        )?;
        let green: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
        let green_bytes: Vec<u8> = green.iter().flat_map(|f| f.to_le_bytes()).collect();
        ctx.write_buffer(ubuf, 0, green_bytes)?;
        let pipeline = ctx.create_render_pipeline(
            "uniform".to_string(),
            UNIFORM_WGSL.to_string(),
            gpu::RenderPipelineDesc {
                vs_entry: "vs_main".to_string(),
                fs_entry: "fs_main".to_string(),
                vertex_stride: 0,
                attrs: vec![],
                output_format: gpu::TextureFormat::Rgba8Unorm,
                blend_alpha: false,
            },
        )?;
        let bind = ctx.create_bind_group(
            pipeline,
            vec![gpu::BindingEntry {
                binding: 0,
                resource_ref: gpu::BindingResource::Buffer(ubuf),
            }],
        )?;
        ctx.submit_render_pass(gpu::RenderPassDesc {
            target: view,
            clear_color: Some((0.0, 0.0, 0.0, 1.0)),
            pipeline,
            vertex_buffer: None,
            index_buffer: None,
            bind_groups: vec![(0, bind)],
            draws: vec![gpu::DrawCall {
                vertices: 3,
                instances: 1,
                first_vertex: 0,
                first_instance: 0,
            }],
        })?;
        let img = ctx.gpu.as_ref().unwrap().read_texture(surface)?;
        let px = img.get_pixel(32, 32);
        assert!(
            px[0] < 40 && px[1] > 200 && px[2] < 40,
            "surface should be the uniform green, got {px:?}"
        );
        Ok(())
    }

    fn mock_stats(cpu: f32) -> SystemStats {
        SystemStats {
            cpu_usage_pct: cpu,
            memory_used_bytes: 8u64 << 30,
            memory_total_bytes: 16u64 << 30,
            disk_read_bps: 0,
            disk_write_bps: 0,
            net_rx_bps: 0,
            net_tx_bps: 0,
            uptime_secs: 0,
            load_avg_one_min: 0.0,
        }
    }

    fn key(k: &str) -> InputEvent {
        InputEvent::Key(types::KeyEvent {
            key: k.to_string(),
            modifiers: types::Modifiers {
                ctrl: false,
                shift: false,
                alt: false,
                meta: false,
            },
            pressed: true,
        })
    }

    // G3: init -> timer-fired returns get-system-stats -> deliver stats -> view
    // shows the CPU percentage. The whole loop runs through the wasmtime host
    // with no subprocess.
    #[test]
    fn g3_effect_roundtrip() -> wasmtime::Result<()> {
        let mut app =
            WasmView::load_ephemeral_run("sysmon-g3", &fixture(), StateStore::ephemeral())?;

        let startup = app.init(&empty_snapshot(), (400.0, 300.0), &[])?;
        assert!(
            startup.iter().any(|e| matches!(e, Effect::GetSystemStats)),
            "init should request system stats"
        );

        let effects = app.update(&InputEvent::TimerFired(1))?;
        assert!(
            effects.iter().any(|e| matches!(e, Effect::GetSystemStats)),
            "timer-fired should request fresh system stats"
        );

        let after = app.update(&InputEvent::SystemStatsResult(mock_stats(42.0)))?;
        assert!(after.is_empty(), "stats-result produces no further effects");

        let tree = app.view()?;
        assert!(
            tree.visible_text().contains("42.0%"),
            "view should render the delivered CPU percentage"
        );
        Ok(())
    }

    // G5: '=' x3 raises the poll interval to 5000ms, persisted through
    // host-state to disk. Reloading the store and re-initing yields a startup
    // timer at the persisted 5000ms interval.
    #[test]
    fn g5_state_persists_across_reload() -> wasmtime::Result<()> {
        let path = std::env::temp_dir().join(format!("plexi-wasm-g5-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);

        {
            let store = StateStore::persistent(path.clone())?;
            let mut app = WasmView::load_ephemeral_run("sysmon-g5", &fixture(), store)?;
            app.init(&empty_snapshot(), (400.0, 300.0), &[])?;
            // default 2000ms + 3 x 1000ms = 5000ms
            for _ in 0..3 {
                app.update(&key("="))?;
            }
        }

        let store = StateStore::persistent(path.clone())?;
        let snapshot = store.snapshot();
        let mut reloaded = WasmView::load_ephemeral_run("sysmon-g5-reload", &fixture(), store)?;
        let startup = reloaded.init(&snapshot, (400.0, 300.0), &[])?;
        assert!(
            startup
                .iter()
                .any(|e| matches!(e, Effect::SetTimer(t) if t.delay_ms == 5000)),
            "reloaded plugin should start its poll timer at the persisted 5000ms interval"
        );

        let _ = std::fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn state_store_traces_load_miss_hit_and_persist_replacement() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("sysmon").join("workspace.json");

        assert_eq!(
            StateStore::format_load_trace(&path, None),
            format!(
                "wasm_state: no state found for scope=context at {} (fallback=none)",
                path.display()
            )
        );
        assert!(
            !StateStore::format_load_trace(&path, None).contains('\n'),
            "load miss trace must be one line"
        );

        let mut store = StateStore::persistent(path.clone())?;
        assert!(store.snapshot().entries.is_empty());
        assert!(
            !path.exists(),
            "opening a missing store does not create a file"
        );

        store.set("interval".to_string(), b"5000".to_vec()).unwrap();
        let first_bytes = std::fs::read(&path)?;
        assert_eq!(
            StateStore::format_persist_trace(&path, first_bytes.len(), false),
            format!(
                "wasm_state: persisted state scope=context bytes={} replaced=false to {}",
                first_bytes.len(),
                path.display()
            )
        );

        store.set("interval".to_string(), b"6000".to_vec()).unwrap();
        let second_bytes = std::fs::read(&path)?;
        assert_eq!(
            StateStore::format_persist_trace(&path, second_bytes.len(), true),
            format!(
                "wasm_state: persisted state scope=context bytes={} replaced=true to {}",
                second_bytes.len(),
                path.display()
            )
        );
        assert!(
            !StateStore::format_persist_trace(&path, second_bytes.len(), true).contains('\n'),
            "persist trace must be one line"
        );

        let reloaded = StateStore::persistent(path.clone())?;
        assert_eq!(reloaded.get("interval").as_deref(), Some(&b"6000"[..]));
        assert_eq!(
            StateStore::format_load_trace(&path, Some((second_bytes.len(), 1))),
            format!(
                "wasm_state: loaded state scope=context bytes={} entries=1 from {}",
                second_bytes.len(),
                path.display()
            )
        );
        Ok(())
    }
}
