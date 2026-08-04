// WasmApp — wasmtime-backed runtime for Plexi v2 WASM component apps.
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
use std::path::{Path, PathBuf};

use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

// The crate itself is named `plexi`; bindgen also generates a top-level `plexi`
// module from the `plexi:platform` package. Nest the generated code so the two
// never collide at a use-path.
pub mod bindings {
    wasmtime::component::bindgen!({
        path: "wit/plexi.wit",
        world: "plexi-full-app",
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
    Alignment, BadgeColor, ButtonStyle, Color, Effect, IndexedNode, InputEvent, KeyEvent,
    Modifiers, MouseButton, MouseEvent, Point, StateSnapshot, SurfaceEvent, SystemStats,
    UiNodeData, UiTree,
};

// ─── Capability grants ─────────────────────────────────────────────────────
//
// host-log is always linked (logging is unconditionally safe). Every other
// host interface is linked only when granted. An app whose component imports an
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
            serde_json::from_slice(&bytes).map_err(std::io::Error::other)?
        } else {
            HashMap::new()
        };
        Ok(StateStore {
            data,
            path: Some(path),
        })
    }

    fn persist(&self) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let bytes = serde_json::to_vec(&self.data).map_err(|e| e.to_string())?;
        std::fs::write(path, bytes).map_err(|e| e.to_string())
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

/// A pipe the guest opened, keyed by the `pipe-handle` it was given. Holds the
/// registry id and kind so sends route to the right backing (binary ring vs.
/// JSON validation).
struct PipeBinding {
    id: String,
    binary: bool,
}

pub struct HostCtx {
    app_id: String,
    state: StateStore,
    // Per-app typed-pipe registry backing the `pipes` import (G13). Binary
    // pipes get a unix socket + drain thread; the registry's Drop closes them
    // when the pane (and its Store) is dropped.
    pipes: crate::host::typed_pipes::TypedPipeRegistry,
    pipe_handles: HashMap<u32, PipeBinding>,
    next_pipe_handle: u32,
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

impl host_log::Host for HostCtx {
    fn debug(&mut self, msg: String) {
        log::debug!("app::{}: {}", self.app_id, msg);
    }
    fn info(&mut self, msg: String) {
        log::info!("app::{}: {}", self.app_id, msg);
    }
    fn warn(&mut self, msg: String) {
        log::warn!("app::{}: {}", self.app_id, msg);
    }
    fn error(&mut self, msg: String) {
        log::error!("app::{}: {}", self.app_id, msg);
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
            },
        );
        log::info!(
            "app::{}: opened {} pipe '{id}' -> handle {handle}",
            self.app_id,
            if binary { "binary" } else { "json" }
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

    fn send_json(&mut self, handle: u32, json: String) -> Result<(), String> {
        let binary = self.pipe_id(handle)?.binary;
        if binary {
            return Err(format!("pipe handle {handle} is a binary pipe"));
        }
        // Validate the frame is well-formed JSON and report malformed payloads
        // to the guest. The WASM runtime does not route JSON pipe payloads —
        // delivery is out-of-band (see docs/wasm-runtime.md); pairing and
        // permission are the host's concern, bytes flow separately.
        serde_json::from_str::<serde_json::Value>(&json)
            .map_err(|e| format!("invalid json: {e}"))?;
        Ok(())
    }

    fn close(&mut self, handle: u32) -> Result<(), String> {
        let binding = self
            .pipe_handles
            .remove(&handle)
            .ok_or_else(|| format!("unknown pipe handle {handle}"))?;
        self.pipes.close(&binding.id);
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
// `WasmApp::audio_process_output` — not through this registry.

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
            "app::{}: audio open_output stream {handle} ({} Hz, {} ch, {} frames)",
            self.app_id,
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
        log::info!("app::{}: audio open_input stream {handle}", self.app_id);
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
        log::info!("app::{}: audio stream {handle} closed", self.app_id);
        Ok(())
    }
}

// ─── gpu import (G7/G11) ─────────────────────────────────────────────────────
//
// Delegates every WIT call to the per-app `GpuDevice`. The device is present
// only when the gpu capability was granted (link-time), so a missing device is
// an internal invariant violation rather than an app error.

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

// ─── WasmApp ───────────────────────────────────────────────────────────────

pub struct WasmApp {
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

impl WasmApp {
    fn engine_and_component(path: &Path) -> wasmtime::Result<(Engine, Component)> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        let engine = Engine::new(&config)?;
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
        app_id: impl Into<String>,
        path: &Path,
        state: StateStore,
    ) -> wasmtime::Result<Self> {
        let app_id = app_id.into();
        let (engine, component) = Self::engine_and_component(path)?;
        let grants = grants_from_component(&engine, &component);
        log::info!("app::{app_id}: ephemeral run grants {grants:?}");
        Self::instantiate(app_id, &engine, &component, &grants, state)
    }

    /// Load an installed/manifest-backed component with explicit host-interface
    /// grants derived from the manifest and remembered user decisions.
    pub fn load_with_grants(
        app_id: impl Into<String>,
        path: &Path,
        state: StateStore,
        grants: Grants,
    ) -> wasmtime::Result<Self> {
        let app_id = app_id.into();
        let (engine, component) = Self::engine_and_component(path)?;
        log::info!("app::{app_id}: manifest wasm grants {grants:?}");
        Self::instantiate(app_id, &engine, &component, &grants, state)
    }

    fn instantiate(
        app_id: String,
        engine: &Engine,
        component: &Component,
        grants: &Grants,
        state: StateStore,
    ) -> wasmtime::Result<Self> {
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
                    .map_err(|e| wasmtime::Error::msg(format!("app::{app_id}: {e}")))?,
            })
        } else {
            None
        };
        if grants.audio {
            audio_rt_control::add_to_linker::<_, HasSelf<HostCtx>>(&mut linker, |c| c)?;
        }

        let ctx = HostCtx {
            app_id: app_id.clone(),
            state,
            pipes: crate::host::typed_pipes::TypedPipeRegistry::new(
                crate::config::config_dir().join("pipes"),
            ),
            pipe_handles: HashMap::new(),
            next_pipe_handle: 0,
            audio_streams: AudioStreams::default(),
            gpu: gpu_device,
            wasi: WasiCtxBuilder::new().build(),
            table: ResourceTable::new(),
        };

        // Instantiate raw, then build per-interface export accessors. The world
        // struct's instantiate would demand every export of `plexi-full-app`;
        // standard apps only export `lifecycle`, so build that unconditionally
        // and probe for the optional `audio-rt-process` export.
        let pre = linker.instantiate_pre(component)?;
        let lifecycle_idx = LifecycleIndices::new(&pre)?;
        let audio_idx = AudioProcessIndices::new(&pre).ok();

        let mut store = Store::new(engine, ctx);
        let instance = pre.instantiate(&mut store)?;
        let lifecycle = lifecycle_idx.load(&mut store, &instance)?;
        let audio = match audio_idx {
            Some(idx) => Some(idx.load(&mut store, &instance)?),
            None => None,
        };

        log::info!(
            "app::{}: wasm component instantiated (audio={})",
            app_id,
            audio.is_some()
        );
        Ok(WasmApp {
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

    pub fn app_id(&self) -> &str {
        &self.store.data().app_id
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
            .ok_or_else(|| wasmtime::Error::msg("audio_process_output on a non-audio app"))?;
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
    /// apps). The live pane uses this to start the host output stream.
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

    /// True when the gpu capability was granted (the app declares a surface).
    pub fn has_gpu(&self) -> bool {
        self.store.data().gpu.is_some()
    }

    /// Allocate a surface texture for a `surface-node`. Returns the opaque
    /// handle to deliver to the guest in a `surface-ready` event, or `None` if
    /// the app has no gpu capability.
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
    /// Surface readbacks performed by this app's device (capture path only).
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

    #[test]
    fn inspect_required_grants_maps_component_imports() -> wasmtime::Result<()> {
        let grants = WasmApp::inspect_required_grants(&audio_fixture())?;

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

    // Stint 0386: pong is compiled against the `plexi-gpu-app` world, which
    // unconditionally imports `pipes` and `gpu`. Ground truth is the compiled
    // component's actual imports (dead-code elimination strips anything the
    // guest never calls, e.g. pong never touches host-state), not the WIT
    // world text.
    #[test]
    fn inspect_required_grants_maps_component_imports_for_pong() -> wasmtime::Result<()> {
        let grants = WasmApp::inspect_required_grants(&pong_fixture())?;

        assert_eq!(
            grants.capability_ids(),
            vec!["pipe.open".to_string(), "gpu.render".to_string()]
        );
        Ok(())
    }

    // Stint 0386: the manifest-backed launch path (`open_installed_wasm_app_pane`)
    // derives `Grants` strictly from `apps/wasm-poc/pong/manifest.toml`'s
    // declared `capabilities` list, then links the component against those
    // grants. Before the fix, `capabilities = []` meant `pipes`/`gpu` were
    // never linked, and wasmtime failed at instantiation with an unresolved
    // import. Parse the real manifest and reproduce that exact derivation to
    // prove the declared capabilities now satisfy the compiled binary's
    // actual required imports end to end.
    #[test]
    fn pong_manifest_capabilities_satisfy_compiled_component_imports() -> wasmtime::Result<()> {
        use crate::app::permissions::{parse_capability_strings, Capability};
        use crate::app::registry::AppManifest;

        let manifest_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("apps/wasm-poc/pong/manifest.toml");
        let manifest_text = std::fs::read_to_string(&manifest_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", manifest_path.display()));
        let manifest: AppManifest =
            toml::from_str(&manifest_text).expect("pong manifest.toml parses");

        let declared_caps = parse_capability_strings(&manifest.app.capabilities.capabilities)
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

        WasmApp::load_with_grants(
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
            app_id: "pipes-test".to_string(),
            state: StateStore::ephemeral(),
            pipes: crate::host::typed_pipes::TypedPipeRegistry::new(dir),
            pipe_handles: std::collections::HashMap::new(),
            next_pipe_handle: 0,
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
        ctx.send_json(h, r#"{"score":3}"#.to_string())
            .expect("valid json");
        assert!(ctx.send_json(h, "{not json".to_string()).is_err());
        assert!(
            ctx.send_binary(h, vec![0]).is_err(),
            "binary on json pipe errors"
        );
        ctx.close(h).expect("close");
    }

    // G12 (host side): a plexi-audio-app component links audio-rt-control and
    // exports audio-rt-process. After the guest starts playing, the host pulls
    // a buffer via process-output and gets non-silent interleaved samples. The
    // whole loop runs synchronously through wasmtime — no OS audio thread, which
    // is exactly the spec's automated pass condition (RT-thread priority and
    // <10ms latency are measured separately by the live path).
    #[test]
    fn g12_audio_process_output_produces_sound() -> wasmtime::Result<()> {
        // Grants are derived from the component's imports; audio-synth imports
        // audio-rt-control, so the audio capability is linked.
        let mut app = WasmApp::load_ephemeral_run(
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

        let mut app = WasmApp::load_ephemeral_run(
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

    fn tree_text(tree: &UiTree) -> String {
        tree.nodes
            .iter()
            .filter_map(|n| match &n.data {
                UiNodeData::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    // G3: init -> timer-fired returns get-system-stats -> deliver stats -> view
    // shows the CPU percentage. The whole loop runs through the wasmtime host
    // with no subprocess.
    #[test]
    fn g3_effect_roundtrip() -> wasmtime::Result<()> {
        let mut app =
            WasmApp::load_ephemeral_run("sysmon-g3", &fixture(), StateStore::ephemeral())?;

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
            tree_text(&tree).contains("42.0%"),
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
            let mut app = WasmApp::load_ephemeral_run("sysmon-g5", &fixture(), store)?;
            app.init(&empty_snapshot(), (400.0, 300.0), &[])?;
            // default 2000ms + 3 x 1000ms = 5000ms
            for _ in 0..3 {
                app.update(&key("="))?;
            }
        }

        let store = StateStore::persistent(path.clone())?;
        let snapshot = store.snapshot();
        let mut reloaded = WasmApp::load_ephemeral_run("sysmon-g5-reload", &fixture(), store)?;
        let startup = reloaded.init(&snapshot, (400.0, 300.0), &[])?;
        assert!(
            startup
                .iter()
                .any(|e| matches!(e, Effect::SetTimer(t) if t.delay_ms == 5000)),
            "reloaded app should start its poll timer at the persisted 5000ms interval"
        );

        let _ = std::fs::remove_file(&path);
        Ok(())
    }
}
