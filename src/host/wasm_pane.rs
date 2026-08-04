// WasmPane — the pane-level driver around a `WasmApp`.
//
// Implements the synchronous Elm effect loop: external input (keys, mouse,
// resize, focus) is queued, `tick` fires due timers and drains the queue, and
// each `update` effect is executed against host services — system stats from a
// pluggable `SystemStatsSource`, ms timers tracked here off frame time, and
// title/status/close surfaced for the pane chrome. Effects that spawn results
// (get-system-stats, timer-fired) push follow-up input events back onto the
// same queue, so the loop converges within one tick. No async runtime.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_queue::ArrayQueue;
use url::Url;

use crate::app::app_trait::KeyDisposition;
use crate::app::permissions::{PermissionState, PermissionStore};
use crate::app_protocol::{
    AiMessage as ProtocolAiMessage, AppEventActor, EventStreamDecl as ProtocolEventStreamDecl,
    ModelTier, TriggerMode,
};
use crate::host::app_timeline::{AppTimeline, EmittedEvent};
use crate::host::services::{
    default_picker_service, FilePickOutcome, FilePickRequest, HttpResponse as HostHttpResponse,
    NetService, PickerService, UreqNetService,
};
use crate::media::audio::{start_output_stream, OutputSession};
use crate::plexi_ai::broker::{AiBroker, AiBrokerRequest, LiveAiBroker};
use crate::ui::theme::Colors;

use super::wasm_app::bindings::plexi::platform::types::{
    AiQueryEffect, AiResponseEvent, AiStreamChunkEvent, AppEventEvent, DeclareEventStreamsEffect,
    DeclareToolsEffect, EmitEventEffect, EventSubscriptionResultEvent,
    EventUnsubscriptionResultEvent, FilePickedEvent, FilePickerMode as WitFilePickerMode,
    FileReadEffect, FileWriteEffect, HttpFetchEffect, HttpResponse as WitHttpResponse,
    OpenFilePickerEffect, SubscribeEventStreamsEffect, ToolCallEvent, ToolResultEffect,
    UiActionEvent, UiValueChangeEvent, UnsubscribeEventStreamsEffect,
};
use super::wasm_app::{
    Effect, InputEvent, KeyEvent, Modifiers, MouseButton, MouseEvent, Point, StateSnapshot,
    SurfaceEvent, SystemStats, UiNodeData, UiTree, WasmApp,
};
use super::wasm_render::RenderResult;

/// A GPU surface the host allocated for the guest's `surface-node`.
/// `width`/`height` are physical pixels (`logical × alloc_ppp`, stint 0527);
/// the guest-facing UI tree keeps declaring logical points.
struct SurfaceState {
    handle: u64,
    width: u32,
    height: u32,
    /// Display scale the surface was allocated for. A change (display move,
    /// scale change) frees the surface so it reallocates at the new
    /// resolution.
    alloc_ppp: f32,
}

/// A WASM effect that requires the live host rather than the pane-local effect
/// loop. `PlexiApp` drains these and returns the corresponding WIT input event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WasmHostEffect {
    ClipboardRead,
    ClipboardWrite {
        text: String,
    },
    Notify {
        title: String,
        body: String,
        icon: Option<String>,
    },
    Spawn {
        app_id: String,
        layout: Option<String>,
        args: Vec<String>,
    },
    SubscribeEvents {
        request_id: String,
        app_id: String,
        event_names: Vec<String>,
        payload_mode: crate::app_protocol::PayloadMode,
        trigger_mode: crate::app_protocol::TriggerMode,
        resource_id: Option<String>,
    },
    UnsubscribeEvents {
        request_id: String,
        subscription_id: String,
    },
    DeclareTools {
        tools: Vec<crate::app_protocol::AiTool>,
    },
    ToolResult {
        call_id: String,
        output_json: Option<String>,
        error: Option<String>,
    },
}

#[derive(Clone)]
pub(crate) struct WasmInputSender {
    queue: Arc<ArrayQueue<InputEvent>>,
    repaint: Option<egui::Context>,
}

impl WasmInputSender {
    fn send(&self, event: InputEvent) -> Result<(), String> {
        self.queue
            .push(event)
            .map_err(|_| "WASM input queue is full".to_string())?;
        if let Some(ctx) = &self.repaint {
            ctx.request_repaint();
        }
        Ok(())
    }

    pub(crate) fn send_tool_call(
        &self,
        call_id: String,
        name: String,
        input_json: String,
        caller_id: String,
    ) -> Result<(), String> {
        self.send(InputEvent::ToolCall(ToolCallEvent {
            call_id,
            name,
            input_json,
            caller_id,
        }))
    }

    #[cfg(test)]
    pub(crate) fn new_for_test() -> (Self, Arc<ArrayQueue<InputEvent>>) {
        let queue = Arc::new(ArrayQueue::new(16));
        (
            Self {
                queue: Arc::clone(&queue),
                repaint: None,
            },
            queue,
        )
    }
}

/// Source of system metrics for the `get-system-stats` effect. Production uses
/// a sysinfo-backed implementation; tests use a fake with fixed values.
pub trait SystemStatsSource: Send {
    fn read(&mut self) -> SystemStats;
}

/// Production [`SystemStatsSource`] backed by `sysinfo`. CPU usage needs two
/// refreshes spaced by a real interval to be meaningful, so the first read
/// after construction reports 0% and subsequent reads (driven by the app's
/// poll timer, ≥1s apart) are accurate. Disk and network byte-rates require
/// interval deltas the host does not yet track and are reported as 0.
pub struct SysinfoStats {
    sys: sysinfo::System,
}

impl SysinfoStats {
    pub fn new() -> Self {
        let mut sys = sysinfo::System::new();
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        SysinfoStats { sys }
    }
}

impl SystemStatsSource for SysinfoStats {
    fn read(&mut self) -> SystemStats {
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();
        let load = sysinfo::System::load_average();
        SystemStats {
            cpu_usage_pct: self.sys.global_cpu_usage(),
            memory_used_bytes: self.sys.used_memory(),
            memory_total_bytes: self.sys.total_memory(),
            disk_read_bps: 0,
            disk_write_bps: 0,
            net_rx_bps: 0,
            net_tx_bps: 0,
            uptime_secs: sysinfo::System::uptime(),
            load_avg_one_min: load.one as f32,
        }
    }
}

struct Timer {
    id: u32,
    delay_ms: u32,
    repeat: bool,
    next_fire_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FsAccess {
    Read,
    Write,
}

#[derive(Default)]
pub struct WasmAccessPolicy {
    fs_read_roots: Vec<PathBuf>,
    fs_write_roots: Vec<PathBuf>,
    /// The subset of write roots granted as directories (folder picks). Only
    /// these permit subdirectory creation; a save-*file* grant never does.
    fs_write_dirs: Vec<PathBuf>,
    net_hosts: Vec<String>,
}

impl WasmAccessPolicy {
    fn grant_fs_read_root(&mut self, root: impl Into<PathBuf>) {
        match canonicalize_scope(root.into(), true) {
            Ok(path) => self.fs_read_roots.push(path),
            Err(e) => log::warn!("wasm access: fs read grant ignored: {e}"),
        }
    }

    fn grant_fs_write_root(&mut self, root: impl Into<PathBuf>) {
        match canonicalize_scope(root.into(), false) {
            Ok(path) => self.fs_write_roots.push(path),
            Err(e) => log::warn!("wasm access: fs write grant ignored: {e}"),
        }
    }

    fn grant_net_host(&mut self, host: impl Into<String>) {
        let host = host.into().to_ascii_lowercase();
        if !host.is_empty() {
            self.net_hosts.push(host);
        }
    }

    /// Register one picker-granted path (stint 0508) as both a read and a
    /// write root. Unlike manifest scopes, a save-as target may not exist
    /// yet, so a missing path resolves through its canonicalized parent.
    /// `is_dir` marks a folder pick, whose subtree may be created and written;
    /// a file pick (open/save) grants only that path. Returns the canonical
    /// path the grant covers.
    fn grant_picked_path(
        &mut self,
        path: impl Into<PathBuf>,
        is_dir: bool,
    ) -> Result<PathBuf, String> {
        let resolved = canonicalize_scope(path.into(), false)?;
        self.fs_read_roots.push(resolved.clone());
        self.fs_write_roots.push(resolved.clone());
        if is_dir {
            self.fs_write_dirs.push(resolved.clone());
        }
        Ok(resolved)
    }

    fn first_root(&self, access: FsAccess) -> Option<&Path> {
        self.fs_roots(access).first().map(PathBuf::as_path)
    }

    fn fs_roots(&self, access: FsAccess) -> &[PathBuf] {
        match access {
            FsAccess::Read => &self.fs_read_roots,
            FsAccess::Write => &self.fs_write_roots,
        }
    }

    fn is_allowed_path(&self, access: FsAccess, path: &Path) -> bool {
        self.fs_roots(access)
            .iter()
            .any(|root| path.starts_with(root))
    }

    /// Whether `path` lies within a directory (folder-pick) grant, so it may be
    /// created as a directory. A save-*file* grant is never a directory grant,
    /// so its path can never be turned into a writable directory tree.
    fn is_within_write_dir(&self, path: &Path) -> bool {
        self.fs_write_dirs.iter().any(|dir| path.starts_with(dir))
    }

    fn allows_host(&self, url: &str) -> Result<(), String> {
        let parsed = Url::parse(url).map_err(|e| format!("invalid url: {e}"))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| "url has no host".to_string())?
            .to_ascii_lowercase();
        if self.net_hosts.iter().any(|allowed| {
            allowed == "*"
                || allowed == &host
                || allowed
                    .strip_prefix("*.")
                    .is_some_and(|suffix| host.ends_with(&format!(".{suffix}")))
        }) {
            Ok(())
        } else {
            Err(format!("net host '{host}' not granted"))
        }
    }
}

fn canonicalize_scope(path: PathBuf, require_existing: bool) -> Result<PathBuf, String> {
    if require_existing {
        return std::fs::canonicalize(&path)
            .map_err(|e| format!("resolve {}: {e}", path.display()));
    }
    canonicalize_for_create(&path)
}

/// Resolves a path that may not exist yet by canonicalizing its deepest
/// existing ancestor and re-appending the not-yet-created tail. Lets a guest
/// lay down a new nested file — a project bundle's `media/<hash>.wav`, or a
/// save-as into a folder the user just named — inside a grant whose directories
/// do not exist on disk yet. Callers must have already rejected `..`/prefix
/// components, so every tail segment is a plain name.
fn canonicalize_for_create(path: &Path) -> Result<PathBuf, String> {
    if path.exists() {
        return std::fs::canonicalize(path).map_err(|e| format!("resolve {}: {e}", path.display()));
    }
    let mut node = path;
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    loop {
        let parent = node
            .parent()
            .ok_or_else(|| format!("path has no existing ancestor: {}", path.display()))?;
        let name = node
            .file_name()
            .ok_or_else(|| format!("path has no file name: {}", path.display()))?;
        tail.push(name.to_os_string());
        if parent.exists() {
            let mut resolved = std::fs::canonicalize(parent)
                .map_err(|e| format!("resolve {}: {e}", parent.display()))?;
            resolved.extend(tail.iter().rev());
            return Ok(resolved);
        }
        node = parent;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WasmCapabilityPrompt {
    capability_id: String,
}

impl WasmCapabilityPrompt {
    pub fn capability_id(&self) -> &str {
        &self.capability_id
    }
}

/// Live RT-audio output for an audio-world app. The host UI thread tops up
/// `ring` from the guest's `process-output`; the cpal callback owned by
/// `_session` drains it on the audio thread. Negotiated `sample_rate`/`channels`
/// come from the device and may differ from what the guest requested — the
/// guest is driven at the negotiated values so pitch stays correct.
struct AudioOut {
    ring: Arc<ArrayQueue<f32>>,
    _session: OutputSession,
    handle: u32,
    sample_rate: u32,
    channels: u32,
    buffer_frames: u32,
    state: u64,
}

pub struct WasmPane {
    app: WasmApp,
    stats: Box<dyn SystemStatsSource>,
    queue: VecDeque<InputEvent>,
    timers: Vec<Timer>,
    wants_close: bool,
    pending_title: Option<String>,
    pending_status: Option<String>,
    audio: Option<AudioOut>,
    surface: Option<SurfaceState>,
    access: WasmAccessPolicy,
    session_grants: HashSet<String>,
    session_blocks: HashSet<String>,
    pending_capability_prompts: VecDeque<WasmCapabilityPrompt>,
    permission_store: Option<PermissionStore>,
    permission_workspace_root: PathBuf,
    deferred_ai_queries: VecDeque<AiQueryEffect>,
    net: Arc<dyn NetService>,
    ai_broker: Arc<dyn AiBroker>,
    app_timeline: Arc<Mutex<AppTimeline>>,
    pane_id: u64,
    /// Host-established context this app instance lives in (stint 0724 Phase
    /// D) — stamped onto every stream this pane declares/emits on. `0`
    /// (matches no real context) until `set_context_id` is called from the
    /// pane-placement closure that also calls `set_pane_id`.
    context_id: u64,
    http_tx: Sender<InputEvent>,
    http_rx: Receiver<InputEvent>,
    /// File-picker backend (stint 0508); scripted in tests / under
    /// `PLEXI_PICKER_SCRIPT` so agents can drive picks without a dialog.
    picker: Arc<dyn PickerService>,
    picker_tx: Sender<(String, bool, FilePickOutcome)>,
    picker_rx: Receiver<(String, bool, FilePickOutcome)>,
    pending_host_effects: VecDeque<WasmHostEffect>,
    external_inputs: Arc<ArrayQueue<InputEvent>>,
    /// Current display scale, set by the live pane every frame. Surfaces
    /// allocate at physical resolution (`logical × ppp`); headless contexts
    /// (tests, `plexi app render`) keep the 1.0 default.
    pixels_per_point: f32,
}

impl WasmPane {
    pub fn new(app: WasmApp, stats: Box<dyn SystemStatsSource>) -> Self {
        let (http_tx, http_rx) = mpsc::channel();
        let (picker_tx, picker_rx) = mpsc::channel();
        WasmPane {
            app,
            stats,
            queue: VecDeque::new(),
            timers: Vec::new(),
            wants_close: false,
            pending_title: None,
            pending_status: None,
            audio: None,
            surface: None,
            access: WasmAccessPolicy::default(),
            session_grants: HashSet::new(),
            session_blocks: HashSet::new(),
            pending_capability_prompts: VecDeque::new(),
            permission_store: None,
            permission_workspace_root: PathBuf::new(),
            deferred_ai_queries: VecDeque::new(),
            net: Arc::new(UreqNetService::new()),
            ai_broker: Arc::new(default_live_broker()),
            app_timeline: crate::host::app_timeline::global(),
            pane_id: 0,
            context_id: 0,
            http_tx,
            http_rx,
            picker: default_picker_service(),
            picker_tx,
            picker_rx,
            pending_host_effects: VecDeque::new(),
            external_inputs: Arc::new(ArrayQueue::new(256)),
            pixels_per_point: 1.0,
        }
    }

    /// Set the pane's current display scale. When it changes while a surface
    /// is allocated, the surface is freed so the next `ensure_surface`
    /// reallocates at the new physical resolution and re-delivers
    /// `surface-ready` to the guest.
    pub fn set_pixels_per_point(&mut self, ppp: f32) {
        if !ppp.is_finite() || ppp <= 0.0 {
            return;
        }
        self.pixels_per_point = ppp;
        if let Some(s) = &self.surface {
            if (s.alloc_ppp - ppp).abs() > f32::EPSILON {
                log::info!(
                    "wasm gpu: ppp changed {} -> {ppp}; freeing surface (texture {}) for realloc",
                    s.alloc_ppp,
                    s.handle
                );
                self.app.free_surface(s.handle);
                self.surface = None;
            }
        }
    }

    #[cfg(test)]
    fn grant_fs_read_root(&mut self, root: impl Into<PathBuf>) {
        self.access.grant_fs_read_root(root);
    }

    #[cfg(test)]
    fn grant_fs_write_root(&mut self, root: impl Into<PathBuf>) {
        self.access.grant_fs_write_root(root);
    }

    #[cfg(test)]
    fn grant_net_host(&mut self, host: impl Into<String>) {
        self.access.grant_net_host(host);
    }

    #[cfg(test)]
    fn set_net_service(&mut self, net: Arc<dyn NetService>) {
        self.net = net;
    }

    #[cfg(test)]
    fn set_picker_service(&mut self, picker: Arc<dyn PickerService>) {
        self.picker = picker;
    }

    #[cfg(test)]
    fn set_ai_broker(&mut self, ai_broker: Arc<dyn AiBroker>) {
        self.ai_broker = ai_broker;
    }

    #[cfg(test)]
    fn set_app_timeline(&mut self, app_timeline: Arc<Mutex<AppTimeline>>) {
        self.app_timeline = app_timeline;
    }

    pub fn set_pane_id(&mut self, pane_id: u64) {
        self.pane_id = pane_id;
    }

    /// Stamp the host-established context this instance lives in — called
    /// alongside `set_pane_id` at pane placement (stint 0724 Phase D).
    /// Always called AFTER `set_pane_id` (see the placement closure in
    /// `pane_ops/create.rs`), so `self.pane_id` is already the real id here —
    /// that ordering is what lets this same call also stamp the owning scope
    /// onto the instance's typed-pipe registry (Phase D 2/2), since both
    /// halves of `Scope::Pane` are only known once both setters have run.
    pub fn set_context_id(&mut self, context_id: u64) {
        self.context_id = context_id;
        self.app.set_pipe_owner(crate::host::scope::Scope::Pane {
            pane_id: self.pane_id,
            context_id,
        });
    }

    pub fn with_remembered_capabilities(
        mut self,
        workspace_root: PathBuf,
        permission_store: PermissionStore,
        granted: HashSet<String>,
        blocked: HashSet<String>,
    ) -> Self {
        for capability_id in granted {
            self.session_grants.insert(capability_id.clone());
            self.apply_capability_grant(&capability_id);
        }
        self.session_blocks.extend(blocked);
        self.permission_workspace_root = workspace_root;
        self.permission_store = Some(permission_store);
        self
    }

    /// Run the guest's `init`, execute its startup effects, and converge the
    /// resulting input queue (e.g. the first stats request).
    pub fn init(
        &mut self,
        snapshot: &super::wasm_app::StateSnapshot,
        size: (f32, f32),
        now_ms: u64,
        args: &[String],
    ) -> wasmtime::Result<()> {
        let effects = self.app.init(snapshot, size, args)?;
        for e in effects {
            self.exec(e, now_ms);
        }
        self.drain(now_ms)?;
        // The guest opens its audio stream during init; start the host output
        // and prime the ring once it has.
        self.start_audio();
        self.pump_audio()?;
        // If the guest's view declares a surface-node, allocate its GPU texture
        // and deliver `surface-ready` so it can set up its pipeline and render.
        self.ensure_surface(now_ms)
    }

    /// Enqueue an external input event for the next `tick`.
    pub fn push_input(&mut self, event: InputEvent) {
        self.queue.push_back(event);
    }

    pub(crate) fn input_sender(&self, repaint: egui::Context) -> WasmInputSender {
        WasmInputSender {
            queue: Arc::clone(&self.external_inputs),
            repaint: Some(repaint),
        }
    }

    fn collect_external_inputs(&mut self) {
        while let Some(event) = self.external_inputs.pop() {
            self.queue.push_back(event);
        }
    }

    fn has_external_inputs(&self) -> bool {
        !self.external_inputs.is_empty()
    }

    fn has_pending_inputs(&self) -> bool {
        !self.queue.is_empty() || self.has_external_inputs()
    }

    /// Deliver a semantic action from the host command surface through the
    /// same guest `update()` path as a rendered button click.
    pub fn dispatch_ui_action(
        &mut self,
        handler_id: impl Into<String>,
        now_ms: u64,
    ) -> wasmtime::Result<()> {
        let handler_id = handler_id.into();
        log::info!("wasm ui: host action handler '{handler_id}'");
        self.queue
            .push_back(InputEvent::UiAction(UiActionEvent { handler_id }));
        self.drain(now_ms)
    }

    pub fn has_pending_capability_prompt(&self) -> bool {
        !self.pending_capability_prompts.is_empty()
    }

    pub fn pending_capability_prompt(&self) -> Option<&WasmCapabilityPrompt> {
        self.pending_capability_prompts.front()
    }

    pub fn decide_next_capability_prompt(&mut self, granted: bool) {
        self.decide_next_capability_prompt_inner(granted, false);
    }

    pub fn decide_next_capability_prompt_remembered(&mut self, granted: bool) {
        self.decide_next_capability_prompt_inner(granted, true);
    }

    fn decide_next_capability_prompt_inner(&mut self, granted: bool, remember: bool) {
        let Some(prompt) = self.pending_capability_prompts.pop_front() else {
            return;
        };
        let capability_id = prompt.capability_id;
        if granted {
            self.session_blocks.remove(&capability_id);
            self.session_grants.insert(capability_id.clone());
            self.apply_capability_grant(&capability_id);
            self.queue
                .push_back(InputEvent::CapabilityGranted(capability_id.clone()));
            if capability_id == "ai.query" {
                self.dispatch_deferred_ai_queries();
            }
        } else {
            self.session_grants.remove(&capability_id);
            self.session_blocks.insert(capability_id.clone());
            self.queue
                .push_back(InputEvent::CapabilityDenied(capability_id.clone()));
            if capability_id == "ai.query" {
                self.deny_deferred_ai_queries();
            }
        }
        if remember {
            if let Some(store) = &mut self.permission_store {
                store.set_wasm(
                    self.app.app_id(),
                    &self.permission_workspace_root,
                    &capability_id,
                    if granted {
                        PermissionState::Green
                    } else {
                        PermissionState::Red
                    },
                );
                store.save();
                log::info!(
                    "wasm capability: remembered app_id={} capability={} granted={}",
                    self.app.app_id(),
                    capability_id,
                    granted
                );
            }
        }
        log::info!(
            "wasm capability: decision app_id={} capability={} granted={}",
            self.app.app_id(),
            capability_id,
            granted
        );
        // `permission_workspace_root` is the app's own workspace root — set at
        // launch via `Self::with_remembered_capabilities` — so it doubles as
        // this event's context root with no new lookup. Empty (the
        // pre-launch default) means "not yet known": route global-only.
        let context_root = (!self.permission_workspace_root.as_os_str().is_empty())
            .then_some(self.permission_workspace_root.as_path());
        crate::host::event_log::emit_scoped(
            crate::host::event_log::HostEvent::PermissionDecision {
                app_id: self.app.app_id().to_string(),
                capability: capability_id,
                granted,
                timestamp: crate::host::event_log::now_timestamp(),
            },
            context_root,
        );
    }

    /// Fire any due timers and drain the input queue. `now_ms` is monotonic
    /// elapsed milliseconds since the pane started.
    pub fn tick(&mut self, now_ms: u64) -> wasmtime::Result<()> {
        self.collect_external_inputs();
        self.collect_http_results();
        self.collect_picker_results();
        self.fire_timers(now_ms);
        self.drain(now_ms)?;
        self.pump_audio()?;
        self.ensure_surface(now_ms)
    }

    /// True once a live audio output stream is running. The live pane uses this
    /// to keep requesting repaints so the ring stays topped up.
    pub fn has_audio(&self) -> bool {
        self.audio.is_some()
    }

    /// Start the host output stream if the guest opened one and it isn't running
    /// yet. A device failure is logged and leaves the pane silent — never fatal.
    fn start_audio(&mut self) {
        if self.audio.is_some() {
            return;
        }
        let Some((handle, rate, channels, buffer_frames)) = self.app.output_stream_config() else {
            return;
        };
        // Ring holds ~8 buffers so a stalled UI thread doesn't underrun audibly.
        let capacity = (buffer_frames.max(1) * channels.max(1) * 8) as usize;
        let ring = Arc::new(ArrayQueue::new(capacity));
        match start_output_stream(rate, channels as u16, Arc::clone(&ring)) {
            Ok(session) => {
                let sample_rate = session.sample_rate;
                let channels = session.channels;
                log::info!(
                    "wasm audio: output stream started (stream {handle}, {sample_rate} Hz, {channels} ch)"
                );
                self.audio = Some(AudioOut {
                    ring,
                    _session: session,
                    handle,
                    sample_rate,
                    channels,
                    buffer_frames,
                    state: 0,
                });
            }
            Err(e) => log::warn!("wasm audio: output stream failed, pane stays silent: {e}"),
        }
    }

    /// Top up the audio ring from the guest's `process-output` while it has room
    /// for at least one more buffer. Runs on the UI thread (which owns the
    /// Store); the cpal callback only pops, so the RT thread never re-enters
    /// wasmtime.
    fn pump_audio(&mut self) -> wasmtime::Result<()> {
        let Some(audio) = self.audio.as_mut() else {
            return Ok(());
        };
        let frame_samples = (audio.buffer_frames * audio.channels) as usize;
        while audio.ring.capacity() - audio.ring.len() >= frame_samples {
            let (samples, next) = self.app.audio_process_output(
                audio.handle,
                audio.buffer_frames,
                audio.channels,
                audio.sample_rate,
                audio.state,
            )?;
            audio.state = next;
            for s in samples {
                if audio.ring.push(s).is_err() {
                    break;
                }
            }
        }
        Ok(())
    }

    /// Allocate the GPU surface and deliver `surface-ready` the first time the
    /// guest's view declares a surface-node. Idempotent: a no-op once allocated
    /// or when the app has no gpu capability.
    fn ensure_surface(&mut self, now_ms: u64) -> wasmtime::Result<()> {
        if self.surface.is_some() || !self.app.has_gpu() {
            return Ok(());
        }
        let tree = self.app.view()?;
        let Some((logical_w, logical_h)) = first_surface_dims(&tree) else {
            return Ok(());
        };
        // Allocate at physical resolution (stint 0527): the composited rect is
        // `logical` points = `logical × ppp` pixels on screen, so a texture
        // allocated at logical dimensions gets bilinearly upscaled and every
        // HiDPI app pane renders at half resolution. The guest-facing
        // coordinate space stays logical — `surface-ready` reports the
        // guest's declared dimensions, its NDC render passes fill the whole
        // (physical) attachment, and its view tree keeps declaring logical
        // points — so app code is untouched.
        let ppp = self.pixels_per_point;
        let width = ((logical_w as f32) * ppp).round().max(1.0) as u32;
        let height = ((logical_h as f32) * ppp).round().max(1.0) as u32;
        let Some(handle) = self.app.alloc_surface(width, height) else {
            return Ok(());
        };
        log::info!(
            "wasm gpu: surface allocated {width}x{height} physical \
             ({logical_w}x{logical_h} logical @ ppp {ppp}) (texture {handle})"
        );
        self.surface = Some(SurfaceState {
            handle,
            width,
            height,
            alloc_ppp: ppp,
        });
        self.queue.push_back(InputEvent::SurfaceReady(SurfaceEvent {
            texture_handle: handle,
            width: logical_w,
            height: logical_h,
        }));
        self.drain(now_ms)
    }

    /// Read the current surface texture back to an RGBA image, if a surface is
    /// allocated. Test-only: pixel assertions in gate tests. Live composition
    /// never blocks the UI thread on a synchronous readback.
    #[cfg(test)]
    pub fn read_surface(&self) -> Option<image::RgbaImage> {
        let s = self.surface.as_ref()?;
        self.app.read_surface(s.handle)
    }

    /// Surface dimensions, if allocated.
    pub fn surface_size(&self) -> Option<(u32, u32)> {
        self.surface.as_ref().map(|s| (s.width, s.height))
    }

    /// sRGB view of the live surface texture for zero-copy egui compositing.
    pub fn surface_srgb_view(&self) -> Option<wgpu::TextureView> {
        let s = self.surface.as_ref()?;
        self.app.surface_srgb_view(s.handle)
    }

    /// Surface readbacks performed by this pane's device (capture path only).
    pub fn surface_readbacks(&self) -> u64 {
        self.app.surface_readbacks()
    }

    /// Queue a nonblocking copy of the live surface for the dedicated-device
    /// fallback (no shared render state to composite into directly).
    pub fn request_surface_readback(&mut self) -> Result<(), String> {
        let handle = self
            .surface
            .as_ref()
            .ok_or_else(|| "no surface allocated".to_string())?
            .handle;
        self.app.request_surface_readback(handle)
    }

    /// Drain the newest completed dedicated-device surface frame, if ready.
    pub fn take_surface_readback(&mut self) -> Option<Result<image::RgbaImage, String>> {
        self.app.take_surface_readback()
    }

    pub fn view(&mut self) -> wasmtime::Result<UiTree> {
        self.app.view()
    }

    /// Route typed-node interactions collected during rendering back into the
    /// guest's normal `update()` loop.
    pub fn apply_render_result(
        &mut self,
        result: RenderResult,
        now_ms: u64,
    ) -> wasmtime::Result<bool> {
        let mut queued = false;
        for handler_id in result.actions {
            log::info!("wasm ui: action handler '{handler_id}'");
            self.queue
                .push_back(InputEvent::UiAction(UiActionEvent { handler_id }));
            queued = true;
        }
        for (handler_id, value) in result.value_changes {
            log::info!("wasm ui: value-change handler '{handler_id}'");
            self.queue
                .push_back(InputEvent::UiValueChange(UiValueChangeEvent {
                    handler_id,
                    value,
                }));
            queued = true;
        }
        // Canvas pointer events (click and press/move/release drag samples,
        // stint 0510), already inverted into the app's declared canvas space.
        for click in result.canvas_clicks {
            log::info!(
                "wasm ui: mouse ({:.1},{:.1}) button={:?} pressed={}",
                click.x,
                click.y,
                click.button,
                click.pressed
            );
            self.queue.push_back(InputEvent::Mouse(MouseEvent {
                position: Point {
                    x: click.x,
                    y: click.y,
                },
                button: click.button.map(|name| match name {
                    "right" => MouseButton::Right,
                    "middle" => MouseButton::Middle,
                    _ => MouseButton::Left,
                }),
                pressed: click.pressed,
                scroll_delta: None,
            }));
            queued = true;
        }
        if queued {
            self.drain(now_ms)?;
        }
        Ok(queued)
    }

    pub fn wants_close(&self) -> bool {
        self.wants_close
    }

    /// Earliest pending timer deadline (monotonic ms), if any. The live render
    /// loop uses this to schedule the next repaint so timers fire on time
    /// without busy-polling.
    pub fn next_deadline_ms(&self) -> Option<u64> {
        self.timers.iter().map(|t| t.next_fire_ms).min()
    }

    /// The cadence the app declared for itself: the shortest repeating timer it
    /// registered. A surface app drives its simulation off a repeating timer, so
    /// that interval *is* its requested frame rate — the host's `FrameClock`
    /// paces to it rather than a construction-time constant. `None` when the app
    /// declared no repeating timer, which leaves the clock at its default.
    ///
    /// `set-timer` is the general timer API, so this reads a cadence out of a
    /// signal that is not exclusively about frames: a surface app whose *only*
    /// repeating timer is a slow status poll gets paced at that poll rate. That
    /// is the honest reading of what the app asked to be woken at — and input
    /// still forces an immediate repaint of its own accord — but a dedicated
    /// frame-cadence signal in the WIT world would say it outright. Adding one
    /// is an ABI change, deliberately out of scope for stint 0552.
    pub fn declared_frame_interval(&self) -> Option<Duration> {
        self.timers
            .iter()
            .filter(|t| t.repeat && t.delay_ms > 0)
            .map(|t| t.delay_ms)
            .min()
            .map(|ms| Duration::from_millis(ms as u64))
    }

    pub fn take_title(&mut self) -> Option<String> {
        self.pending_title.take()
    }

    pub fn take_status(&mut self) -> Option<String> {
        self.pending_status.take()
    }

    pub fn take_host_effects(&mut self) -> Vec<WasmHostEffect> {
        self.pending_host_effects.drain(..).collect()
    }

    pub fn complete_host_effect(&mut self, event: InputEvent) {
        self.queue.push_back(event);
    }

    fn fire_timers(&mut self, now_ms: u64) {
        let mut fired: Vec<u32> = Vec::new();
        self.timers.retain_mut(|t| {
            if now_ms >= t.next_fire_ms {
                fired.push(t.id);
                if t.repeat {
                    t.next_fire_ms = now_ms + t.delay_ms as u64;
                    true
                } else {
                    false
                }
            } else {
                true
            }
        });
        for id in fired {
            self.queue.push_back(InputEvent::TimerFired(id));
        }
    }

    fn collect_http_results(&mut self) {
        while let Ok(event) = self.http_rx.try_recv() {
            self.queue.push_back(event);
        }
    }

    fn drain(&mut self, now_ms: u64) -> wasmtime::Result<()> {
        while let Some(event) = self.queue.pop_front() {
            let effects = self.app.update(&event)?;
            for e in effects {
                self.exec(e, now_ms);
            }
        }
        Ok(())
    }

    fn exec(&mut self, effect: Effect, now_ms: u64) {
        match effect {
            Effect::GetSystemStats => {
                let stats = self.stats.read();
                self.queue.push_back(InputEvent::SystemStatsResult(stats));
            }
            Effect::SetTimer(t) => {
                let next_fire_ms = now_ms + t.delay_ms as u64;
                if let Some(existing) = self.timers.iter_mut().find(|x| x.id == t.id) {
                    existing.delay_ms = t.delay_ms;
                    existing.repeat = t.repeat;
                    existing.next_fire_ms = next_fire_ms;
                } else {
                    self.timers.push(Timer {
                        id: t.id,
                        delay_ms: t.delay_ms,
                        repeat: t.repeat,
                        next_fire_ms,
                    });
                }
            }
            Effect::CancelTimer(id) => {
                self.timers.retain(|t| t.id != id);
            }
            Effect::SetTitle(title) => {
                self.pending_title = Some(title);
            }
            Effect::SetStatus(status) => {
                self.pending_status = Some(status);
            }
            Effect::CloseSelf => {
                self.wants_close = true;
            }
            Effect::RequestCapability(cap) => {
                self.request_capability(cap);
            }
            Effect::FileRead(req) => {
                log::info!("wasm fs: file-read {}", req.path);
                let result = self.read_file(req);
                self.queue.push_back(InputEvent::FileReadResult(result));
            }
            Effect::FileWrite(req) => {
                log::info!("wasm fs: file-write {}", req.path);
                let result = self.write_file(req);
                self.queue.push_back(InputEvent::FileWriteResult(result));
            }
            Effect::HttpFetch(req) => {
                log::info!("wasm net: {} {}", req.method, req.url);
                self.fetch_http(req);
            }
            Effect::AiQuery(req) => {
                log::info!("wasm ai: ai-query {}", req.request_id);
                self.handle_ai_query(req);
            }
            Effect::DeclareEventStreams(req) => {
                log::info!("wasm events: declare {} stream(s)", req.streams.len());
                let result = self.declare_event_streams(req);
                self.queue
                    .push_back(InputEvent::DeclareEventStreamsResult(result));
            }
            Effect::EmitEvent(req) => {
                log::info!("wasm events: emit '{}'", req.event);
                let result = self.emit_event(req);
                self.queue.push_back(InputEvent::EmitEventResult(result));
            }
            Effect::SubscribeEventStreams(req) => self.subscribe_event_streams(req),
            Effect::UnsubscribeEventStreams(req) => self.unsubscribe_event_streams(req),
            Effect::DeclareTools(req) => self.declare_tools(req),
            Effect::ToolResult(req) => self.tool_result(req),
            Effect::ClipboardRead => {
                self.queue_host_effect("clipboard.read", WasmHostEffect::ClipboardRead, |err| {
                    InputEvent::ClipboardReadResult(Err(err))
                })
            }
            Effect::ClipboardWrite(text) => self.queue_host_effect(
                "clipboard.write",
                WasmHostEffect::ClipboardWrite { text },
                |err| InputEvent::ClipboardWriteResult(Err(err)),
            ),
            Effect::Notify(req) => self.queue_host_effect(
                "notify",
                WasmHostEffect::Notify {
                    title: req.title,
                    body: req.body,
                    icon: req.icon,
                },
                |err| InputEvent::NotifyResult(Err(err)),
            ),
            Effect::Spawn(req) => self.queue_host_effect(
                "spawn.app",
                WasmHostEffect::Spawn {
                    app_id: req.app_id,
                    layout: req.layout,
                    args: req.args,
                },
                |err| InputEvent::SpawnResult(Err(err)),
            ),
            Effect::OpenFilePicker(req) => {
                log::info!(
                    "wasm picker: open-file-picker request_id={} mode={:?} multiple={} filter={:?}",
                    req.request_id,
                    req.mode,
                    req.multiple,
                    req.filter
                );
                self.open_file_picker(req);
            }
        }
    }

    /// Service one `open-file-picker` effect (stint 0508). The dialog (or
    /// scripted queue) runs on a background thread; the outcome re-enters the
    /// pane through `picker_rx` in `collect_picker_results`, where the picked
    /// paths become fs grants before `file-picked` reaches the guest.
    fn open_file_picker(&mut self, req: OpenFilePickerEffect) {
        if !self.has_session_grant("fs.pick") {
            log::info!(
                "wasm picker: request {} denied: missing capability fs.pick",
                req.request_id
            );
            self.queue
                .push_back(InputEvent::FilePickCancelled(req.request_id));
            return;
        }
        let is_folder = matches!(req.mode, WitFilePickerMode::Folder);
        let request = FilePickRequest {
            filter: req.filter,
            multiple: req.multiple,
            mode: match req.mode {
                WitFilePickerMode::Open => crate::app_protocol::FilePickerMode::Open,
                WitFilePickerMode::Folder => crate::app_protocol::FilePickerMode::Folder,
                WitFilePickerMode::Save => crate::app_protocol::FilePickerMode::Save,
            },
        };
        let picker = Arc::clone(&self.picker);
        let tx = self.picker_tx.clone();
        std::thread::spawn(move || {
            let outcome = picker.pick(&request);
            let _ = tx.send((req.request_id, is_folder, outcome));
        });
    }

    /// Drain finished picker requests: register grants for picked paths, then
    /// queue the guest-facing event. Cancellation creates no grant and leaves
    /// no request state behind.
    fn collect_picker_results(&mut self) {
        while let Ok((request_id, is_folder, outcome)) = self.picker_rx.try_recv() {
            match outcome {
                FilePickOutcome::Picked(paths) => {
                    let mut granted = Vec::new();
                    for path in paths {
                        match self.access.grant_picked_path(&path, is_folder) {
                            Ok(resolved) => {
                                log::info!(
                                    "wasm picker: fs grant registered for picked path {}",
                                    resolved.display()
                                );
                                granted.push(resolved.display().to_string());
                            }
                            Err(error) => {
                                log::error!(
                                    "wasm picker: picked path {} not granted: {error}",
                                    path.display()
                                );
                            }
                        }
                    }
                    if granted.is_empty() {
                        log::error!(
                            "wasm picker: pick {request_id}: no picked path could be granted; cancelling"
                        );
                        self.queue
                            .push_back(InputEvent::FilePickCancelled(request_id));
                    } else {
                        self.queue
                            .push_back(InputEvent::FilePicked(FilePickedEvent {
                                request_id,
                                paths: granted,
                            }));
                    }
                }
                FilePickOutcome::Cancelled => {
                    log::info!("wasm picker: pick {request_id} cancelled");
                    self.queue
                        .push_back(InputEvent::FilePickCancelled(request_id));
                }
            }
        }
    }

    fn queue_host_effect(
        &mut self,
        capability_id: &str,
        effect: WasmHostEffect,
        denied: impl FnOnce(String) -> InputEvent,
    ) {
        if self.has_session_grant(capability_id) {
            log::info!(
                "wasm effect: app_id={} capability={} effect={effect:?}",
                self.app.app_id(),
                capability_id
            );
            self.pending_host_effects.push_back(effect);
        } else {
            log::info!(
                "wasm effect: denied app_id={} capability={capability_id}",
                self.app.app_id()
            );
            self.queue
                .push_back(denied(format!("capability '{capability_id}' is required")));
        }
    }

    fn request_capability(&mut self, capability_id: String) {
        log::info!(
            "wasm capability: request app_id={} capability={}",
            self.app.app_id(),
            capability_id
        );
        if self.session_grants.contains(&capability_id) {
            self.queue
                .push_back(InputEvent::CapabilityGranted(capability_id));
            return;
        }
        if self.session_blocks.contains(&capability_id) {
            self.queue
                .push_back(InputEvent::CapabilityDenied(capability_id));
            return;
        }
        if self
            .pending_capability_prompts
            .iter()
            .any(|prompt| prompt.capability_id == capability_id)
        {
            return;
        }
        self.pending_capability_prompts
            .push_back(WasmCapabilityPrompt { capability_id });
    }

    fn apply_capability_grant(&mut self, capability_id: &str) {
        if let Some(path) = capability_id.strip_prefix("fs:read:") {
            self.access.grant_fs_read_root(PathBuf::from(path));
        } else if let Some(path) = capability_id.strip_prefix("fs:write:") {
            self.access.grant_fs_write_root(PathBuf::from(path));
        } else if let Some(host) = capability_id.strip_prefix("net:fetch:") {
            self.access.grant_net_host(host);
        } else if matches!(
            capability_id,
            "clipboard.read" | "clipboard.write" | "notify" | "spawn.app"
        ) {
            log::info!("wasm capability: session grant enabled {capability_id}");
        } else {
            log::info!(
                "wasm capability: session grant recorded without runtime access for unknown capability '{capability_id}'"
            );
        }
    }

    fn has_session_grant(&self, capability_id: &str) -> bool {
        self.session_grants.contains(capability_id)
    }

    fn has_session_block(&self, capability_id: &str) -> bool {
        self.session_blocks.contains(capability_id)
    }

    fn handle_ai_query(&mut self, req: AiQueryEffect) {
        if self.has_session_grant("ai.query") {
            self.dispatch_ai_query(req);
            return;
        }
        if self.has_session_block("ai.query") {
            self.queue_ai_denied(req.request_id, "capability denied: ai.query blocked");
            return;
        }
        if self.deferred_ai_queries.len() >= 16 {
            self.queue_ai_denied(
                req.request_id,
                "capability withheld: too many deferred ai.query requests pending consent",
            );
            return;
        }
        self.request_capability("ai.query".to_string());
        self.deferred_ai_queries.push_back(req);
    }

    fn dispatch_deferred_ai_queries(&mut self) {
        let deferred = std::mem::take(&mut self.deferred_ai_queries);
        for req in deferred {
            self.dispatch_ai_query(req);
        }
    }

    fn deny_deferred_ai_queries(&mut self) {
        let deferred = std::mem::take(&mut self.deferred_ai_queries);
        for req in deferred {
            self.queue_ai_denied(req.request_id, "capability denied: ai.query");
        }
    }

    fn queue_ai_denied(&mut self, request_id: String, message: &str) {
        self.queue
            .push_back(InputEvent::AiResponse(AiResponseEvent {
                request_id,
                content: None,
                tokens_in: 0,
                tokens_out: 0,
                error: Some(message.to_string()),
            }));
    }

    fn dispatch_ai_query(&mut self, req: AiQueryEffect) {
        let model_tier = match parse_model_tier(&req.model_tier) {
            Ok(tier) => tier,
            Err(e) => {
                self.queue_ai_denied(req.request_id, &e);
                return;
            }
        };
        let broker = Arc::clone(&self.ai_broker);
        let tx = self.http_tx.clone();
        let app_id = self.app.app_id().to_string();
        let request_id = req.request_id.clone();
        let messages = req
            .messages
            .into_iter()
            .map(|msg| ProtocolAiMessage {
                role: msg.role,
                content: msg.content,
            })
            .collect::<Vec<_>>();
        std::thread::Builder::new()
            .name(format!("wasm-ai-query-{app_id}-{request_id}"))
            .spawn(move || {
                let chunk_tx = tx.clone();
                let chunk_request_id = request_id.clone();
                let mut on_delta = move |d: crate::plexi_ai::turn_loop::TurnDelta<'_>| {
                    let event = match d {
                        crate::plexi_ai::turn_loop::TurnDelta::Text(text) => {
                            InputEvent::AiStreamChunk(AiStreamChunkEvent {
                                request_id: chunk_request_id.clone(),
                                delta: text.to_string(),
                                reasoning: None,
                                done: false,
                            })
                        }
                        crate::plexi_ai::turn_loop::TurnDelta::Reasoning(reasoning) => {
                            InputEvent::AiStreamChunk(AiStreamChunkEvent {
                                request_id: chunk_request_id.clone(),
                                delta: String::new(),
                                reasoning: Some(reasoning.to_string()),
                                done: false,
                            })
                        }
                        // App-facing `ai.query` streams carry only text and
                        // reasoning; tool-call generation progress drives the
                        // assistant transcript, not app protocol events.
                        crate::plexi_ai::turn_loop::TurnDelta::ToolCallProgress { .. } => return,
                    };
                    if let Err(e) = chunk_tx.send(event) {
                        log::warn!("wasm ai: stream receiver dropped: {e}");
                    }
                };
                let response = broker.dispatch(
                    AiBrokerRequest {
                        app_id,
                        model_tier,
                        concrete_model: None,
                        reasoning_effort: None,
                        system: req.system,
                        messages,
                        tools: Vec::new(),
                        workspace_root: None,
                        open_panes: crate::plexi_ai::broker::get_pane_snapshot(),
                        tool_dispatcher: None,
                        cancel: crate::plexi_ai::CancelToken::new(),
                        max_tool_iterations: None,
                    },
                    &mut on_delta,
                );
                let _ = tx.send(InputEvent::AiStreamChunk(AiStreamChunkEvent {
                    request_id: request_id.clone(),
                    delta: String::new(),
                    reasoning: None,
                    done: true,
                }));
                let _ = tx.send(InputEvent::AiResponse(AiResponseEvent {
                    request_id,
                    content: response.content,
                    tokens_in: response.tokens_in,
                    tokens_out: response.tokens_out,
                    error: response.error,
                }));
            })
            .expect("failed to spawn wasm ai-query thread");
    }

    fn declare_event_streams(
        &mut self,
        req: DeclareEventStreamsEffect,
    ) -> Result<Vec<String>, String> {
        let mut streams = Vec::with_capacity(req.streams.len());
        for stream in req.streams {
            let schema = serde_json::from_str::<serde_json::Value>(&stream.schema_json)
                .map_err(|e| format!("declare_event_streams: invalid schema json: {e}"))?;
            streams.push(ProtocolEventStreamDecl {
                name: stream.name,
                schema,
                description: stream.description,
            });
        }
        self.app_timeline.lock().unwrap().declare_streams(
            self.context_id,
            self.app.app_id(),
            streams,
        )
    }

    fn emit_event(&mut self, req: EmitEventEffect) -> Result<u64, String> {
        let payload = match req.payload_json {
            Some(json) => Some(
                serde_json::from_str::<serde_json::Value>(&json)
                    .map_err(|e| format!("emit_event: invalid payload json: {e}"))?,
            ),
            None => None,
        };
        let actor = parse_app_event_actor(&req.actor)?;
        let suggested_trigger = req
            .suggested_trigger
            .as_deref()
            .map(parse_trigger_mode)
            .transpose()?;
        let emitted = EmittedEvent {
            event: req.event,
            actor,
            actor_id: req.actor_id,
            caused_by: req.caused_by,
            summary: req.summary,
            resource_id: req.resource_id,
            resource_scope: req.resource_scope,
            revision_after: req.revision_after,
            payload,
            state_ref: req.state_ref,
            revision_before: req.revision_before,
            rollback_token: req.rollback_token,
            changed_resources: req.changed_resources,
            suggested_trigger,
        };
        let outcome = self.app_timeline.lock().unwrap().record_event(
            self.context_id,
            self.app.app_id(),
            self.pane_id,
            emitted,
        )?;
        Ok(outcome.event_id)
    }

    fn subscribe_event_streams(&mut self, req: SubscribeEventStreamsEffect) {
        let payload_mode = parse_payload_mode(&req.payload_mode);
        let trigger_mode = parse_trigger_mode(&req.trigger_mode);
        match (payload_mode, trigger_mode) {
            (Ok(payload_mode), Ok(trigger_mode)) => {
                log::info!(
                    "wasm events: subscribe app_id={} publisher={} streams={:?}",
                    self.app.app_id(),
                    req.app_id,
                    req.event_names
                );
                self.pending_host_effects
                    .push_back(WasmHostEffect::SubscribeEvents {
                        request_id: req.request_id,
                        app_id: req.app_id,
                        event_names: req.event_names,
                        payload_mode,
                        trigger_mode,
                        resource_id: req.resource_id,
                    });
            }
            (Err(error), _) | (_, Err(error)) => {
                self.queue.push_back(InputEvent::EventSubscriptionResult(
                    EventSubscriptionResultEvent {
                        request_id: req.request_id,
                        subscription_id: None,
                        error: Some(error),
                    },
                ));
            }
        }
    }

    fn unsubscribe_event_streams(&mut self, req: UnsubscribeEventStreamsEffect) {
        log::info!(
            "wasm events: unsubscribe app_id={} subscription={}",
            self.app.app_id(),
            req.subscription_id
        );
        self.pending_host_effects
            .push_back(WasmHostEffect::UnsubscribeEvents {
                request_id: req.request_id,
                subscription_id: req.subscription_id,
            });
    }

    fn declare_tools(&mut self, req: DeclareToolsEffect) {
        let parsed = req
            .tools
            .into_iter()
            .map(|tool| {
                let input_schema =
                    serde_json::from_str(&tool.input_schema_json).map_err(|error| {
                        format!("tool '{}': invalid input schema: {error}", tool.name)
                    })?;
                let output_schema =
                    serde_json::from_str(&tool.output_schema_json).map_err(|error| {
                        format!("tool '{}': invalid output schema: {error}", tool.name)
                    })?;
                Ok(crate::app_protocol::AiTool {
                    name: tool.name,
                    description: tool.description,
                    input_schema,
                    output_schema,
                    timeout_ms: tool.timeout_ms,
                    read_only: tool.read_only,
                })
            })
            .collect::<Result<Vec<_>, String>>();
        match parsed {
            Ok(tools) => {
                log::info!(
                    "wasm tools: app_id={} declared {} tool(s)",
                    self.app.app_id(),
                    tools.len()
                );
                self.pending_host_effects
                    .push_back(WasmHostEffect::DeclareTools { tools });
            }
            Err(error) => self
                .queue
                .push_back(InputEvent::DeclareToolsResult(Err(error))),
        }
    }

    fn tool_result(&mut self, req: ToolResultEffect) {
        log::info!(
            "wasm tools: app_id={} completed call_id={}",
            self.app.app_id(),
            req.call_id
        );
        self.pending_host_effects
            .push_back(WasmHostEffect::ToolResult {
                call_id: req.call_id,
                output_json: req.output_json,
                error: req.error,
            });
    }

    fn scoped_path(
        &self,
        access: FsAccess,
        path: &str,
        require_existing_file: bool,
    ) -> Result<PathBuf, String> {
        let raw = Path::new(path);
        if raw
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::Prefix(_)))
        {
            return Err(format!("path not allowed: {path}"));
        }
        let full = if raw.is_absolute() {
            raw.to_path_buf()
        } else {
            let root = self
                .access
                .first_root(access)
                .ok_or_else(|| "no filesystem scope granted".to_string())?;
            root.join(raw)
        };
        let resolved = if require_existing_file {
            std::fs::canonicalize(&full).map_err(|e| format!("resolve {}: {e}", full.display()))?
        } else {
            canonicalize_for_create(&full)?
        };
        if self.access.is_allowed_path(access, &resolved) {
            Ok(resolved)
        } else {
            Err(format!("path outside granted scope: {}", full.display()))
        }
    }

    fn read_file(&self, req: FileReadEffect) -> Result<Vec<u8>, String> {
        let path = self.scoped_path(FsAccess::Read, &req.path, true)?;
        let bytes = std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        if bytes.len() > crate::host::MAX_FILE_IO_BYTES {
            return Err(format!(
                "read {}: file is {} bytes, over the {}-byte per-call file I/O limit",
                path.display(),
                bytes.len(),
                crate::host::MAX_FILE_IO_BYTES
            ));
        }
        Ok(bytes)
    }

    fn write_file(&self, req: FileWriteEffect) -> Result<(), String> {
        let path = self.scoped_path(FsAccess::Write, &req.path, false)?;
        if req.content.len() > crate::host::MAX_FILE_IO_BYTES {
            return Err(format!(
                "write {}: payload is {} bytes, over the {}-byte per-call file I/O limit",
                path.display(),
                req.content.len(),
                crate::host::MAX_FILE_IO_BYTES
            ));
        }
        // Create intermediate directories inside a folder grant so a guest can
        // lay down a nested asset (e.g. a project bundle's `media/` folder)
        // without a separate mkdir affordance. Only directories strictly below
        // a granted write root are created: a save-*file* grant (root == the
        // file) never has its path materialised as a directory tree.
        if let Some(parent) = path.parent() {
            if self.access.is_within_write_dir(parent) {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("write {}: create parent dir: {e}", path.display()))?;
            }
        }
        std::fs::write(&path, req.content).map_err(|e| format!("write {}: {e}", path.display()))
    }

    fn fetch_http(&mut self, req: HttpFetchEffect) {
        if let Err(e) = self.access.allows_host(&req.url) {
            log::warn!("wasm net: denied {}: {e}", req.url);
            self.queue_denied_http(e);
            return;
        }
        let net = Arc::clone(&self.net);
        let tx = self.http_tx.clone();
        std::thread::spawn(move || {
            let headers: HashMap<String, String> = req.headers.into_iter().collect();
            let body = req
                .body
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
            let response = net.http(
                &req.method,
                &req.url,
                &headers,
                body.as_deref(),
                crate::host::services::DEFAULT_MAX_HTTP_BODY_BYTES,
            );
            let _ = tx.send(InputEvent::HttpResponse(host_http_to_wit(response)));
        });
    }

    fn queue_denied_http(&mut self, message: String) {
        self.queue
            .push_back(InputEvent::HttpResponse(WitHttpResponse {
                status: 403,
                headers: vec![("x-plexi-error".to_string(), "capability-denied".to_string())],
                body: message.into_bytes(),
            }));
    }
}

fn host_http_to_wit(response: HostHttpResponse) -> WitHttpResponse {
    let mut headers = Vec::new();
    for (name, values) in response.response_headers {
        for value in values {
            headers.push((name.clone(), value));
        }
    }
    if response.truncated {
        // The WIT response has no truncation field; surface the cut honestly
        // instead of letting a capped body read as the full document.
        headers.push(("x-plexi-body-truncated".to_string(), "true".to_string()));
    }
    let body = match response.error {
        Some(err) if response.body.is_empty() => err.into_bytes(),
        Some(err) => format!("{}\n{}", response.body, err).into_bytes(),
        None => response.body.into_bytes(),
    };
    WitHttpResponse {
        status: response.status,
        headers,
        body,
    }
}

fn default_live_broker() -> LiveAiBroker {
    LiveAiBroker::new(crate::config::PlexiConfig::load().ai)
}

fn parse_model_tier(raw: &str) -> Result<ModelTier, String> {
    match raw {
        "low" | "Low" => Ok(ModelTier::Low),
        "medium" | "Medium" => Ok(ModelTier::Medium),
        "high" | "High" => Ok(ModelTier::High),
        other => Err(format!("invalid model tier: {other}")),
    }
}

fn parse_app_event_actor(raw: &str) -> Result<AppEventActor, String> {
    match raw {
        "user" | "User" => Ok(AppEventActor::User),
        "agent" | "Agent" => Ok(AppEventActor::Agent),
        "app" | "App" => Ok(AppEventActor::App),
        "system" | "System" => Ok(AppEventActor::System),
        other => Err(format!("invalid app event actor: {other}")),
    }
}

fn parse_trigger_mode(raw: &str) -> Result<TriggerMode, String> {
    match raw {
        "never" | "Never" => Ok(TriggerMode::Never),
        "conversation" | "Conversation" => Ok(TriggerMode::Conversation),
        "ambient" | "Ambient" => Ok(TriggerMode::Ambient),
        "ask" | "Ask" => Ok(TriggerMode::Ask),
        other => Err(format!("invalid trigger mode: {other}")),
    }
}

fn parse_payload_mode(raw: &str) -> Result<crate::app_protocol::PayloadMode, String> {
    match raw {
        "off" | "Off" => Ok(crate::app_protocol::PayloadMode::Off),
        "summary" | "Summary" => Ok(crate::app_protocol::PayloadMode::Summary),
        "full" | "Full" => Ok(crate::app_protocol::PayloadMode::Full),
        "state_ref" | "StateRef" => Ok(crate::app_protocol::PayloadMode::StateRef),
        other => Err(format!("invalid payload mode: {other}")),
    }
}

// ─── Live adapter ───────────────────────────────────────────────────────────
//
// `LiveWasmPane` bridges the time-injected, headless [`WasmPane`] to the host's
// live egui render loop. It owns the monotonic clock (so `WasmPane` stays pure
// and testable), runs `init` lazily on the first frame once the pane size is
// known, translates egui key input into guest `InputEvent`s using the host's
// printable-vs-named key split, and renders the guest's view
// tree each frame. A fatal guest/runtime error is captured and shown in place
// rather than propagated — a misbehaving WASM app must never crash the host.

pub struct LiveWasmPane {
    inner: WasmPane,
    started: Instant,
    spawn_name: String,
    title: Option<String>,
    pending_init: Option<StateSnapshot>,
    error: Option<String>,
    /// Concatenated text of the last rendered view tree. Lets the headless
    /// scene runner assert on rendered content without re-entering the guest.
    last_text: String,
    /// Monotonic generation of the tree handed to the renderer. This runtime
    /// re-evaluates `view()` every frame, so every painted tree is genuinely
    /// new; the counter is what host-owned edit buffers reconcile against
    /// (stint 0720).
    tree_seq: u64,
    /// Runtime-neutral semantics retained from the last committed guest tree.
    semantic_state: crate::host::pane::SemanticPaneState,
    /// egui texture id of the guest surface, registered once into the host's
    /// shared renderer and sampled live (zero-copy). `None` until first present
    /// or when no shared device exists (headless).
    surface_id: Option<egui::TextureId>,
    /// Dimensions backing `surface_id`; a change forces re-registration.
    surface_dims: Option<(u32, u32)>,
    /// Readback fallback: an egui texture re-uploaded each frame when no shared
    /// wgpu device is available (eframe built without the wgpu backend). The
    /// slow path the zero-copy registration replaces; kept for resilience.
    fallback_tex: Option<egui::TextureHandle>,
    /// Host-owned fixed-timestep pacing for the surface.
    clock: super::wasm_frame::FrameClock,
    /// Host-owned wall-clock metrics. Apps display these; they never invent FPS.
    telemetry: super::wasm_frame::FrameTelemetry,
    /// Wall-clock instant of the previous presented frame, for interval metrics.
    last_present: Option<Instant>,
    /// Guest clock for a surface pane, advanced only by whole fixed timesteps
    /// the [`FrameClock`](super::wasm_frame::FrameClock) granted. `None` until
    /// the first surface step anchors it to pane-elapsed time. Kept in
    /// nanoseconds because the grid is not a whole number of milliseconds — at
    /// 60 Hz, truncating each 16.666 ms step to 16 ms would bleed 40 ms of guest
    /// time per real second.
    sim_ns: Option<u64>,
    /// Launch arguments (`plexi app open <path> -- <args>`), forwarded to the
    /// guest's `init` as its argv. Empty for palette/registry launches.
    launch_args: Vec<String>,
}

impl LiveWasmPane {
    /// Build a live pane. `init` is deferred to the first `ui` call, when the
    /// egui region size is known. `snapshot` is the persisted state to restore.
    pub fn new(
        inner: WasmPane,
        spawn_name: impl Into<String>,
        snapshot: StateSnapshot,
        launch_args: Vec<String>,
    ) -> Self {
        LiveWasmPane {
            inner,
            started: Instant::now(),
            spawn_name: spawn_name.into(),
            title: None,
            pending_init: Some(snapshot),
            error: None,
            last_text: String::new(),
            tree_seq: 0,
            semantic_state: crate::host::pane::SemanticPaneState::empty("wasm"),
            surface_id: None,
            surface_dims: None,
            fallback_tex: None,
            clock: super::wasm_frame::FrameClock::new(60),
            telemetry: super::wasm_frame::FrameTelemetry::new(240),
            last_present: None,
            sim_ns: None,
            launch_args,
        }
    }

    fn now_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    /// The clock the guest sees: the host's fixed grid once a surface pane has
    /// started stepping, wall time otherwise. Every path that hands the guest a
    /// timestamp goes through here, so a timer can never be armed in one domain
    /// and fired in another.
    fn guest_now_ms(&self) -> u64 {
        self.sim_ns
            .map(|ns| ns / 1_000_000)
            .unwrap_or_else(|| self.now_ms())
    }

    /// Advance a surface pane by the host's fixed timestep, returning the guest
    /// clock afterwards and the step the clock granted.
    ///
    /// This is the whole host-owned pacing contract in one place: retarget the
    /// clock at the cadence the app declared, ask it how many fixed sim steps
    /// this repaint is owed, run exactly that many, and discard the remainder it
    /// refused to grant. A dropped step is skipped, never banked and replayed
    /// later — that is what keeps a long stall from spiralling.
    ///
    /// Dropped *time*, however, still advances the guest clock. The guest gets
    /// one clock, quantised to the fixed grid but tracking wall time to within a
    /// step; a stalled pane skips simulation rather than falling permanently
    /// behind. Letting it lag instead would split the pane into two clock
    /// domains — timers armed from `background_tick`/`dispatch_ui_action` run on
    /// wall time — and those timers would then fire a whole stall late.
    fn step_surface(
        &mut self,
        now_inst: Instant,
        wall_ms: u64,
    ) -> wasmtime::Result<(u64, super::wasm_frame::FrameStep)> {
        let declared = self.inner.declared_frame_interval();
        let before = self.clock.target_interval();
        self.clock.retarget(declared);
        if self.clock.target_interval() != before {
            log::info!(
                "app::{}: surface frame clock retargeted {}ms -> {}ms ({})",
                self.spawn_name,
                before.as_millis(),
                self.clock.target_interval().as_millis(),
                if declared.is_some() {
                    "app-declared cadence"
                } else {
                    "app declares none; host default"
                },
            );
        }
        let step = self.clock.advance(now_inst);
        self.telemetry.record_dropped(step.dropped);
        let dt_ns = (self.clock.target_interval().as_nanos() as u64).max(1);
        // The very first grid point IS the arming instant: the guest's timers
        // were set against `wall_ms`, so stepping past it before the first tick
        // would fire them a whole interval early.
        let mut sim_ns = self
            .sim_ns
            .unwrap_or_else(|| (wall_ms * 1_000_000).saturating_sub(dt_ns));
        let mut result = Ok(());
        for _ in 0..step.steps {
            sim_ns += dt_ns;
            result = self.inner.tick(sim_ns / 1_000_000);
            if result.is_err() {
                break;
            }
        }
        // Skipped simulation, not skipped time.
        sim_ns += dt_ns * step.dropped as u64;
        self.sim_ns = Some(sim_ns);
        result.map(|()| (sim_ns / 1_000_000, step))
    }

    /// Delay to request for the next surface repaint. Routes through the shared
    /// [`repaint_delay_until`](super::wasm_frame::repaint_delay_until) so the
    /// egui predicted-frame compensation stays identical to the CPython path.
    fn surface_repaint_delay(&self, now_inst: Instant, predicted_frame: Duration) -> Duration {
        super::wasm_frame::repaint_delay_until(
            now_inst + self.clock.target_interval(),
            now_inst,
            predicted_frame,
        )
    }

    /// True while the app is alive (no fatal error, has not asked to close).
    // Consumed only by the cfg(test) scene runner today; retained as the
    // pane-liveness accessor for the host status surface.
    #[allow(dead_code)]
    pub fn is_running(&self) -> bool {
        self.error.is_none() && !self.inner.wants_close()
    }

    /// Text content of the most recently rendered view, for scene assertions.
    // Reads `last_text` (written every frame in `ui`); keep non-cfg(test) so
    // that field stays live in the production build.
    #[allow(dead_code)]
    pub fn last_render_text(&self) -> &str {
        &self.last_text
    }

    fn fail(&mut self, ctx: &str, e: impl std::fmt::Display) {
        let msg = format!("{ctx}: {e}");
        log::error!("wasm pane error — {msg}");
        self.error = Some(msg);
    }

    pub fn wants_close(&self) -> bool {
        self.inner.wants_close()
    }

    pub fn display_name(&self) -> String {
        self.title
            .clone()
            .unwrap_or_else(|| self.spawn_name.clone())
    }

    pub(crate) fn semantic_state(&self) -> &crate::host::pane::SemanticPaneState {
        &self.semantic_state
    }

    pub fn has_pending_capability_prompt(&self) -> bool {
        self.inner.has_pending_capability_prompt()
    }

    pub fn set_pane_id(&mut self, pane_id: u64) {
        self.inner.set_pane_id(pane_id);
    }

    pub fn set_context_id(&mut self, context_id: u64) {
        self.inner.set_context_id(context_id);
    }

    pub fn take_host_effects(&mut self) -> Vec<WasmHostEffect> {
        self.inner.take_host_effects()
    }

    pub(crate) fn input_sender(&self, repaint: egui::Context) -> WasmInputSender {
        self.inner.input_sender(repaint)
    }

    pub(crate) fn queue_outbound_event(&mut self, event: crate::app_protocol::PlexiEvent) {
        let event = match event {
            crate::app_protocol::PlexiEvent::AppEventsSubscribed {
                request_id,
                subscription_id,
                error,
            } => InputEvent::EventSubscriptionResult(EventSubscriptionResultEvent {
                request_id,
                subscription_id,
                error,
            }),
            crate::app_protocol::PlexiEvent::AppEventsUnsubscribed {
                request_id,
                removed,
                error,
            } => InputEvent::EventUnsubscriptionResult(EventUnsubscriptionResultEvent {
                request_id,
                removed,
                error,
            }),
            crate::app_protocol::PlexiEvent::AppEvent {
                subscription_id,
                app_id,
                event,
                event_id,
                resource_id,
                trigger_mode,
                summary,
                payload,
                state_ref,
                created_at,
            } => InputEvent::AppEvent(AppEventEvent {
                subscription_id,
                app_id,
                event,
                event_id,
                resource_id,
                trigger_mode: format!("{trigger_mode:?}").to_ascii_lowercase(),
                summary,
                payload_json: payload.map(|value| value.to_string()),
                state_ref,
                created_at,
            }),
            crate::app_protocol::PlexiEvent::DeclareEventStreamsResult { streams, error } => {
                InputEvent::DeclareEventStreamsResult(match (streams, error) {
                    (Some(streams), None) => Ok(streams),
                    (_, Some(error)) => Err(error),
                    _ => Err("declare event streams returned no result".to_string()),
                })
            }
            crate::app_protocol::PlexiEvent::EmitEventResult { sequence, error } => {
                InputEvent::EmitEventResult(match (sequence, error) {
                    (Some(sequence), None) => Ok(sequence),
                    (_, Some(error)) => Err(error),
                    _ => Err("emit event returned no result".to_string()),
                })
            }
            other => {
                log::warn!("wasm input: unsupported host event {other:?}");
                return;
            }
        };
        self.inner.push_input(event);
    }

    pub(crate) fn background_tick(&mut self) {
        if self.pending_init.is_some() {
            return;
        }
        if let Err(error) = self.inner.tick(self.guest_now_ms()) {
            self.fail("background step", error);
            return;
        }
        match self.inner.view() {
            Ok(tree) => {
                self.last_text = collect_tree_text(&tree);
                self.semantic_state = crate::host::pane::SemanticPaneState::from_wasm_tree(&tree);
            }
            Err(error) => self.fail("background view", error),
        }
    }

    pub(crate) fn needs_background_tick(&self) -> bool {
        self.inner.has_pending_inputs()
    }

    /// Deliver `plexi app action` to the guest and refresh the cached semantic
    /// tree immediately so pane-state callers observe the resulting view.
    pub fn dispatch_ui_action(&mut self, handler_id: impl Into<String>) -> Result<(), String> {
        if self.pending_init.is_some() {
            return Err("WASM app has not rendered its initial frame yet".to_string());
        }
        self.inner
            .dispatch_ui_action(handler_id, self.guest_now_ms())
            .map_err(|e| format!("WASM action failed: {e}"))?;
        let tree = self
            .inner
            .view()
            .map_err(|e| format!("WASM view after action failed: {e}"))?;
        self.last_text = collect_tree_text(&tree);
        self.semantic_state = crate::host::pane::SemanticPaneState::from_wasm_tree(&tree);
        Ok(())
    }

    pub fn complete_host_effect(&mut self, event: InputEvent) {
        self.inner.complete_host_effect(event);
    }

    pub fn draw_capability_modal(&mut self, ctx: &egui::Context, colors: &Colors) {
        let Some(prompt) = self.inner.pending_capability_prompt() else {
            return;
        };
        let capability_id = prompt.capability_id().to_string();
        let actions = [
            crate::ui::dialog::DialogAction::new(
                "grant_once",
                "Grant once",
                crate::ui::button::ButtonKind::Primary,
            )
            .shortcut(crate::ui::dialog::DialogShortcut::new(
                &["Enter"],
                egui::Modifiers::NONE,
                egui::Key::Enter,
            )),
            crate::ui::dialog::DialogAction::new(
                "grant_always",
                "Always allow",
                crate::ui::button::ButtonKind::Primary,
            )
            .shortcut(crate::ui::dialog::DialogShortcut::new(
                &["Shift", "Enter"],
                egui::Modifiers::SHIFT,
                egui::Key::Enter,
            )),
            crate::ui::dialog::DialogAction::new(
                "deny_once",
                "Deny once",
                crate::ui::button::ButtonKind::Secondary,
            )
            .shortcut(crate::ui::dialog::DialogShortcut::new(
                &["Esc"],
                egui::Modifiers::NONE,
                egui::Key::Escape,
            )),
            crate::ui::dialog::DialogAction::new(
                "deny_always",
                "Always deny",
                crate::ui::button::ButtonKind::Danger,
            )
            .shortcut(crate::ui::dialog::DialogShortcut::new(
                &["Shift", "Esc"],
                egui::Modifiers::SHIFT,
                egui::Key::Escape,
            )),
        ];
        let response = crate::ui::dialog::ActionModal::new(
            "wasm_capability_prompt_overlay",
            "Capability request",
            &actions,
        )
        .width(crate::overlays::MODAL_WIDTH)
        .show(ctx, colors, |ui| {
            crate::ui::typography::caption(
                ui,
                format!("{} requests:", self.display_name()),
                colors,
            );
            crate::ui::typography::caption(ui, capability_id.clone(), colors);
            crate::ui::typography::caption(
                ui,
                "Always decisions are saved for this app and workspace.",
                colors,
            );
        });

        let decision = match response.selected {
            Some("grant_once") => Some((true, false)),
            Some("grant_always") => Some((true, true)),
            Some("deny_once") => Some((false, false)),
            Some("deny_always") => Some((false, true)),
            _ if response.dismissed => Some((false, false)),
            _ => None,
        };

        if let Some((granted, remember)) = decision {
            if remember {
                self.inner.decide_next_capability_prompt_remembered(granted);
            } else {
                self.inner.decide_next_capability_prompt(granted);
            }
            if let Err(e) = self.inner.drain(self.guest_now_ms()) {
                self.fail("capability decision", e);
            }
            ctx.request_repaint();
        }
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        colors: &Colors,
        pending_click: Option<crate::host::pane::PendingPaneClick>,
        pane_key: u64,
    ) {
        if let Some(err) = &self.error {
            ui.colored_label(colors.danger, err);
            return;
        }

        let size = ui.available_size();
        // The guest clock, not wall time: a surface pane whose texture is
        // momentarily freed (a display-scale change) still takes the non-surface
        // branch below, and must not arm timers in a different domain than the
        // one the surface path fires them in.
        let mut now = self.guest_now_ms();
        // Keep the guest's surface allocation in sync with the display scale
        // (stint 0527) — a display move/scale change frees and reallocates.
        self.inner.set_pixels_per_point(ui.ctx().pixels_per_point());

        let stepped = if let Some(snapshot) = self.pending_init.take() {
            log::info!(
                "app::{}: wasm init args={:?}",
                self.spawn_name,
                self.launch_args
            );
            self.inner
                .init(&snapshot, (size.x, size.y), now, &self.launch_args)
        } else if self.inner.surface_size().is_some() {
            // Surface panes are paced by the host clock, not by the repaint
            // rate: the guest advances in whole fixed steps on its own clock.
            self.step_surface(Instant::now(), now).map(|(sim, _)| {
                now = sim;
            })
        } else {
            self.inner.tick(now)
        };
        if let Err(e) = stepped {
            self.fail("step", e);
            return;
        }

        if let Some(t) = self.inner.take_title() {
            self.title = Some(t);
        }
        let _ = self.inner.take_status();

        let tree = match self.inner.view() {
            Ok(t) => t,
            Err(e) => {
                self.fail("view", e);
                return;
            }
        };
        self.last_text = collect_tree_text(&tree);
        self.semantic_state = crate::host::pane::SemanticPaneState::from_wasm_tree(&tree);
        // Composite the guest's GPU render zero-copy: register its surface
        // texture into the host's shared egui renderer once and sample it live.
        // The guest re-renders into the same texture every frame, so the
        // registration stays valid — no readback, no per-frame upload. Headless
        // contexts (no shared render state) draw the surface-node placeholder.
        let present_start = Instant::now();
        let surface_tid: Option<egui::TextureId> = match self.inner.surface_size() {
            Some((w, h)) => match super::wasm_gpu::host_render_state() {
                // Preferred: register the guest texture into egui's shared
                // renderer once and sample it live. No readback, no upload.
                // Linear filtering is inert at integer ppp — the surface is
                // allocated at physical resolution and composited into a
                // pixel-snapped rect (1:1 texel→pixel, stint 0527) — and
                // remains only as the fractional-ppp resampling fallback.
                Some(rs) => {
                    if self.surface_id.is_none() || self.surface_dims != Some((w, h)) {
                        if let Some(view) = self.inner.surface_srgb_view() {
                            let mut renderer = rs.renderer.write();
                            let id = renderer.register_native_texture(
                                &rs.device,
                                &view,
                                wgpu::FilterMode::Linear,
                            );
                            if let Some(old) = self.surface_id.replace(id) {
                                renderer.free_texture(&old);
                            }
                            self.surface_dims = Some((w, h));
                        }
                    }
                    self.surface_id
                }
                // Fallback: no shared device. Keep an N-buffered async readback
                // ring in flight so the UI thread never blocks on `poll(Wait)`:
                // queue this frame's copy, then upload whichever previous copy
                // has finished. The texture lags by at most a couple of frames
                // instead of stalling until the current one completes.
                None => {
                    if let Err(e) = self.inner.request_surface_readback() {
                        log::warn!(
                            "wasm present: async surface readback request failed for {}: {e}",
                            self.spawn_name
                        );
                    }
                    match self.inner.take_surface_readback() {
                        Some(Ok(img)) => {
                            let color = egui::ColorImage::from_rgba_unmultiplied(
                                [w as usize, h as usize],
                                img.as_raw(),
                            );
                            match &mut self.fallback_tex {
                                Some(tex) => tex.set(color, egui::TextureOptions::LINEAR),
                                none => {
                                    *none = Some(ui.ctx().load_texture(
                                        format!("wasm-surface-{}", self.spawn_name),
                                        color,
                                        egui::TextureOptions::LINEAR,
                                    ));
                                }
                            }
                        }
                        Some(Err(e)) => {
                            log::warn!(
                                "wasm present: async surface readback failed for {}: {e}",
                                self.spawn_name
                            );
                        }
                        // Nothing completed yet this frame; keep showing the
                        // most recently uploaded texture rather than stalling.
                        None => {}
                    }
                    self.fallback_tex.as_ref().map(|t| t.id())
                }
            },
            None => None,
        };
        self.tree_seq += 1;
        let result = super::wasm_render::render_ui_tree_with_canvas_fits(
            ui,
            &tree,
            colors,
            surface_tid,
            None,
            pending_click,
            Some(crate::ui::focus::SurfaceKey::Pane(pane_key)),
            self.tree_seq,
        );
        match self.inner.apply_render_result(result, now) {
            Ok(true) => {
                match self.inner.view() {
                    Ok(t) => {
                        self.last_text = collect_tree_text(&t);
                        self.semantic_state =
                            crate::host::pane::SemanticPaneState::from_wasm_tree(&t);
                    }
                    Err(e) => {
                        self.fail("view after input", e);
                        return;
                    }
                }
                ui.ctx().request_repaint();
            }
            Ok(false) => {}
            Err(e) => {
                self.fail("input", e);
                return;
            }
        }

        // Host-owned telemetry for surface apps. The clock itself already
        // advanced in `step_surface` (it gates the guest's sim steps); here we
        // only record the wall-clock measurements. The guest never measures its
        // own frame rate.
        if self.inner.surface_size().is_some() {
            let now_inst = Instant::now();
            if let Some(prev) = self.last_present {
                self.telemetry
                    .record_frame(now_inst.saturating_duration_since(prev));
            }
            self.last_present = Some(now_inst);
            self.telemetry.record_present(present_start.elapsed());
            // Sampled, not per-frame, so presentation logging never taxes the
            // hot path the way the old per-frame readback/upload logs did.
            if self.telemetry.frames().is_multiple_of(120) {
                // `readbacks` should stay 0 on the zero-copy path; a climbing
                // count means this pane is on the readback fallback.
                log::info!(
                    "wasm present: surface={} fps={:.0} p95_interval={:.1}ms p95_present={:.2}ms dropped={} readbacks={}",
                    self.spawn_name,
                    self.telemetry.fps(),
                    self.telemetry.p95_interval_ms(),
                    self.telemetry.p95_present_ms(),
                    self.telemetry.dropped(),
                    self.inner.surface_readbacks(),
                );
            }
        }

        // Schedule the next repaint from the soonest active cadence — egui
        // coalesces multiple requests to the earliest. Surface apps pace at the
        // host frame clock; audio tops up its ring; timers fire on deadline.
        let has_surface = self.inner.surface_size().is_some();
        if has_surface {
            let predicted_frame = Duration::from_secs_f32(ui.input(|input| input.predicted_dt));
            let delay = self.surface_repaint_delay(Instant::now(), predicted_frame);
            ui.ctx().request_repaint_after(delay);
        }
        if self.inner.has_audio() {
            ui.ctx().request_repaint_after(Duration::from_millis(15));
        } else if !has_surface {
            if let Some(deadline) = self.inner.next_deadline_ms() {
                ui.ctx()
                    .request_repaint_after(Duration::from_millis(deadline.saturating_sub(now)));
            }
        }
    }

    /// Translate egui key input into guest `InputEvent::Key`s and enqueue them.
    ///
    /// WASM guests match key names literally — there is no SDK normalization
    /// layer as there is for Python apps — so the host emits the canonical short
    /// dialect via [`canonical_key_name`] (`up`/`down`/`left`/`right`, `space`,
    /// `enter`, `escape`, lowercase letters/digits, literal punctuation). Both
    /// press and release edges are forwarded so apps can track held state (a game
    /// paddle, a key being held down); OS auto-repeat is collapsed and Cmd-chords
    /// are reserved for host shortcuts. See [`translate_key_event`].
    pub fn handle_key(&mut self, input: &crate::app::input_router::PlexiInput) -> KeyDisposition {
        let mut consumed = false;
        for event in input.events() {
            match event {
                egui::Event::Key { key, modifiers, .. } => {
                    // Bare Escape is reserved for the host CloseApp binding
                    // (keys.rs, `BindingContext::AppActive`). Reporting it
                    // consumed claims Escape out of the frame's input buffer
                    // before `poll_actions` can fire CloseApp, so the focused
                    // WASM pane never closes on Escape. Skip it — never forward
                    // to the guest, never count toward `consumed`. Cmd+Escape
                    // (context zoom-out) is already dropped by
                    // `translate_key_event`.
                    if *key == egui::Key::Escape && !modifiers.command {
                        continue;
                    }
                    if let Some(ke) = translate_key_event(event) {
                        self.inner.push_input(InputEvent::Key(ke));
                    }
                    consumed = true;
                }
                // Text composition (shift-resolved characters, IME) is reserved
                // for a future text-input channel; game/command apps consume the
                // raw key edges above. Swallow so stray text never leaks to host
                // handlers while a WASM pane is focused.
                egui::Event::Text(_) => consumed = true,
                _ => {}
            }
        }
        if consumed {
            KeyDisposition::Consumed
        } else {
            KeyDisposition::Passthrough
        }
    }
}

/// Join every text node in a view tree into a single string (newline-joined),
/// preserving arena order. Used for headless content assertions.
fn collect_tree_text(tree: &UiTree) -> String {
    tree.nodes
        .iter()
        .filter_map(|n| match &n.data {
            UiNodeData::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Dimensions of the first `surface-node` in the tree, if any. The host uses
/// these to allocate the backing GPU texture.
fn first_surface_dims(tree: &UiTree) -> Option<(u32, u32)> {
    tree.nodes.iter().find_map(|n| match &n.data {
        UiNodeData::Surface(s) => Some((s.width, s.height)),
        _ => None,
    })
}

/// Canonical guest key name for an egui key — the single source of truth for the
/// WASM key dialect. Navigation and whitespace keys get short names
/// (`up`/`down`/`left`/`right`, `space`, `enter`, `escape`, …); digits and
/// punctuation get their literal character; letters and any unmapped key fall
/// back to the lowercased egui name (which is the bare character for letters).
/// Guests match these strings directly, so this table IS the contract.
fn canonical_key_name(key: egui::Key) -> String {
    use egui::Key::*;
    match key {
        ArrowUp => "up".into(),
        ArrowDown => "down".into(),
        ArrowLeft => "left".into(),
        ArrowRight => "right".into(),
        Space => "space".into(),
        Enter => "enter".into(),
        Escape => "escape".into(),
        Tab => "tab".into(),
        Backspace => "backspace".into(),
        Delete => "delete".into(),
        Home => "home".into(),
        End => "end".into(),
        PageUp => "pageup".into(),
        PageDown => "pagedown".into(),
        Insert => "insert".into(),
        Num0 => "0".into(),
        Num1 => "1".into(),
        Num2 => "2".into(),
        Num3 => "3".into(),
        Num4 => "4".into(),
        Num5 => "5".into(),
        Num6 => "6".into(),
        Num7 => "7".into(),
        Num8 => "8".into(),
        Num9 => "9".into(),
        Minus => "-".into(),
        Equals => "=".into(),
        Plus => "+".into(),
        OpenBracket => "[".into(),
        CloseBracket => "]".into(),
        Backslash => "\\".into(),
        Semicolon => ";".into(),
        Quote => "'".into(),
        Backtick => "`".into(),
        Comma => ",".into(),
        Period => ".".into(),
        Slash => "/".into(),
        other => format!("{other:?}").to_lowercase(),
    }
}

/// Translate a single egui event into a guest key edge, or `None` when it
/// carries nothing the guest should see: a non-key event, OS auto-repeat
/// (collapsed so a physical press fires once), or a Cmd-chord (reserved for host
/// shortcuts). Both press and release edges are returned so apps can track held
/// state — this is what makes hold-to-move game input work.
fn translate_key_event(event: &egui::Event) -> Option<KeyEvent> {
    let egui::Event::Key {
        key,
        pressed,
        repeat,
        modifiers,
        ..
    } = event
    else {
        return None;
    };
    if modifiers.command || *repeat {
        return None;
    }
    Some(KeyEvent {
        key: canonical_key_name(*key),
        modifiers: Modifiers {
            ctrl: modifiers.ctrl,
            shift: modifiers.shift,
            alt: modifiers.alt,
            meta: modifiers.command,
        },
        pressed: *pressed,
    })
}

#[cfg(test)]
mod tests {
    use super::super::wasm_app::bindings::plexi::platform::types::{
        AiMessage as WitAiMessage, EventStreamDecl as WitEventStreamDecl, NotificationEffect,
        SpawnEffect,
    };
    use super::*;
    use crate::host::services::HttpResponse as HostHttpResponse;
    use crate::host::wasm_app::{
        KeyEvent, Modifiers, StateSnapshot, StateStore, UiNodeData, WasmApp,
    };
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wasm-fixtures/sysmon.wasm")
    }

    fn counter_fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wasm-fixtures/counter.wasm")
    }

    struct FakeStats {
        cpu: f32,
    }
    impl SystemStatsSource for FakeStats {
        fn read(&mut self) -> SystemStats {
            SystemStats {
                cpu_usage_pct: self.cpu,
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
    }

    fn pane(cpu: f32) -> WasmPane {
        let app = WasmApp::load_ephemeral_run("sysmon-pane", &fixture(), StateStore::ephemeral())
            .expect("load");
        WasmPane::new(app, Box::new(FakeStats { cpu }))
    }

    fn counter_pane() -> WasmPane {
        let app = WasmApp::load_ephemeral_run(
            "counter-pane",
            &counter_fixture(),
            StateStore::ephemeral(),
        )
        .expect("load counter");
        WasmPane::new(app, Box::new(FakeStats { cpu: 0.0 }))
    }

    /// Stint 0724 Phase D 2/2: `WasmPane::set_context_id` — always called
    /// right after `set_pane_id` from the pane-placement closure in
    /// `pane_ops/create.rs` — must stamp the owning `Scope::Pane` onto this
    /// instance's typed-pipe registry (`WasmApp`'s `HostCtx.pipes`), matching
    /// the SAME pane_id/context_id the placement closure assigned. This is
    /// the one owner-scope test the sub-phase requires: it proves the field
    /// is populated and correct at creation, without inventing a cross-pane
    /// consumption check that doesn't exist today (typed pipes are one
    /// registry per pane by construction — see `typed_pipes.rs`'s own
    /// `directed_pipe_routes_to_target_pane_only` test).
    #[test]
    fn typed_pipe_owner_stamped_at_placement_matches_pane_context() {
        let mut pane = counter_pane();
        assert_eq!(
            pane.app.pipe_owner(),
            None,
            "owner must be unstamped before pane placement runs its setters"
        );

        pane.set_pane_id(42);
        pane.set_context_id(7);

        assert_eq!(
            pane.app.pipe_owner(),
            Some(&crate::host::scope::Scope::Pane {
                pane_id: 42,
                context_id: 7,
            }),
            "set_context_id must stamp the owning Scope::Pane using the SAME \
             pane_id set_pane_id just assigned"
        );
    }

    #[test]
    fn queued_host_event_marks_inactive_wasm_pane_ready_to_tick() {
        let mut pane = counter_pane();
        assert!(!pane.has_pending_inputs());
        pane.complete_host_effect(InputEvent::EventUnsubscriptionResult(
            EventUnsubscriptionResultEvent {
                request_id: "unsub-1".to_string(),
                removed: true,
                error: None,
            },
        ));
        assert!(pane.has_pending_inputs());
    }

    fn key(k: &str) -> InputEvent {
        InputEvent::Key(KeyEvent {
            key: k.to_string(),
            modifiers: Modifiers {
                ctrl: false,
                shift: false,
                alt: false,
                meta: false,
            },
            pressed: true,
        })
    }

    fn cpu_text(tree: &UiTree) -> String {
        tree.nodes
            .iter()
            .filter_map(|n| match &n.data {
                UiNodeData::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn tree_text(tree: &UiTree) -> String {
        cpu_text(tree)
    }

    fn file_read(path: &str) -> Effect {
        Effect::FileRead(FileReadEffect {
            path: path.to_string(),
        })
    }

    fn file_write(path: &str, content: &[u8]) -> Effect {
        Effect::FileWrite(FileWriteEffect {
            path: path.to_string(),
            content: content.to_vec(),
        })
    }

    fn http_fetch(url: &str) -> Effect {
        Effect::HttpFetch(HttpFetchEffect {
            url: url.to_string(),
            method: "GET".to_string(),
            headers: vec![("accept".to_string(), "text/plain".to_string())],
            body: None,
        })
    }

    fn pop_event(p: &mut WasmPane) -> InputEvent {
        p.queue.pop_front().expect("queued event")
    }

    fn request_capability(capability_id: impl Into<String>) -> Effect {
        Effect::RequestCapability(capability_id.into())
    }

    #[test]
    fn protected_effects_require_a_grant_then_leave_for_the_live_host() {
        let mut p = pane(0.0);

        p.exec(Effect::ClipboardRead, 0);
        assert!(p.take_host_effects().is_empty());
        assert!(matches!(
            pop_event(&mut p),
            InputEvent::ClipboardReadResult(Err(_))
        ));

        p.exec(Effect::ClipboardWrite("secret".to_string()), 0);
        assert!(p.take_host_effects().is_empty());
        assert!(matches!(
            pop_event(&mut p),
            InputEvent::ClipboardWriteResult(Err(_))
        ));

        grant_capability(&mut p, "clipboard.read");
        p.exec(Effect::ClipboardRead, 0);
        assert_eq!(p.take_host_effects(), vec![WasmHostEffect::ClipboardRead]);
        p.complete_host_effect(InputEvent::ClipboardReadResult(Ok(Some(
            "copied".to_string(),
        ))));
        assert!(matches!(
            pop_event(&mut p),
            InputEvent::ClipboardReadResult(Ok(Some(text))) if text == "copied"
        ));

        grant_capability(&mut p, "clipboard.write");
        p.exec(Effect::ClipboardWrite("secret".to_string()), 0);
        assert_eq!(
            p.take_host_effects(),
            vec![WasmHostEffect::ClipboardWrite {
                text: "secret".to_string()
            }]
        );

        p.complete_host_effect(InputEvent::ClipboardWriteResult(Ok(())));
        assert!(matches!(
            pop_event(&mut p),
            InputEvent::ClipboardWriteResult(Ok(()))
        ));
    }

    #[test]
    fn notify_and_spawn_round_trip_through_live_host_effect_queue() {
        let mut p = pane(0.0);
        grant_capability(&mut p, "notify");
        grant_capability(&mut p, "spawn.app");

        p.exec(
            Effect::Notify(NotificationEffect {
                title: "Saved".to_string(),
                body: "Document saved".to_string(),
                icon: Some("check".to_string()),
            }),
            0,
        );
        p.exec(
            Effect::Spawn(SpawnEffect {
                app_id: "com.plexi.counter".to_string(),
                layout: Some("split_h".to_string()),
                args: vec!["--demo".to_string()],
            }),
            0,
        );
        assert_eq!(
            p.take_host_effects(),
            vec![
                WasmHostEffect::Notify {
                    title: "Saved".to_string(),
                    body: "Document saved".to_string(),
                    icon: Some("check".to_string()),
                },
                WasmHostEffect::Spawn {
                    app_id: "com.plexi.counter".to_string(),
                    layout: Some("split_h".to_string()),
                    args: vec!["--demo".to_string()],
                },
            ]
        );
        p.complete_host_effect(InputEvent::NotifyResult(Ok(())));
        p.complete_host_effect(InputEvent::SpawnResult(Ok(42)));
        assert!(matches!(
            pop_event(&mut p),
            InputEvent::NotifyResult(Ok(()))
        ));
        assert!(matches!(pop_event(&mut p), InputEvent::SpawnResult(Ok(42))));
    }

    fn grant_capability(p: &mut WasmPane, capability_id: &str) {
        p.exec(request_capability(capability_id.to_string()), 0);
        assert_eq!(
            p.pending_capability_prompt()
                .map(WasmCapabilityPrompt::capability_id),
            Some(capability_id)
        );
        p.decide_next_capability_prompt(true);
        match pop_event(p) {
            InputEvent::CapabilityGranted(granted) => assert_eq!(granted, capability_id),
            other => panic!("expected capability-granted event, got {other:?}"),
        }
    }

    struct FakeNet;

    impl NetService for FakeNet {
        fn http(
            &self,
            method: &str,
            url: &str,
            headers: &HashMap<String, String>,
            body: Option<&str>,
            _max_body_bytes: u64,
        ) -> HostHttpResponse {
            let mut response_headers = HashMap::new();
            response_headers.insert("content-type".to_string(), vec!["text/plain".to_string()]);
            HostHttpResponse {
                status: 201,
                truncated: false,
                body: format!(
                    "{method} {url} accept={} body={}",
                    headers.get("accept").map(String::as_str).unwrap_or(""),
                    body.unwrap_or("")
                ),
                error: None,
                response_headers,
            }
        }
    }

    struct FakeAiBroker {
        calls: Arc<AtomicUsize>,
    }

    impl AiBroker for FakeAiBroker {
        fn dispatch(
            &self,
            request: AiBrokerRequest,
            on_delta: &mut dyn FnMut(crate::plexi_ai::turn_loop::TurnDelta<'_>),
        ) -> crate::plexi_ai::broker::AiBrokerResponse {
            self.calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(request.model_tier, ModelTier::Medium);
            assert_eq!(request.messages.len(), 1);
            on_delta(crate::plexi_ai::turn_loop::TurnDelta::Text("stream "));
            crate::plexi_ai::broker::AiBrokerResponse::ok("stream final".to_string(), 3, 4)
        }
    }

    fn ai_query(request_id: &str) -> Effect {
        Effect::AiQuery(AiQueryEffect {
            request_id: request_id.to_string(),
            model_tier: "medium".to_string(),
            system: "You are concise.".to_string(),
            messages: vec![WitAiMessage {
                role: "user".to_string(),
                content: "hello".to_string(),
            }],
        })
    }

    fn declare_stream(name: &str) -> Effect {
        Effect::DeclareEventStreams(DeclareEventStreamsEffect {
            streams: vec![WitEventStreamDecl {
                name: name.to_string(),
                schema_json: r#"{"type":"object"}"#.to_string(),
                description: Some("test stream".to_string()),
            }],
        })
    }

    fn emit_event(name: &str) -> Effect {
        Effect::EmitEvent(EmitEventEffect {
            event: name.to_string(),
            actor: "app".to_string(),
            actor_id: None,
            caused_by: None,
            summary: "Moved".to_string(),
            resource_id: "game-1".to_string(),
            resource_scope: Some("game".to_string()),
            revision_after: "rev-1".to_string(),
            payload_json: Some(r#"{"move":"e4"}"#.to_string()),
            state_ref: None,
            revision_before: None,
            rollback_token: None,
            changed_resources: vec!["game-1".to_string()],
            suggested_trigger: Some("conversation".to_string()),
        })
    }

    fn collect_async_events(p: &mut WasmPane, min_events: usize) {
        for _ in 0..50 {
            p.collect_http_results();
            if p.queue.len() >= min_events {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn open_picker(request_id: &str, mode: WitFilePickerMode, multiple: bool) -> Effect {
        Effect::OpenFilePicker(OpenFilePickerEffect {
            request_id: request_id.to_string(),
            filter: vec![],
            multiple,
            mode,
        })
    }

    fn scripted_picker(
        outcomes: Vec<crate::host::services::FilePickOutcome>,
    ) -> Arc<dyn PickerService> {
        Arc::new(crate::host::services::ScriptedPickerService::from_outcomes(
            outcomes,
        ))
    }

    fn collect_picker_events(p: &mut WasmPane, min_events: usize) {
        for _ in 0..100 {
            p.collect_picker_results();
            if p.queue.len() >= min_events {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Without `fs.pick`, the picker never runs: the guest gets an immediate
    /// `file-pick-cancelled` and no fs grant is created.
    #[test]
    fn file_pick_without_capability_cancels_and_grants_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let secret = dir.path().join("secret.txt");
        std::fs::write(&secret, b"secret").expect("seed");

        let mut p = pane(0.0);
        p.set_picker_service(scripted_picker(vec![
            crate::host::services::FilePickOutcome::Picked(vec![secret.clone()]),
        ]));
        p.exec(open_picker("pick-1", WitFilePickerMode::Open, false), 0);

        match pop_event(&mut p) {
            InputEvent::FilePickCancelled(request_id) => assert_eq!(request_id, "pick-1"),
            other => panic!("expected file-pick-cancelled, got {other:?}"),
        }
        p.exec(file_read(&secret.to_string_lossy()), 0);
        match pop_event(&mut p) {
            InputEvent::FileReadResult(Err(msg)) => {
                assert!(msg.contains("scope"), "unexpected error: {msg}");
            }
            other => panic!("expected denied file-read-result, got {other:?}"),
        }
    }

    /// A scripted pick grants exactly the picked file: reading it through the
    /// delivered absolute path succeeds while a sibling stays rejected.
    #[test]
    fn file_pick_grants_scoped_read_and_rejects_outside_paths() {
        let dir = tempfile::tempdir().expect("tempdir");
        let picked = dir.path().join("picked.txt");
        let sibling = dir.path().join("sibling.txt");
        std::fs::write(&picked, b"picked bytes").expect("seed picked");
        std::fs::write(&sibling, b"nope").expect("seed sibling");

        let mut p = pane(0.0);
        grant_capability(&mut p, "fs.pick");
        p.set_picker_service(scripted_picker(vec![
            crate::host::services::FilePickOutcome::Picked(vec![picked.clone()]),
        ]));
        p.exec(open_picker("pick-2", WitFilePickerMode::Open, false), 0);
        collect_picker_events(&mut p, 1);

        let delivered = match pop_event(&mut p) {
            InputEvent::FilePicked(event) => {
                assert_eq!(event.request_id, "pick-2");
                assert_eq!(event.paths.len(), 1);
                event.paths[0].clone()
            }
            other => panic!("expected file-picked, got {other:?}"),
        };
        assert_eq!(
            std::path::PathBuf::from(&delivered),
            picked.canonicalize().expect("canonical picked")
        );

        p.exec(file_read(&delivered), 0);
        match pop_event(&mut p) {
            InputEvent::FileReadResult(Ok(bytes)) => assert_eq!(bytes, b"picked bytes"),
            other => panic!("expected successful file-read-result, got {other:?}"),
        }

        p.exec(file_read(&sibling.to_string_lossy()), 0);
        match pop_event(&mut p) {
            InputEvent::FileReadResult(Err(msg)) => {
                assert!(
                    msg.contains("outside granted scope"),
                    "unexpected error: {msg}"
                );
            }
            other => panic!("expected denied file-read-result, got {other:?}"),
        }
    }

    /// A save-as pick grants a path that does not exist yet; writing then
    /// reading it back through the grant round-trips.
    #[test]
    fn save_pick_grants_new_path_for_write_and_read_back() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("exported.txt");

        let mut p = pane(0.0);
        grant_capability(&mut p, "fs.pick");
        p.set_picker_service(scripted_picker(vec![
            crate::host::services::FilePickOutcome::Picked(vec![target.clone()]),
        ]));
        p.exec(open_picker("save-1", WitFilePickerMode::Save, false), 0);
        collect_picker_events(&mut p, 1);

        let delivered = match pop_event(&mut p) {
            InputEvent::FilePicked(event) => event.paths[0].clone(),
            other => panic!("expected file-picked, got {other:?}"),
        };
        p.exec(file_write(&delivered, b"exported"), 0);
        assert!(matches!(
            pop_event(&mut p),
            InputEvent::FileWriteResult(Ok(()))
        ));
        p.exec(file_read(&delivered), 0);
        match pop_event(&mut p) {
            InputEvent::FileReadResult(Ok(bytes)) => assert_eq!(bytes, b"exported"),
            other => panic!("expected successful file-read-result, got {other:?}"),
        }
    }

    /// A folder pick grants the subtree: files under the picked directory are
    /// readable and writable through absolute paths.
    #[test]
    fn folder_pick_grants_subtree_access() {
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("nested");
        std::fs::create_dir(&nested).expect("mkdir");
        std::fs::write(nested.join("inner.txt"), b"inner").expect("seed");

        let mut p = pane(0.0);
        grant_capability(&mut p, "fs.pick");
        p.set_picker_service(scripted_picker(vec![
            crate::host::services::FilePickOutcome::Picked(vec![dir.path().to_path_buf()]),
        ]));
        p.exec(open_picker("folder-1", WitFilePickerMode::Folder, false), 0);
        collect_picker_events(&mut p, 1);

        let root = match pop_event(&mut p) {
            InputEvent::FilePicked(event) => std::path::PathBuf::from(&event.paths[0]),
            other => panic!("expected file-picked, got {other:?}"),
        };
        let inner = root.join("nested/inner.txt");
        p.exec(file_read(&inner.to_string_lossy()), 0);
        match pop_event(&mut p) {
            InputEvent::FileReadResult(Ok(bytes)) => assert_eq!(bytes, b"inner"),
            other => panic!("expected successful file-read-result, got {other:?}"),
        }
        let created = root.join("nested/created.txt");
        p.exec(file_write(&created.to_string_lossy(), b"created"), 0);
        assert!(matches!(
            pop_event(&mut p),
            InputEvent::FileWriteResult(Ok(()))
        ));
    }

    /// A save-mode (file) pick grants exactly that file — its path can never be
    /// turned into a writable directory tree, even though directory creation is
    /// now allowed under folder grants.
    #[test]
    fn save_file_grant_cannot_be_written_as_a_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("export.wav"); // does not exist yet

        let mut p = pane(0.0);
        grant_capability(&mut p, "fs.pick");
        p.set_picker_service(scripted_picker(vec![
            crate::host::services::FilePickOutcome::Picked(vec![file.clone()]),
        ]));
        p.exec(open_picker("save-1", WitFilePickerMode::Save, false), 0);
        collect_picker_events(&mut p, 1);
        let granted = match pop_event(&mut p) {
            InputEvent::FilePicked(event) => std::path::PathBuf::from(&event.paths[0]),
            other => panic!("expected file-picked, got {other:?}"),
        };

        // Writing under the granted file must fail: the file grant is not a
        // directory, so its path is never created as one.
        let child = granted.join("evil.txt");
        p.exec(file_write(&child.to_string_lossy(), b"x"), 0);
        assert!(matches!(
            pop_event(&mut p),
            InputEvent::FileWriteResult(Err(_))
        ));

        // The exact granted file still writes.
        p.exec(file_write(&granted.to_string_lossy(), b"wav"), 0);
        assert!(matches!(
            pop_event(&mut p),
            InputEvent::FileWriteResult(Ok(()))
        ));
    }

    /// Cancelling is first-class: the guest gets `file-pick-cancelled`, no
    /// grant exists afterwards, and no request state lingers.
    #[test]
    fn cancelled_pick_reaches_guest_without_grants() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("never-granted.txt");
        std::fs::write(&path, b"never").expect("seed");

        let mut p = pane(0.0);
        grant_capability(&mut p, "fs.pick");
        p.set_picker_service(scripted_picker(vec![
            crate::host::services::FilePickOutcome::Cancelled,
        ]));
        p.exec(open_picker("pick-3", WitFilePickerMode::Open, false), 0);
        collect_picker_events(&mut p, 1);

        match pop_event(&mut p) {
            InputEvent::FilePickCancelled(request_id) => assert_eq!(request_id, "pick-3"),
            other => panic!("expected file-pick-cancelled, got {other:?}"),
        }
        p.exec(file_read(&path.to_string_lossy()), 0);
        assert!(matches!(
            pop_event(&mut p),
            InputEvent::FileReadResult(Err(_))
        ));
        p.collect_picker_results();
        assert!(p.queue.is_empty(), "no dangling picker state after cancel");
    }

    // init runs startup effects: the first get-system-stats resolves through
    // the stats source and the view shows the CPU percentage.
    #[test]
    fn init_resolves_first_stats() -> wasmtime::Result<()> {
        let mut p = pane(42.0);
        p.init(&StateSnapshot { entries: vec![] }, (400.0, 300.0), 0, &[])?;
        assert!(cpu_text(&p.view()?).contains("42.0%"));
        Ok(())
    }

    // The repeating poll timer (id 1, 2000ms) fires once elapsed time passes
    // its deadline, requesting fresh stats that reflect the updated source.
    #[test]
    fn poll_timer_refreshes_stats() -> wasmtime::Result<()> {
        let mut p = pane(10.0);
        p.init(&StateSnapshot { entries: vec![] }, (400.0, 300.0), 0, &[])?;
        assert!(cpu_text(&p.view()?).contains("10.0%"));

        p.stats = Box::new(FakeStats { cpu: 88.0 });
        p.tick(2_500)?; // past the 2000ms poll deadline
        assert!(cpu_text(&p.view()?).contains("88.0%"));
        Ok(())
    }

    // 'q' maps to a close-self effect, surfaced to the pane as wants_close.
    #[test]
    fn q_closes() -> wasmtime::Result<()> {
        let mut p = pane(5.0);
        p.init(&StateSnapshot { entries: vec![] }, (400.0, 300.0), 0, &[])?;
        assert!(!p.wants_close());
        p.push_input(key("q"));
        p.tick(10)?;
        assert!(p.wants_close());
        Ok(())
    }

    // '=' x3 raises the poll interval to 5000ms; the next fire only occurs at
    // the new deadline, not the old 2000ms one.
    #[test]
    fn equals_raises_poll_interval() -> wasmtime::Result<()> {
        let mut p = pane(7.0);
        p.init(&StateSnapshot { entries: vec![] }, (400.0, 300.0), 0, &[])?;
        for _ in 0..3 {
            p.push_input(key("="));
        }
        p.tick(100)?;
        // a tick at 3000ms would have fired the old 2000ms timer; with a 5000ms
        // interval there is exactly one pending timer at next_fire 100+5000.
        assert_eq!(p.timers.len(), 1);
        assert_eq!(p.timers[0].delay_ms, 5000);
        assert_eq!(p.timers[0].next_fire_ms, 5100);
        Ok(())
    }

    #[test]
    fn file_read_returns_bytes_inside_scope() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("hello.txt");
        std::fs::write(&path, b"hello wasm").expect("seed");

        let mut p = pane(0.0);
        p.grant_fs_read_root(dir.path());
        p.exec(file_read("hello.txt"), 0);

        match pop_event(&mut p) {
            InputEvent::FileReadResult(Ok(bytes)) => assert_eq!(bytes, b"hello wasm"),
            other => panic!("expected successful file-read-result, got {other:?}"),
        }
    }

    #[test]
    fn file_read_outside_scope_returns_error() {
        let root = tempfile::tempdir().expect("root tempdir");
        let outside = tempfile::tempdir().expect("outside tempdir");
        let outside_path = outside.path().join("secret.txt");
        std::fs::write(&outside_path, b"nope").expect("seed outside");

        let mut p = pane(0.0);
        p.grant_fs_read_root(root.path());
        p.exec(file_read(&outside_path.to_string_lossy()), 0);

        match pop_event(&mut p) {
            InputEvent::FileReadResult(Err(msg)) => {
                assert!(
                    msg.contains("outside granted scope"),
                    "unexpected error: {msg}"
                );
            }
            other => panic!("expected denied file-read-result, got {other:?}"),
        }
    }

    #[test]
    fn file_write_round_trips_through_scope() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut p = pane(0.0);
        p.grant_fs_write_root(dir.path());
        p.exec(file_write("out.txt", b"written"), 0);

        match pop_event(&mut p) {
            InputEvent::FileWriteResult(Ok(())) => {}
            other => panic!("expected successful file-write-result, got {other:?}"),
        }
        assert_eq!(
            std::fs::read(dir.path().join("out.txt")).expect("read written file"),
            b"written"
        );
    }

    #[test]
    fn file_write_then_read_round_trips_binary_exact() {
        // Every byte value, including invalid UTF-8 and NULs (stint 0509).
        let payload: Vec<u8> = (0..=255u8).cycle().take(4096).collect();
        let dir = tempfile::tempdir().expect("tempdir");
        let mut p = pane(0.0);
        p.grant_fs_write_root(dir.path());
        p.grant_fs_read_root(dir.path());

        p.exec(file_write("clip.bin", &payload), 0);
        match pop_event(&mut p) {
            InputEvent::FileWriteResult(Ok(())) => {}
            other => panic!("expected successful file-write-result, got {other:?}"),
        }

        p.exec(file_read("clip.bin"), 0);
        match pop_event(&mut p) {
            InputEvent::FileReadResult(Ok(bytes)) => assert_eq!(bytes, payload),
            other => panic!("expected successful file-read-result, got {other:?}"),
        }
    }

    #[test]
    fn file_write_over_size_limit_returns_named_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut p = pane(0.0);
        p.grant_fs_write_root(dir.path());
        p.exec(
            Effect::FileWrite(FileWriteEffect {
                path: "big.bin".to_string(),
                content: vec![0u8; crate::host::MAX_FILE_IO_BYTES + 1],
            }),
            0,
        );
        match pop_event(&mut p) {
            InputEvent::FileWriteResult(Err(msg)) => {
                assert!(
                    msg.contains(&crate::host::MAX_FILE_IO_BYTES.to_string()),
                    "error must name the limit: {msg}"
                );
                assert!(
                    !dir.path().join("big.bin").exists(),
                    "oversize write must not touch disk"
                );
            }
            other => panic!("expected denied file-write-result, got {other:?}"),
        }
    }

    #[test]
    fn http_fetch_round_trips_via_net_service() {
        let mut p = pane(0.0);
        p.grant_net_host("api.test");
        p.set_net_service(Arc::new(FakeNet));
        p.exec(http_fetch("https://api.test/status"), 0);

        for _ in 0..20 {
            p.collect_http_results();
            if !p.queue.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        match pop_event(&mut p) {
            InputEvent::HttpResponse(resp) => {
                assert_eq!(resp.status, 201);
                assert_eq!(
                    resp.body,
                    b"GET https://api.test/status accept=text/plain body="
                );
                assert_eq!(
                    resp.headers,
                    vec![("content-type".to_string(), "text/plain".to_string())]
                );
            }
            other => panic!("expected http-response, got {other:?}"),
        }
    }

    #[test]
    fn http_fetch_denied_host_returns_403_response() {
        let mut p = pane(0.0);
        p.set_net_service(Arc::new(FakeNet));
        p.exec(http_fetch("https://api.test/status"), 0);

        match pop_event(&mut p) {
            InputEvent::HttpResponse(resp) => {
                assert_eq!(resp.status, 403);
                assert!(String::from_utf8_lossy(&resp.body).contains("not granted"));
            }
            other => panic!("expected denied http-response, got {other:?}"),
        }
    }

    #[test]
    fn request_capability_grant_and_deny_queue_guest_events() {
        let mut p = pane(0.0);
        let granted_cap = "unknown:feature";
        p.exec(request_capability(granted_cap), 0);
        assert!(p.has_pending_capability_prompt());
        p.decide_next_capability_prompt(true);
        match pop_event(&mut p) {
            InputEvent::CapabilityGranted(cap) => assert_eq!(cap, granted_cap),
            other => panic!("expected capability-granted, got {other:?}"),
        }

        p.exec(request_capability(granted_cap), 0);
        match pop_event(&mut p) {
            InputEvent::CapabilityGranted(cap) => assert_eq!(cap, granted_cap),
            other => panic!("expected session grant auto-answer, got {other:?}"),
        }

        let denied_cap = "fs:read:/nope";
        p.exec(request_capability(denied_cap), 0);
        assert!(p.has_pending_capability_prompt());
        p.decide_next_capability_prompt(false);
        match pop_event(&mut p) {
            InputEvent::CapabilityDenied(cap) => assert_eq!(cap, denied_cap),
            other => panic!("expected capability-denied, got {other:?}"),
        }

        p.exec(request_capability(denied_cap), 0);
        match pop_event(&mut p) {
            InputEvent::CapabilityDenied(cap) => assert_eq!(cap, denied_cap),
            other => panic!("expected session deny auto-answer, got {other:?}"),
        }
    }

    // ── Host-owned surface pacing (stint 0552) ──────────────────────────────
    //
    // sysmon declares a repeating poll timer, which is exactly the shape a
    // surface app uses to declare its cadence, so these drive the real pacing
    // path without needing a GPU surface allocated.

    fn live_pane() -> LiveWasmPane {
        let mut live = LiveWasmPane::new(
            pane(0.0),
            "sysmon",
            StateSnapshot { entries: vec![] },
            Vec::new(),
        );
        live.inner
            .init(&StateSnapshot { entries: vec![] }, (400.0, 300.0), 0, &[])
            .expect("init");
        live
    }

    #[test]
    fn declared_frame_interval_reports_the_apps_repeating_cadence() {
        let live = live_pane();
        let declared = live
            .inner
            .declared_frame_interval()
            .expect("app declared a repeating cadence");
        assert_eq!(
            declared,
            Duration::from_millis(live.inner.timers[0].delay_ms as u64)
        );
        assert_ne!(
            declared,
            live.clock.target_interval(),
            "fixture must declare something other than the constructed 60 Hz \
             for this suite to prove anything"
        );
    }

    #[test]
    fn surface_step_retargets_the_clock_at_the_declared_rate() {
        let mut live = live_pane();
        let declared = live.inner.declared_frame_interval().expect("declared");
        live.step_surface(Instant::now(), 0).expect("step");
        assert_eq!(
            live.clock.target_interval(),
            declared.clamp(
                super::super::wasm_frame::MIN_TARGET_INTERVAL,
                super::super::wasm_frame::MAX_TARGET_INTERVAL,
            )
        );
    }

    #[test]
    fn surface_steps_drive_the_guest_clock_by_whole_fixed_steps() {
        let mut live = live_pane();
        let t0 = Instant::now();
        // The baseline step lands ON the arming instant, not one interval past
        // it — otherwise every surface app fires its frame timer early on the
        // first paint after init.
        let (baseline, _) = live.step_surface(t0, 7_000).expect("baseline");
        assert_eq!(baseline, 7_000);

        // Three whole intervals plus a partial: three steps, remainder banked.
        let dt = live.clock.target_interval();
        let (sim, step) = live
            .step_surface(t0 + dt * 3 + dt / 2, 7_000)
            .expect("step");
        assert_eq!(step.steps, 3);
        assert_eq!(step.dropped, 0);
        assert_eq!(sim, 7_000 + (dt * 3).as_millis() as u64);
    }

    #[test]
    fn surface_guest_clock_keeps_sub_millisecond_step_remainder() {
        let mut live = live_pane();
        // No declared cadence: the host default is 16.666… ms, which truncates
        // to 16 ms per step and would bleed ~40 ms of guest time per second.
        live.inner.timers.clear();
        let t0 = Instant::now();
        let dt = live.clock.target_interval();
        let (baseline, _) = live.step_surface(t0, 0).expect("baseline");
        for i in 1..=60 {
            live.step_surface(t0 + dt * i, 0).expect("step");
        }
        let elapsed = live.guest_now_ms() - baseline;
        assert!(
            (999..=1001).contains(&elapsed),
            "60 steps of the default cadence must be ~1s of guest time, got {elapsed}ms"
        );
    }

    #[test]
    fn surface_catch_up_is_bounded_and_the_remainder_is_dropped_not_deferred() {
        let mut live = live_pane();
        let t0 = Instant::now();
        let (after_baseline, _) = live.step_surface(t0, 0).expect("baseline");
        let declared = live.clock.target_interval();
        let dt = declared.as_millis() as u64;

        // A 20-interval stall: simulate up to the clock's bound, drop the rest.
        let (sim, step) = live.step_surface(t0 + declared * 20, 0).expect("stall");
        assert!(
            step.steps > 0 && step.steps < 20,
            "catch-up must be bounded, ran {} steps",
            step.steps
        );
        assert_eq!(step.steps + step.dropped, 20, "no owed step goes missing");
        assert_eq!(live.telemetry.dropped(), step.dropped as u64);
        // Simulation was skipped, but the guest clock still tracks wall time —
        // it must not strand a whole stall behind.
        assert_eq!(sim, after_baseline + dt * 20);

        // Dropped steps are never repaid: the next on-cadence repaint grants
        // exactly one, with no backlog.
        let dropped_so_far = live.telemetry.dropped();
        let (next, step) = live
            .step_surface(t0 + declared * 20 + declared, 0)
            .expect("next");
        assert_eq!(step.steps, 1);
        assert_eq!(step.dropped, 0);
        assert_eq!(next, sim + dt);
        assert_eq!(live.telemetry.dropped(), dropped_so_far);
    }

    #[test]
    fn guest_timestamps_follow_the_surface_clock_once_it_starts_stepping() {
        let mut live = live_pane();
        assert!(
            live.sim_ns.is_none(),
            "a pane that has not stepped a surface runs on wall time"
        );
        let (sim, _) = live.step_surface(Instant::now(), 4_000).expect("step");
        assert_eq!(live.guest_now_ms(), sim);
    }

    #[test]
    fn surface_repaint_delay_includes_the_egui_predicted_frame() {
        let mut live = live_pane();
        live.step_surface(Instant::now(), 0).expect("step");
        let predicted = Duration::from_millis(8);
        let now = Instant::now();
        assert_eq!(
            live.surface_repaint_delay(now, predicted),
            live.clock.target_interval() + predicted
        );
    }

    #[test]
    fn live_wasm_pane_reports_pending_capability_prompt_for_focus() {
        let mut live = LiveWasmPane::new(
            pane(0.0),
            "wasm-test",
            StateSnapshot { entries: vec![] },
            Vec::new(),
        );
        assert!(!live.has_pending_capability_prompt());
        live.inner.exec(request_capability("fs:read:/tmp"), 0);
        assert!(live.has_pending_capability_prompt());
        live.inner.decide_next_capability_prompt(false);
        assert!(!live.has_pending_capability_prompt());
    }

    #[test]
    fn fs_read_effect_is_blocked_until_scoped_capability_grant() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("hello.txt"), b"hello wasm").expect("seed");
        let mut p = pane(0.0);

        p.exec(file_read("hello.txt"), 0);
        match pop_event(&mut p) {
            InputEvent::FileReadResult(Err(msg)) => {
                assert!(msg.contains("no filesystem scope granted"));
            }
            other => panic!("expected denied file-read-result, got {other:?}"),
        }

        let capability = format!("fs:read:{}", dir.path().display());
        grant_capability(&mut p, &capability);
        p.exec(file_read("hello.txt"), 0);
        match pop_event(&mut p) {
            InputEvent::FileReadResult(Ok(bytes)) => assert_eq!(bytes, b"hello wasm"),
            other => panic!("expected successful file-read-result, got {other:?}"),
        }
    }

    #[test]
    fn fs_write_effect_is_blocked_until_scoped_capability_grant() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut p = pane(0.0);

        p.exec(file_write("out.txt", b"written"), 0);
        match pop_event(&mut p) {
            InputEvent::FileWriteResult(Err(msg)) => {
                assert!(msg.contains("no filesystem scope granted"));
            }
            other => panic!("expected denied file-write-result, got {other:?}"),
        }

        let capability = format!("fs:write:{}", dir.path().display());
        grant_capability(&mut p, &capability);
        p.exec(file_write("out.txt", b"written"), 0);
        match pop_event(&mut p) {
            InputEvent::FileWriteResult(Ok(())) => {}
            other => panic!("expected successful file-write-result, got {other:?}"),
        }
        assert_eq!(
            std::fs::read(dir.path().join("out.txt")).expect("read written file"),
            b"written"
        );
    }

    #[test]
    fn http_fetch_effect_is_blocked_until_scoped_capability_grant() {
        let mut p = pane(0.0);
        p.set_net_service(Arc::new(FakeNet));

        p.exec(http_fetch("https://api.test/status"), 0);
        match pop_event(&mut p) {
            InputEvent::HttpResponse(resp) => assert_eq!(resp.status, 403),
            other => panic!("expected denied http-response, got {other:?}"),
        }

        grant_capability(&mut p, "net:fetch:api.test");
        p.exec(http_fetch("https://api.test/status"), 0);

        for _ in 0..20 {
            p.collect_http_results();
            if !p.queue.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        match pop_event(&mut p) {
            InputEvent::HttpResponse(resp) => assert_eq!(resp.status, 201),
            other => panic!("expected allowed http-response, got {other:?}"),
        }
    }

    #[test]
    fn ai_query_denied_does_not_call_broker() {
        let mut p = pane(0.0);
        let calls = Arc::new(AtomicUsize::new(0));
        p.set_ai_broker(Arc::new(FakeAiBroker {
            calls: Arc::clone(&calls),
        }));
        p.exec(request_capability("ai.query"), 0);
        p.decide_next_capability_prompt(false);
        match pop_event(&mut p) {
            InputEvent::CapabilityDenied(capability) => assert_eq!(capability, "ai.query"),
            other => panic!("expected capability-denied, got {other:?}"),
        }

        p.exec(ai_query("q-denied"), 0);
        match pop_event(&mut p) {
            InputEvent::AiResponse(resp) => {
                assert_eq!(resp.request_id, "q-denied");
                assert!(resp.content.is_none());
                assert!(resp.error.as_deref().unwrap_or_default().contains("denied"));
            }
            other => panic!("expected denied ai-response, got {other:?}"),
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn ai_query_granted_streams_and_returns_final_response() {
        let mut p = pane(0.0);
        let calls = Arc::new(AtomicUsize::new(0));
        p.set_ai_broker(Arc::new(FakeAiBroker {
            calls: Arc::clone(&calls),
        }));
        grant_capability(&mut p, "ai.query");

        p.exec(ai_query("q-ok"), 0);
        collect_async_events(&mut p, 3);

        match pop_event(&mut p) {
            InputEvent::AiStreamChunk(chunk) => {
                assert_eq!(chunk.request_id, "q-ok");
                assert_eq!(chunk.delta, "stream ");
                assert!(!chunk.done);
            }
            other => panic!("expected ai-stream-chunk, got {other:?}"),
        }
        match pop_event(&mut p) {
            InputEvent::AiStreamChunk(chunk) => {
                assert_eq!(chunk.request_id, "q-ok");
                assert!(chunk.done);
            }
            other => panic!("expected final ai-stream-chunk, got {other:?}"),
        }
        match pop_event(&mut p) {
            InputEvent::AiResponse(resp) => {
                assert_eq!(resp.request_id, "q-ok");
                assert_eq!(resp.content.as_deref(), Some("stream final"));
                assert_eq!(resp.tokens_in, 3);
                assert_eq!(resp.tokens_out, 4);
                assert!(resp.error.is_none());
            }
            other => panic!("expected final ai-response, got {other:?}"),
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn app_events_declare_and_emit_into_timeline() {
        let mut p = pane(0.0);
        p.set_pane_id(44);
        let timeline = Arc::new(Mutex::new(AppTimeline::default()));
        p.set_app_timeline(Arc::clone(&timeline));

        p.exec(declare_stream("move.played"), 0);
        match pop_event(&mut p) {
            InputEvent::DeclareEventStreamsResult(Ok(names)) => {
                assert_eq!(names, vec!["move.played".to_string()]);
            }
            other => panic!("expected declare-event-streams result, got {other:?}"),
        }

        p.exec(emit_event("move.played"), 0);
        match pop_event(&mut p) {
            InputEvent::EmitEventResult(Ok(event_id)) => assert_eq!(event_id, 1),
            other => panic!("expected emit-event result, got {other:?}"),
        }
        let timeline = timeline.lock().unwrap();
        assert_eq!(timeline.events().len(), 1);
        assert_eq!(timeline.events()[0].event, "move.played");
        assert_eq!(timeline.events()[0].pane_id, 44);
        assert_eq!(timeline.events()[0].resource_id, "game-1");
    }

    #[test]
    fn app_event_emit_without_declaration_returns_error() {
        let mut p = pane(0.0);
        let timeline = Arc::new(Mutex::new(AppTimeline::default()));
        p.set_app_timeline(Arc::clone(&timeline));

        p.exec(emit_event("move.played"), 0);
        match pop_event(&mut p) {
            InputEvent::EmitEventResult(Err(msg)) => {
                assert!(msg.contains("not a declared event stream"));
            }
            other => panic!("expected emit-event error, got {other:?}"),
        }
        assert!(timeline.lock().unwrap().events().is_empty());
    }

    #[test]
    fn subscribe_and_unsubscribe_effects_cross_the_host_boundary() {
        let mut p = pane(0.0);
        p.exec(
            Effect::SubscribeEventStreams(SubscribeEventStreamsEffect {
                request_id: "subscribe-1".to_string(),
                app_id: "python-notes".to_string(),
                event_names: vec!["note.saved".to_string()],
                payload_mode: "full".to_string(),
                trigger_mode: "conversation".to_string(),
                resource_id: Some("note-1".to_string()),
            }),
            0,
        );
        assert_eq!(
            p.take_host_effects(),
            vec![WasmHostEffect::SubscribeEvents {
                request_id: "subscribe-1".to_string(),
                app_id: "python-notes".to_string(),
                event_names: vec!["note.saved".to_string()],
                payload_mode: crate::app_protocol::PayloadMode::Full,
                trigger_mode: crate::app_protocol::TriggerMode::Conversation,
                resource_id: Some("note-1".to_string()),
            }]
        );

        p.exec(
            Effect::UnsubscribeEventStreams(UnsubscribeEventStreamsEffect {
                request_id: "unsubscribe-1".to_string(),
                subscription_id: "sub-1".to_string(),
            }),
            0,
        );
        assert_eq!(
            p.take_host_effects(),
            vec![WasmHostEffect::UnsubscribeEvents {
                request_id: "unsubscribe-1".to_string(),
                subscription_id: "sub-1".to_string(),
            }]
        );
    }

    #[test]
    fn subscribed_host_event_enters_wasm_update_queue() {
        let mut live = LiveWasmPane::new(
            pane(0.0),
            "sysmon",
            StateSnapshot { entries: vec![] },
            vec![],
        );
        live.queue_outbound_event(crate::app_protocol::PlexiEvent::AppEvent {
            subscription_id: "sub-1".to_string(),
            app_id: "python-notes".to_string(),
            event: "note.saved".to_string(),
            event_id: 9,
            resource_id: "note-1".to_string(),
            trigger_mode: crate::app_protocol::TriggerMode::Conversation,
            summary: Some("Saved note".to_string()),
            payload: Some(serde_json::json!({"title": "Hello"})),
            state_ref: None,
            created_at: "2026-07-13T00:00:00Z".to_string(),
        });

        let event = live.inner.queue.pop_front().expect("queued AppEvent");
        let InputEvent::AppEvent(event) = event else {
            panic!("expected AppEvent, got {event:?}");
        };
        assert_eq!(event.subscription_id, "sub-1");
        assert_eq!(event.app_id, "python-notes");
        assert_eq!(event.event, "note.saved");
        assert_eq!(event.payload_json.as_deref(), Some(r#"{"title":"Hello"}"#));
    }

    #[test]
    fn tool_declarations_require_both_json_schemas() {
        let mut p = pane(0.0);
        p.exec(
            Effect::DeclareTools(DeclareToolsEffect {
                tools: vec![
                    super::super::wasm_app::bindings::plexi::platform::types::ToolDecl {
                        name: "notes.search".to_string(),
                        description: "Search notes".to_string(),
                        input_schema_json: r#"{"type":"object"}"#.to_string(),
                        output_schema_json: r#"{"type":"array"}"#.to_string(),
                        timeout_ms: Some(2_000),
                        read_only: true,
                    },
                ],
            }),
            0,
        );
        let effects = p.take_host_effects();
        let WasmHostEffect::DeclareTools { tools } = &effects[0] else {
            panic!("expected DeclareTools, got {:?}", effects[0]);
        };
        assert_eq!(tools[0].name, "notes.search");
        assert_eq!(tools[0].input_schema["type"], "object");
        assert_eq!(tools[0].output_schema["type"], "array");
        assert!(tools[0].read_only);

        p.exec(
            Effect::DeclareTools(DeclareToolsEffect {
                tools: vec![
                    super::super::wasm_app::bindings::plexi::platform::types::ToolDecl {
                        name: "bad".to_string(),
                        description: "Bad schema".to_string(),
                        input_schema_json: "not-json".to_string(),
                        output_schema_json: r#"{"type":"object"}"#.to_string(),
                        timeout_ms: None,
                        read_only: false,
                    },
                ],
            }),
            0,
        );
        assert!(matches!(
            pop_event(&mut p),
            InputEvent::DeclareToolsResult(Err(error)) if error.contains("invalid input schema")
        ));
    }

    // Lane B: typed-node interactions collected by the renderer are fed back
    // into the guest update loop, so a button click changes the next view.
    #[test]
    fn ui_button_click_updates_guest_view() -> wasmtime::Result<()> {
        use egui_kittest::kittest::Queryable;

        let mut p = counter_pane();
        p.init(&StateSnapshot { entries: vec![] }, (400.0, 300.0), 0, &[])?;
        assert!(tree_text(&p.view()?).contains("Count: 0"));

        let colors = Colors::from_config(&crate::config::ThemeConfig::default());
        let mut tree_seq: u64 = 0;
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::Vec2::new(400.0, 300.0))
            .build_ui_state(
                move |ui, pane| {
                    let tree = pane.view().expect("view");
                    // This runtime re-evaluates `view()` every frame, so every
                    // painted tree is a new generation — mirror the live pane.
                    tree_seq += 1;
                    let result = crate::host::wasm_render::render_ui_tree_with_surface(
                        ui, &tree, &colors, None, None, tree_seq,
                    );
                    pane.apply_render_result(result, 0)
                        .expect("apply interactions");
                },
                p,
            );

        harness.get_by_label("Increment").click();
        harness.run();

        let text = tree_text(&harness.state_mut().view()?);
        assert!(text.contains("Count: 1"), "guest view after click:\n{text}");
        Ok(())
    }

    #[test]
    fn host_semantic_action_updates_guest_view() -> wasmtime::Result<()> {
        let mut pane = counter_pane();
        pane.init(&StateSnapshot { entries: vec![] }, (400.0, 300.0), 0, &[])?;
        assert!(tree_text(&pane.view()?).contains("Count: 0"));

        pane.dispatch_ui_action("increment", 1)?;

        let text = tree_text(&pane.view()?);
        assert!(
            text.contains("Count: 1"),
            "guest view after host action:\n{text}"
        );
        Ok(())
    }

    #[test]
    fn wasm_view_normalizes_committed_semantics() -> wasmtime::Result<()> {
        let mut p = counter_pane();
        p.init(&StateSnapshot { entries: vec![] }, (400.0, 300.0), 0, &[])?;

        let state = crate::host::pane::SemanticPaneState::from_wasm_tree(&p.view()?);

        assert_eq!(state.runtime_kind, "wasm");
        assert!(state
            .nodes
            .iter()
            .any(|node| node.label.as_deref() == Some("Increment")));
        Ok(())
    }

    // ── G7: surface-node lifecycle + input (Pong) ────────────────────────

    fn pong_fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wasm-fixtures/pong.wasm")
    }

    fn pong_pane() -> WasmPane {
        let app = WasmApp::load_ephemeral_run("pong", &pong_fixture(), StateStore::ephemeral())
            .expect("load pong (gpu device required)");
        WasmPane::new(app, Box::new(FakeStats { cpu: 0.0 }))
    }

    // Bright (white paddle) pixels in the left paddle column [21,28].
    fn left_paddle_centroid_y(img: &image::RgbaImage) -> f32 {
        let (mut sum, mut count) = (0.0f32, 0u32);
        for y in 0..img.height() {
            for x in 21..29u32 {
                let p = img.get_pixel(x, y);
                if p[0] > 150 && p[1] > 150 && p[2] > 180 {
                    sum += y as f32;
                    count += 1;
                }
            }
        }
        assert!(count > 0, "no white paddle pixels found in left column");
        sum / count as f32
    }

    // G7: the guest declares a surface-node; the host allocates the GPU texture
    // and delivers surface-ready, the guest sets up its pipeline and renders the
    // game into it. Pressing 'w' and advancing the tick timer moves the left
    // paddle up — observable as the white paddle's centroid rising in the
    // read-back surface. Proves surface lifecycle + input + real GPU rendering.
    #[test]
    fn g7_surface_lifecycle_and_input() -> wasmtime::Result<()> {
        let mut p = pong_pane();
        p.init(&StateSnapshot { entries: vec![] }, (480.0, 360.0), 0, &[])?;

        // Surface allocated and the guest rendered game objects into it.
        let (w, h) = p.surface_size().expect("surface allocated after init");
        assert_eq!(
            (w, h),
            (480, 320),
            "surface sized to the guest's surface-node"
        );
        let before = p.read_surface().expect("surface readback");
        let bright = before
            .pixels()
            .filter(|px| px[0] as u32 + px[1] as u32 + px[2] as u32 > 480)
            .count();
        assert!(
            bright > 100,
            "rendered surface shows game objects (paddles/ball)"
        );

        let cy_before = left_paddle_centroid_y(&before);

        // Hold 'w' (left paddle up) and fire 60 tick frames.
        p.push_input(key("w"));
        let mut t = 0u64;
        for _ in 0..61 {
            t += 16; // TICK_MS
            p.tick(t)?;
        }
        let after = p.read_surface().expect("surface readback");
        let cy_after = left_paddle_centroid_y(&after);

        assert!(
            cy_after < cy_before - 20.0,
            "left paddle moved up after 60 W frames: {cy_before} -> {cy_after}"
        );
        Ok(())
    }

    // Stint 0527: on a HiDPI display the surface allocates at physical
    // resolution (logical × ppp) so composition is 1:1 with screen pixels,
    // and a display-scale change frees + reallocates the surface with a
    // fresh surface-ready so the guest re-targets the new texture.
    #[test]
    fn surface_allocates_physical_resolution_and_reallocates_on_ppp_change() -> wasmtime::Result<()>
    {
        let mut p = pong_pane();
        p.set_pixels_per_point(2.0);
        p.init(&StateSnapshot { entries: vec![] }, (480.0, 360.0), 0, &[])?;

        assert_eq!(
            p.surface_size(),
            Some((960, 640)),
            "surface allocated at logical (480x320) × ppp 2.0"
        );
        let img = p.read_surface().expect("surface readback");
        let bright = img
            .pixels()
            .filter(|px| px[0] as u32 + px[1] as u32 + px[2] as u32 > 480)
            .count();
        assert!(
            bright > 100,
            "guest rendered game objects into the physical-resolution texture"
        );

        // Display moves to a 1.0-scale screen: the surface is freed and the
        // next tick reallocates at the new resolution.
        p.set_pixels_per_point(1.0);
        assert!(
            p.surface_size().is_none(),
            "surface freed when the display scale changes"
        );
        p.tick(16)?;
        assert_eq!(
            p.surface_size(),
            Some((480, 320)),
            "surface reallocated at the new display scale"
        );
        Ok(())
    }

    // ── Perf gate: presentation does zero readbacks ───────────────────────────
    //
    // The whole point of the zero-copy compositor is that steady presentation
    // never reads the surface back to the CPU. This gate locks that invariant
    // deterministically (no wall-clock timing, which was historically flaky):
    //   1. the zero-copy sRGB view is obtainable (the compositing precondition),
    //   2. a long run of steady ticks reads back exactly zero times,
    //   3. the capture path is isolated — calling it reads back exactly once.
    #[test]
    fn perf_gate_presentation_reads_back_zero() -> wasmtime::Result<()> {
        let mut p = pong_pane();
        p.init(&StateSnapshot { entries: vec![] }, (480.0, 360.0), 0, &[])?;

        // Precondition for zero-copy compositing: the surface exposes an sRGB
        // view egui can sample directly.
        assert!(
            p.surface_srgb_view().is_some(),
            "surface must expose an sRGB view for zero-copy compositing"
        );

        // Drive 300 steady frames. The tick/present path must never read back.
        let base = p.surface_readbacks();
        for f in 0..300u64 {
            p.tick(f * 16)?;
        }
        assert_eq!(
            p.surface_readbacks() - base,
            0,
            "steady presentation must not read the surface back to the CPU"
        );

        // The capture path stays available and isolated: one call, one readback.
        let before = p.surface_readbacks();
        let _img = p.read_surface().expect("capture-path readback");
        assert_eq!(
            p.surface_readbacks() - before,
            1,
            "capture readback must be on-demand and isolated from the present path"
        );
        Ok(())
    }

    // ── Perf gate: dedicated-device fallback stays off the UI thread ─────────
    //
    // The async ring's whole purpose is to keep the non-shared-device fallback
    // from blocking the UI thread on `poll(Wait)`. This locks a generous
    // wall-clock ceiling on the request -> completion round trip at the pane's
    // default 480x360 size so a regression back to synchronous readback gets
    // caught before it lands.
    #[test]
    fn async_readback_arrives_with_load_aware_budget() -> wasmtime::Result<()> {
        let mut p = pong_pane();
        p.init(&StateSnapshot { entries: vec![] }, (480.0, 360.0), 0, &[])?;
        let (w, h) = p.surface_size().expect("surface allocated after init");
        let deadline =
            Instant::now() + crate::testing::load_aware_timeout(std::time::Duration::from_secs(5));

        p.request_surface_readback()
            .expect("queue async surface readback");
        let img = loop {
            if let Some(result) = p.take_surface_readback() {
                break result.expect("async surface readback succeeds");
            }
            assert!(
                Instant::now() < deadline,
                "async surface readback did not arrive by {deadline:?} at {w}x{h}"
            );
            std::thread::yield_now();
        };
        assert_eq!(img.width(), w);
        assert_eq!(img.height(), h);
        Ok(())
    }

    #[test]
    #[ignore = "perf-gate: run explicitly on a quiet machine"]
    fn perf_gate_async_readback_within_budget() -> wasmtime::Result<()> {
        let mut p = pong_pane();
        p.init(&StateSnapshot { entries: vec![] }, (480.0, 360.0), 0, &[])?;
        let (w, h) = p.surface_size().expect("surface allocated after init");

        const THRESHOLD: std::time::Duration = std::time::Duration::from_millis(50);
        let start = Instant::now();
        p.request_surface_readback()
            .expect("queue async surface readback");
        let img = loop {
            if let Some(result) = p.take_surface_readback() {
                break result.expect("async surface readback succeeds");
            }
            assert!(
                start.elapsed() < THRESHOLD,
                "async surface readback exceeded {THRESHOLD:?} budget at {w}x{h}"
            );
            std::thread::yield_now();
        };
        assert_eq!(img.width(), w);
        assert_eq!(img.height(), h);
        let elapsed = start.elapsed();
        assert!(
            elapsed < THRESHOLD,
            "async surface readback took {elapsed:?}, budget {THRESHOLD:?} at {w}x{h}"
        );
        Ok(())
    }

    // ── Key translation contract (egui → guest) ───────────────────────────────
    //
    // These lock the host's egui→guest key dialect. WASM guests match these
    // strings literally (no SDK normalization layer), so the host must emit the
    // clean short form and forward both press and release edges.

    fn key_edge(k: &str, pressed: bool) -> InputEvent {
        InputEvent::Key(KeyEvent {
            key: k.to_string(),
            modifiers: Modifiers {
                ctrl: false,
                shift: false,
                alt: false,
                meta: false,
            },
            pressed,
        })
    }

    fn egui_key(
        key: egui::Key,
        pressed: bool,
        repeat: bool,
        modifiers: egui::Modifiers,
    ) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat,
            modifiers,
        }
    }

    // Regression: arrows previously arrived as egui Debug names ("arrowup"),
    // which no guest matches. They must use the canonical short form.
    #[test]
    fn canonical_names_use_short_form() {
        assert_eq!(canonical_key_name(egui::Key::ArrowUp), "up");
        assert_eq!(canonical_key_name(egui::Key::ArrowDown), "down");
        assert_eq!(canonical_key_name(egui::Key::ArrowLeft), "left");
        assert_eq!(canonical_key_name(egui::Key::ArrowRight), "right");
        assert_eq!(canonical_key_name(egui::Key::Space), "space");
        assert_eq!(canonical_key_name(egui::Key::Enter), "enter");
        assert_eq!(canonical_key_name(egui::Key::Escape), "escape");
        assert_eq!(canonical_key_name(egui::Key::W), "w");
        assert_eq!(canonical_key_name(egui::Key::Num1), "1");
        assert_eq!(canonical_key_name(egui::Key::Minus), "-");
    }

    // Regression: handle_key only emitted pressed:true, so held-control apps
    // (games) never saw the release and the paddle stuck. Both edges translate.
    #[test]
    fn both_press_and_release_edges_translate() {
        let down = translate_key_event(&egui_key(egui::Key::W, true, false, egui::Modifiers::NONE))
            .expect("press forwarded");
        assert_eq!((down.key.as_str(), down.pressed), ("w", true));

        let up = translate_key_event(&egui_key(egui::Key::W, false, false, egui::Modifiers::NONE))
            .expect("release forwarded");
        assert_eq!((up.key.as_str(), up.pressed), ("w", false));
    }

    // Arrow press translates to a short-name key edge end to end.
    #[test]
    fn arrow_press_translates_to_short_name() {
        let ev = translate_key_event(&egui_key(
            egui::Key::ArrowRight,
            true,
            false,
            egui::Modifiers::NONE,
        ))
        .expect("arrow forwarded");
        assert_eq!((ev.key.as_str(), ev.pressed), ("right", true));
    }

    // OS auto-repeat is collapsed to clean press/release edges so discrete
    // actions fire once per physical press.
    #[test]
    fn auto_repeat_is_collapsed() {
        let repeat = translate_key_event(&egui_key(
            egui::Key::ArrowRight,
            true,
            true,
            egui::Modifiers::NONE,
        ));
        assert!(repeat.is_none(), "auto-repeat must not reach the guest");
    }

    // Cmd-chords are reserved for host shortcuts and never reach the guest.
    #[test]
    fn cmd_chords_are_reserved_for_host() {
        let cmd = translate_key_event(&egui_key(
            egui::Key::N,
            true,
            false,
            egui::Modifiers::COMMAND,
        ));
        assert!(
            cmd.is_none(),
            "Cmd-chords are host shortcuts, not guest input"
        );
    }

    /// Stint 0430: bare Escape is reserved for the host CloseApp binding
    /// (keys.rs, `BindingContext::AppActive`). A focused WASM pane must report
    /// `Passthrough` for it so `poll_actions` can fire CloseApp; before the fix
    /// `handle_key` reported `Consumed` for every key event, claiming Escape out
    /// of the frame buffer and starving the close. Normal keys stay `Consumed`.
    #[test]
    fn handle_key_reserves_bare_escape_for_host_close() {
        fn disposition(live: &mut LiveWasmPane, event: egui::Event) -> KeyDisposition {
            let ctx = egui::Context::default();
            let mut raw = egui::RawInput::default();
            raw.events.push(event);
            let mut out = KeyDisposition::Passthrough;
            let _ = ctx.run_ui(raw, |ui| {
                let ctx = ui.ctx();
                let input = crate::app::input_router::PlexiInput::take_from(ctx);
                out = live.handle_key(&input);
            });
            out
        }

        let mut live = LiveWasmPane::new(
            pane(0.0),
            "wasm-test",
            StateSnapshot { entries: vec![] },
            Vec::new(),
        );

        assert_eq!(
            disposition(
                &mut live,
                egui_key(egui::Key::Escape, true, false, egui::Modifiers::NONE)
            ),
            KeyDisposition::Passthrough,
            "bare Escape must pass through so the host CloseApp binding fires"
        );
        assert_eq!(
            disposition(
                &mut live,
                egui_key(egui::Key::W, true, false, egui::Modifiers::NONE)
            ),
            KeyDisposition::Consumed,
            "normal keys are consumed and forwarded to the WASM guest"
        );
    }

    // End-to-end: a held control moves the paddle, and releasing it stops the
    // paddle. Only possible if release edges flow through to the guest.
    #[test]
    fn pong_paddle_stops_on_release() -> wasmtime::Result<()> {
        let mut p = pong_pane();
        p.init(&StateSnapshot { entries: vec![] }, (480.0, 360.0), 0, &[])?;
        let start = left_paddle_centroid_y(&p.read_surface().expect("readback"));

        let mut t = 0u64;
        p.push_input(key_edge("w", true));
        for _ in 0..30 {
            t += 16;
            p.tick(t)?;
        }
        let held = left_paddle_centroid_y(&p.read_surface().expect("readback"));

        p.push_input(key_edge("w", false));
        for _ in 0..30 {
            t += 16;
            p.tick(t)?;
        }
        let after_release = left_paddle_centroid_y(&p.read_surface().expect("readback"));

        assert!(
            held < start - 10.0,
            "paddle moved up while W held: {start} -> {held}"
        );
        assert!(
            (after_release - held).abs() < 3.0,
            "paddle held position after W released: {held} -> {after_release}"
        );
        Ok(())
    }

    // ── daw.* connector tools (stint 0517) ───────────────────────────────

    fn daw_fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wasm-fixtures/daw-engine.wasm")
    }

    fn daw_pane() -> WasmPane {
        let app = WasmApp::load_ephemeral_run("daw", &daw_fixture(), StateStore::ephemeral())
            .expect("load daw-engine");
        WasmPane::new(app, Box::new(FakeStats { cpu: 0.0 }))
    }

    fn tool_call(call_id: &str, name: &str, input_json: &str) -> InputEvent {
        InputEvent::ToolCall(ToolCallEvent {
            call_id: call_id.to_string(),
            name: name.to_string(),
            input_json: input_json.to_string(),
            caller_id: "assistant".to_string(),
        })
    }

    /// The ToolResult host effect for `call_id`, as (output_json, error).
    fn tool_result(effects: &[WasmHostEffect], call_id: &str) -> (Option<String>, Option<String>) {
        effects
            .iter()
            .find_map(|e| match e {
                WasmHostEffect::ToolResult {
                    call_id: id,
                    output_json,
                    error,
                } if id == call_id => Some((output_json.clone(), error.clone())),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no ToolResult for {call_id} in {effects:?}"))
    }

    fn tool_output(effects: &[WasmHostEffect], call_id: &str) -> serde_json::Value {
        let (output, error) = tool_result(effects, call_id);
        assert_eq!(error, None, "{call_id} unexpectedly errored");
        serde_json::from_str(&output.expect("ok result carries output_json"))
            .expect("tool output is JSON")
    }

    // The app declares the full daw.* surface at init with truthful
    // read-only flags: exactly the four read tools may auto-grant; every
    // mutation (and both POC tools) must prompt.
    #[test]
    fn daw_app_declares_namespaced_tools_with_correct_read_only_flags() -> wasmtime::Result<()> {
        let mut p = daw_pane();
        p.init(&StateSnapshot { entries: vec![] }, (480.0, 360.0), 0, &[])?;
        let effects = p.take_host_effects();
        let tools = effects
            .iter()
            .find_map(|e| match e {
                WasmHostEffect::DeclareTools { tools } => Some(tools),
                _ => None,
            })
            .expect("init declares tools");

        let read_only: Vec<&str> = tools
            .iter()
            .filter(|t| t.read_only)
            .map(|t| t.name.as_str())
            .collect();
        assert_eq!(
            read_only,
            vec![
                "daw.project_info",
                "daw.list_tracks",
                "daw.get_track",
                "daw.transport_state"
            ]
        );
        let mutating: Vec<&str> = tools
            .iter()
            .filter(|t| !t.read_only)
            .map(|t| t.name.as_str())
            .collect();
        assert_eq!(
            mutating,
            vec![
                "daw.add_track",
                "daw.set_track_volume",
                "daw.mute_track",
                "daw.solo_track",
                "daw.set_bpm",
                "daw.add_clip",
                "daw.play",
                "daw.stop",
                "daw_mixdown",
                "daw_command"
            ]
        );
        for tool in tools {
            assert!(
                tool.input_schema.is_object() && tool.output_schema.is_object(),
                "{} schemas must be JSON objects",
                tool.name
            );
        }
        Ok(())
    }

    // The 0517 demo loop end to end: an assistant ToolCall reaches the guest,
    // mutates the model through DawCommand, returns a chainable ToolResult,
    // and the pane's semantic tree reflects the change. Errors surface the
    // model's reason and never rebind dangling ids.
    #[test]
    fn daw_tool_call_mutates_model_and_tree_reflects_it() -> wasmtime::Result<()> {
        let mut p = daw_pane();
        p.init(&StateSnapshot { entries: vec![] }, (480.0, 360.0), 0, &[])?;
        p.take_host_effects();

        // Create a MIDI track — the headline "create a MIDI track" ask.
        p.push_input(tool_call(
            "c1",
            "daw.add_track",
            r#"{"kind":"midi","name":"Bass"}"#,
        ));
        p.tick(16)?;
        let out = tool_output(&p.take_host_effects(), "c1");
        assert_eq!(out["outcome"], "applied");
        let track_id = out["track_id"].as_u64().expect("new track id");
        assert!(
            tree_text(&p.view()?).contains("Bass"),
            "tree shows the new track"
        );

        // "Reduce the volume of the bass track" — by name, then read it back
        // by id (list → set → get chaining on ids).
        p.push_input(tool_call(
            "c2",
            "daw.set_track_volume",
            r#"{"track":"bass","volume":0.25}"#,
        ));
        p.tick(32)?;
        let out = tool_output(&p.take_host_effects(), "c2");
        assert_eq!(out["outcome"], "applied");
        assert_eq!(out["track_id"], track_id);
        assert!(
            tree_text(&p.view()?).contains("vol 0.25"),
            "tree shows the new volume"
        );

        p.push_input(tool_call(
            "c3",
            "daw.get_track",
            &format!(r#"{{"track":{track_id}}}"#),
        ));
        p.tick(48)?;
        let out = tool_output(&p.take_host_effects(), "c3");
        assert_eq!(out["name"], "Bass");
        assert!((out["volume"].as_f64().unwrap() - 0.25).abs() < 1e-6);

        // A dangling id is a clean not-found, never a rebind.
        p.push_input(tool_call(
            "c4",
            "daw.set_track_volume",
            r#"{"track":999,"volume":1.0}"#,
        ));
        p.tick(64)?;
        let (output, error) = tool_result(&p.take_host_effects(), "c4");
        assert_eq!(output, None);
        let error = error.expect("dangling id errors");
        assert!(error.contains("track id 999 not found"), "{error}");
        Ok(())
    }

    // ── jukebox.* transport tools + file pick (stint 0513) ───────────────

    fn jukebox_fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wasm-fixtures/jukebox.wasm")
    }

    fn jukebox_pane() -> WasmPane {
        let app =
            WasmApp::load_ephemeral_run("jukebox", &jukebox_fixture(), StateStore::ephemeral())
                .expect("load jukebox");
        WasmPane::new(app, Box::new(FakeStats { cpu: 0.0 }))
    }

    /// The now-playing transport state is a Badge node (not Text), so
    /// `tree_text` does not see it; scan badge text directly.
    fn tree_has_badge(tree: &UiTree, needle: &str) -> bool {
        tree.nodes.iter().any(|n| match &n.data {
            UiNodeData::Badge(b) => b.text.contains(needle),
            _ => false,
        })
    }

    /// Minimal float32 (format 3) WAV bytes — the exact shape the jukebox's
    /// in-guest decoder accepts, so the pick path exercises a real decode.
    fn wav_f32_bytes(samples: &[f32], rate: u32, channels: u32) -> Vec<u8> {
        let data_len = (samples.len() * 4) as u32;
        let byte_rate = rate * channels * 4;
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_len).to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
        out.extend_from_slice(&(channels as u16).to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&byte_rate.to_le_bytes());
        out.extend_from_slice(&((channels * 4) as u16).to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_len.to_le_bytes());
        for s in samples {
            out.extend_from_slice(&s.to_le_bytes());
        }
        out
    }

    // The jukebox declares its connector surface at init with truthful
    // read-only flags: exactly the two read tools may auto-grant; every
    // transport mutation must prompt (a mislabel would bypass the prompt).
    #[test]
    fn jukebox_declares_namespaced_tools_with_correct_read_only_flags() -> wasmtime::Result<()> {
        let mut p = jukebox_pane();
        p.init(&StateSnapshot { entries: vec![] }, (480.0, 360.0), 0, &[])?;
        let effects = p.take_host_effects();
        let tools = effects
            .iter()
            .find_map(|e| match e {
                WasmHostEffect::DeclareTools { tools } => Some(tools),
                _ => None,
            })
            .expect("init declares tools");

        let read_only: Vec<&str> = tools
            .iter()
            .filter(|t| t.read_only)
            .map(|t| t.name.as_str())
            .collect();
        assert_eq!(read_only, vec!["jukebox.list_files", "jukebox.now_playing"]);
        let mutating: Vec<&str> = tools
            .iter()
            .filter(|t| !t.read_only)
            .map(|t| t.name.as_str())
            .collect();
        assert_eq!(
            mutating,
            vec![
                "jukebox.play",
                "jukebox.pause",
                "jukebox.next",
                "jukebox.set_volume"
            ]
        );
        for tool in tools {
            assert!(
                tool.input_schema.is_object() && tool.output_schema.is_object(),
                "{} schemas must be JSON objects",
                tool.name
            );
        }
        Ok(())
    }

    // The assistant drives transport end to end: each ToolCall reaches the
    // guest, mutates the pure model, returns a chainable ToolResult, and the
    // pane's semantic tree reflects the new transport state. The demo playlist
    // seeded at init makes this hermetic — no grants, no files.
    #[test]
    fn jukebox_tool_calls_drive_transport_and_tree_reflects_it() -> wasmtime::Result<()> {
        let mut p = jukebox_pane();
        p.init(&StateSnapshot { entries: vec![] }, (480.0, 360.0), 0, &[])?;
        p.take_host_effects();

        // Three demo tracks are seeded at init.
        p.push_input(tool_call("t1", "jukebox.list_files", "{}"));
        p.tick(16)?;
        let out = tool_output(&p.take_host_effects(), "t1");
        assert_eq!(out["count"], 3);

        p.push_input(tool_call("t2", "jukebox.play", "{}"));
        p.tick(32)?;
        let out = tool_output(&p.take_host_effects(), "t2");
        assert_eq!(out["playing"], true);
        assert!(tree_has_badge(&p.view()?, "PLAYING"), "tree shows playing");

        // "Skip to the next track" — advances the index and rewinds.
        p.push_input(tool_call("t3", "jukebox.next", "{}"));
        p.tick(48)?;
        let out = tool_output(&p.take_host_effects(), "t3");
        assert_eq!(out["index"], 1);

        p.push_input(tool_call("t4", "jukebox.now_playing", "{}"));
        p.tick(64)?;
        let out = tool_output(&p.take_host_effects(), "t4");
        assert_eq!(out["index"], 1);
        assert_eq!(out["playing"], true);
        assert_eq!(out["track_count"], 3);

        // "Turn it down" — a mutating tool with an argument.
        p.push_input(tool_call("t5", "jukebox.set_volume", r#"{"volume":0.4}"#));
        p.tick(80)?;
        let out = tool_output(&p.take_host_effects(), "t5");
        assert!((out["volume"].as_f64().unwrap() - 0.4).abs() < 1e-6);

        p.push_input(tool_call("t6", "jukebox.pause", "{}"));
        p.tick(96)?;
        let out = tool_output(&p.take_host_effects(), "t6");
        assert_eq!(out["playing"], false);
        assert!(tree_has_badge(&p.view()?, "STOPPED"), "tree shows stopped");

        // A malformed mutating call is a clean error, never a silent no-op.
        p.push_input(tool_call("t7", "jukebox.set_volume", "{}"));
        p.tick(112)?;
        let (output, error) = tool_result(&p.take_host_effects(), "t7");
        assert_eq!(output, None);
        assert!(error.expect("missing volume errors").contains("volume"));
        Ok(())
    }

    // The full 0508+0509+0513 loop: `o` opens the picker, the scripted pick
    // grants + reads a real WAV, the guest decodes it in-guest, and the picked
    // track joins the playlist the assistant can then see and play.
    #[test]
    fn jukebox_pick_reads_decodes_real_wav_and_lists_it() -> wasmtime::Result<()> {
        let dir = tempfile::tempdir().expect("tempdir");
        let song = dir.path().join("my-song.wav");
        // 0.1s of 220 Hz mono at 48 kHz.
        let frames = 4_800usize;
        let samples: Vec<f32> = (0..frames)
            .map(|i| ((i as f32 / 48_000.0) * std::f32::consts::TAU * 220.0).sin() * 0.5)
            .collect();
        std::fs::write(&song, wav_f32_bytes(&samples, 48_000, 1)).expect("seed wav");

        let mut p = jukebox_pane();
        p.init(&StateSnapshot { entries: vec![] }, (480.0, 360.0), 0, &[])?;
        p.take_host_effects();

        grant_capability(&mut p, "fs.pick");
        p.set_picker_service(scripted_picker(vec![
            crate::host::services::FilePickOutcome::Picked(vec![song.clone()]),
        ]));

        // `o` requests fs.pick; the grant opens the picker; the pick grants +
        // reads the file; the guest decodes it. The picker runs on a thread,
        // so tick until the loaded track appears (or time out).
        p.push_input(key("o"));
        let mut loaded = false;
        for i in 0..100 {
            p.tick(100 + i * 16)?;
            let tree = tree_text(&p.view()?);
            if tree.contains("my-song") && !tree.contains("loading") {
                loaded = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(loaded, "picked track loaded into the playlist tree");

        // The assistant now sees the picked track loaded alongside the demos.
        p.push_input(tool_call("j1", "jukebox.list_files", "{}"));
        p.tick(2_000)?;
        let out = tool_output(&p.take_host_effects(), "j1");
        assert_eq!(out["count"], 4, "3 demo tracks + 1 picked");
        let picked = out["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "my-song")
            .expect("picked track named by file stem");
        assert_eq!(picked["state"], "loaded");
        assert!(
            picked["duration_ms"].as_u64().unwrap() > 0,
            "decoded a real duration"
        );
        Ok(())
    }

    // A cancelled pick adds nothing and never wedges the app.
    #[test]
    fn jukebox_pick_cancel_adds_no_tracks() -> wasmtime::Result<()> {
        let mut p = jukebox_pane();
        p.init(&StateSnapshot { entries: vec![] }, (480.0, 360.0), 0, &[])?;
        p.take_host_effects();

        grant_capability(&mut p, "fs.pick");
        p.set_picker_service(scripted_picker(vec![
            crate::host::services::FilePickOutcome::Cancelled,
        ]));

        p.push_input(key("o"));
        for i in 0..40 {
            p.tick(100 + i * 16)?;
            if tree_text(&p.view()?).contains("cancelled") {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        p.push_input(tool_call("c1", "jukebox.list_files", "{}"));
        p.tick(2_000)?;
        let out = tool_output(&p.take_host_effects(), "c1");
        assert_eq!(out["count"], 3, "still just the demo playlist");
        Ok(())
    }
}
