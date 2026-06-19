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
use crate::app_protocol::{
    AiMessage as ProtocolAiMessage, AppEventActor, EventStreamDecl as ProtocolEventStreamDecl,
    ModelTier, TriggerMode,
};
use crate::host::app_timeline::{AppTimeline, EmittedEvent};
use crate::host::services::{HttpResponse as HostHttpResponse, NetService, UreqNetService};
use crate::media::audio::{start_output_stream, OutputSession};
use crate::plexi_ai::broker::{AiBroker, AiBrokerRequest, LiveAiBroker};
use crate::ui::theme::Colors;

use super::wasm_app::bindings::plexi::platform::types::{
    AiQueryEffect, AiResponseEvent, AiStreamChunkEvent, DeclareEventStreamsEffect, EmitEventEffect,
    FileReadEffect, FileWriteEffect, HttpFetchEffect, HttpResponse as WitHttpResponse,
    UiActionEvent, UiValueChangeEvent,
};
use super::wasm_app::{
    Effect, InputEvent, KeyEvent, Modifiers, StateSnapshot, SurfaceEvent, SystemStats, UiNodeData,
    UiTree, WasmApp,
};
use super::wasm_render::{render_ui_tree_with_surface, RenderResult};

/// A GPU surface the host allocated for the guest's `surface-node`.
struct SurfaceState {
    handle: u64,
    width: u32,
    height: u32,
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

    fn allows_host(&self, url: &str) -> Result<(), String> {
        let parsed = Url::parse(url).map_err(|e| format!("invalid url: {e}"))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| "url has no host".to_string())?
            .to_ascii_lowercase();
        if self.net_hosts.iter().any(|allowed| allowed == &host) {
            Ok(())
        } else {
            Err(format!("net host '{host}' not granted"))
        }
    }
}

fn canonicalize_scope(path: PathBuf, require_existing: bool) -> Result<PathBuf, String> {
    if require_existing || path.exists() {
        return std::fs::canonicalize(&path)
            .map_err(|e| format!("resolve {}: {e}", path.display()));
    }
    let parent = path
        .parent()
        .ok_or_else(|| format!("scope has no parent: {}", path.display()))?;
    let parent =
        std::fs::canonicalize(parent).map_err(|e| format!("resolve {}: {e}", parent.display()))?;
    let name = path
        .file_name()
        .ok_or_else(|| format!("scope has no file name: {}", path.display()))?;
    Ok(parent.join(name))
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
    deferred_ai_queries: VecDeque<AiQueryEffect>,
    net: Arc<dyn NetService>,
    ai_broker: Arc<dyn AiBroker>,
    app_timeline: Arc<Mutex<AppTimeline>>,
    pane_id: u64,
    http_tx: Sender<InputEvent>,
    http_rx: Receiver<InputEvent>,
}

impl WasmPane {
    pub fn new(app: WasmApp, stats: Box<dyn SystemStatsSource>) -> Self {
        let (http_tx, http_rx) = mpsc::channel();
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
            deferred_ai_queries: VecDeque::new(),
            net: Arc::new(UreqNetService::new()),
            ai_broker: Arc::new(default_live_broker()),
            app_timeline: crate::host::app_timeline::global(),
            pane_id: 0,
            http_tx,
            http_rx,
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

    /// Run the guest's `init`, execute its startup effects, and converge the
    /// resulting input queue (e.g. the first stats request).
    pub fn init(
        &mut self,
        snapshot: &super::wasm_app::StateSnapshot,
        size: (f32, f32),
        now_ms: u64,
    ) -> wasmtime::Result<()> {
        let effects = self.app.init(snapshot, size)?;
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

    pub fn has_pending_capability_prompt(&self) -> bool {
        !self.pending_capability_prompts.is_empty()
    }

    pub fn pending_capability_prompt(&self) -> Option<&WasmCapabilityPrompt> {
        self.pending_capability_prompts.front()
    }

    pub fn decide_next_capability_prompt(&mut self, granted: bool) {
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
        log::info!(
            "wasm capability: decision app_id={} capability={} granted={}",
            self.app.app_id(),
            capability_id,
            granted
        );
        crate::host::event_log::emit(crate::host::event_log::HostEvent::PermissionDecision {
            app_id: self.app.app_id().to_string(),
            capability: capability_id,
            granted,
            timestamp: crate::host::event_log::now_timestamp(),
        });
    }

    /// Fire any due timers and drain the input queue. `now_ms` is monotonic
    /// elapsed milliseconds since the pane started.
    pub fn tick(&mut self, now_ms: u64) -> wasmtime::Result<()> {
        self.collect_http_results();
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
        let Some((width, height)) = first_surface_dims(&tree) else {
            return Ok(());
        };
        let Some(handle) = self.app.alloc_surface(width, height) else {
            return Ok(());
        };
        log::info!("wasm gpu: surface allocated {width}x{height} (texture {handle})");
        self.surface = Some(SurfaceState {
            handle,
            width,
            height,
        });
        self.queue.push_back(InputEvent::SurfaceReady(SurfaceEvent {
            texture_handle: handle,
            width,
            height,
        }));
        self.drain(now_ms)
    }

    /// Read the current surface texture back to an RGBA image, if a surface is
    /// allocated. Used to composite into egui (live) and assert pixels (gates).
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

    pub fn take_title(&mut self) -> Option<String> {
        self.pending_title.take()
    }

    pub fn take_status(&mut self) -> Option<String> {
        self.pending_status.take()
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
                    };
                    if let Err(e) = chunk_tx.send(event) {
                        log::warn!("wasm ai: stream receiver dropped: {e}");
                    }
                };
                let response = broker.dispatch(
                    AiBrokerRequest {
                        app_id,
                        model_tier,
                        system: req.system,
                        messages,
                        tools: Vec::new(),
                        workspace_root: None,
                        open_panes: crate::plexi_ai::broker::get_pane_snapshot(),
                        tool_dispatcher: None,
                        cancel: crate::plexi_ai::CancelToken::new(),
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
        self.app_timeline
            .lock()
            .unwrap()
            .declare_streams(self.app.app_id(), streams)
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
            self.app.app_id(),
            self.pane_id,
            emitted,
        )?;
        Ok(outcome.event_id)
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
        let resolved = if require_existing_file || full.exists() {
            std::fs::canonicalize(&full).map_err(|e| format!("resolve {}: {e}", full.display()))?
        } else {
            let parent = full
                .parent()
                .ok_or_else(|| format!("path has no parent: {}", full.display()))?;
            let parent = std::fs::canonicalize(parent)
                .map_err(|e| format!("resolve {}: {e}", parent.display()))?;
            let name = full
                .file_name()
                .ok_or_else(|| format!("path has no file name: {}", full.display()))?;
            parent.join(name)
        };
        if self.access.is_allowed_path(access, &resolved) {
            Ok(resolved)
        } else {
            Err(format!("path outside granted scope: {}", full.display()))
        }
    }

    fn read_file(&self, req: FileReadEffect) -> Result<Vec<u8>, String> {
        let path = self.scoped_path(FsAccess::Read, &req.path, true)?;
        std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))
    }

    fn write_file(&self, req: FileWriteEffect) -> Result<(), String> {
        let path = self.scoped_path(FsAccess::Write, &req.path, false)?;
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
            let response = net.http(&req.method, &req.url, &headers, body.as_deref());
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

// ─── Live adapter ───────────────────────────────────────────────────────────
//
// `LiveWasmPane` bridges the time-injected, headless [`WasmPane`] to the host's
// live egui render loop. It owns the monotonic clock (so `WasmPane` stays pure
// and testable), runs `init` lazily on the first frame once the pane size is
// known, translates egui key input into guest `InputEvent`s using the same
// printable-vs-named key split as `process_app`, and renders the guest's view
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
}

impl LiveWasmPane {
    /// Build a live pane. `init` is deferred to the first `ui` call, when the
    /// egui region size is known. `snapshot` is the persisted state to restore.
    pub fn new(inner: WasmPane, spawn_name: impl Into<String>, snapshot: StateSnapshot) -> Self {
        LiveWasmPane {
            inner,
            started: Instant::now(),
            spawn_name: spawn_name.into(),
            title: None,
            pending_init: Some(snapshot),
            error: None,
            last_text: String::new(),
            surface_id: None,
            surface_dims: None,
            fallback_tex: None,
            clock: super::wasm_frame::FrameClock::new(60),
            telemetry: super::wasm_frame::FrameTelemetry::new(240),
            last_present: None,
        }
    }

    fn now_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
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

    pub fn has_pending_capability_prompt(&self) -> bool {
        self.inner.has_pending_capability_prompt()
    }

    pub fn set_pane_id(&mut self, pane_id: u64) {
        self.inner.set_pane_id(pane_id);
    }

    pub fn draw_capability_modal(&mut self, ctx: &egui::Context, colors: &Colors) {
        let Some(prompt) = self.inner.pending_capability_prompt() else {
            return;
        };
        let capability_id = prompt.capability_id().to_string();
        let actions = [
            crate::ui::dialog::DialogAction::new(
                "grant",
                "Grant",
                crate::ui::button::ButtonKind::Primary,
            )
            .shortcut(crate::ui::dialog::DialogShortcut::new(
                &["Enter"],
                "grant",
                egui::Modifiers::NONE,
                egui::Key::Enter,
            )),
            crate::ui::dialog::DialogAction::new(
                "deny",
                "Deny",
                crate::ui::button::ButtonKind::Danger,
            )
            .shortcut(crate::ui::dialog::DialogShortcut::new(
                &["Esc"],
                "deny",
                egui::Modifiers::NONE,
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
                "This decision applies to the current session.",
                colors,
            );
        });

        let decision = if response.selected == Some("grant") {
            Some(true)
        } else if response.dismissed || response.selected == Some("deny") {
            Some(false)
        } else {
            None
        };

        if let Some(granted) = decision {
            self.inner.decide_next_capability_prompt(granted);
            if let Err(e) = self.inner.drain(self.now_ms()) {
                self.fail("capability decision", e);
            }
            ctx.request_repaint();
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, colors: &Colors) {
        if let Some(err) = &self.error {
            ui.colored_label(colors.danger, err);
            return;
        }

        let size = ui.available_size();
        let now = self.now_ms();

        let stepped = if let Some(snapshot) = self.pending_init.take() {
            self.inner.init(&snapshot, (size.x, size.y), now)
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
                // Fallback: no shared device, so read the surface back and
                // re-upload it as an egui texture (the slow legacy path).
                None => {
                    if let Some(img) = self.inner.read_surface() {
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
                    self.fallback_tex.as_ref().map(|t| t.id())
                }
            },
            None => None,
        };
        let result = render_ui_tree_with_surface(ui, &tree, colors, surface_tid);
        match self.inner.apply_render_result(result, now) {
            Ok(true) => {
                match self.inner.view() {
                    Ok(t) => self.last_text = collect_tree_text(&t),
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

        // Host-owned pacing + telemetry for surface apps: the clock advances by
        // wall-clock time and records interval/present/dropped metrics. The
        // guest never measures its own frame rate.
        if self.inner.surface_size().is_some() {
            let now_inst = Instant::now();
            if let Some(prev) = self.last_present {
                self.telemetry
                    .record_frame(now_inst.saturating_duration_since(prev));
            }
            self.last_present = Some(now_inst);
            let step = self.clock.advance(now_inst);
            self.telemetry.record_dropped(step.dropped);
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
        if self.inner.surface_size().is_some() {
            ui.ctx().request_repaint_after(self.clock.target_interval());
        }
        if self.inner.has_audio() {
            ui.ctx().request_repaint_after(Duration::from_millis(15));
        } else if let Some(deadline) = self.inner.next_deadline_ms() {
            ui.ctx()
                .request_repaint_after(Duration::from_millis(deadline.saturating_sub(now)));
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
    pub fn handle_key(&mut self, input: &egui::InputState) -> KeyDisposition {
        let mut consumed = false;
        for event in &input.events {
            match event {
                egui::Event::Key { .. } => {
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
        AiMessage as WitAiMessage, EventStreamDecl as WitEventStreamDecl,
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
        ) -> HostHttpResponse {
            let mut response_headers = HashMap::new();
            response_headers.insert("content-type".to_string(), vec!["text/plain".to_string()]);
            HostHttpResponse {
                status: 201,
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

    // init runs startup effects: the first get-system-stats resolves through
    // the stats source and the view shows the CPU percentage.
    #[test]
    fn init_resolves_first_stats() -> wasmtime::Result<()> {
        let mut p = pane(42.0);
        p.init(&StateSnapshot { entries: vec![] }, (400.0, 300.0), 0)?;
        assert!(cpu_text(&p.view()?).contains("42.0%"));
        Ok(())
    }

    // The repeating poll timer (id 1, 2000ms) fires once elapsed time passes
    // its deadline, requesting fresh stats that reflect the updated source.
    #[test]
    fn poll_timer_refreshes_stats() -> wasmtime::Result<()> {
        let mut p = pane(10.0);
        p.init(&StateSnapshot { entries: vec![] }, (400.0, 300.0), 0)?;
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
        p.init(&StateSnapshot { entries: vec![] }, (400.0, 300.0), 0)?;
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
        p.init(&StateSnapshot { entries: vec![] }, (400.0, 300.0), 0)?;
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

    #[test]
    fn live_wasm_pane_reports_pending_capability_prompt_for_focus() {
        let mut live = LiveWasmPane::new(pane(0.0), "wasm-test", StateSnapshot { entries: vec![] });
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

    // Lane B: typed-node interactions collected by the renderer are fed back
    // into the guest update loop, so a button click changes the next view.
    #[test]
    fn ui_button_click_updates_guest_view() -> wasmtime::Result<()> {
        use egui_kittest::kittest::Queryable;

        let mut p = counter_pane();
        p.init(&StateSnapshot { entries: vec![] }, (400.0, 300.0), 0)?;
        assert!(tree_text(&p.view()?).contains("Count: 0"));

        let colors = Colors::from_config(&crate::config::ThemeConfig::default());
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::Vec2::new(400.0, 300.0))
            .build_ui_state(
                move |ui, pane| {
                    let tree = pane.view().expect("view");
                    let result = render_ui_tree_with_surface(ui, &tree, &colors, None);
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

    // ── G7: surface-node lifecycle + input (Pong) ────────────────────────

    fn pong_fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wasm-fixtures/pong.wasm")
    }

    fn breakout_fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wasm-fixtures/breakout.wasm")
    }

    fn pong_pane() -> WasmPane {
        let app =
            WasmApp::load_ephemeral_run("pong", &pong_fixture(), StateStore::ephemeral())
                .expect("load pong (gpu device required)");
        WasmPane::new(app, Box::new(FakeStats { cpu: 0.0 }))
    }

    fn breakout_pane() -> WasmPane {
        let app = WasmApp::load_ephemeral_run(
            "breakout",
            &breakout_fixture(),
            StateStore::ephemeral(),
        )
        .expect("load breakout (gpu device required)");
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

    fn breakout_paddle_centroid_x(img: &image::RgbaImage) -> f32 {
        let (mut sum, mut count) = (0.0f32, 0u32);
        for y in 520..550u32 {
            for x in 0..img.width() {
                let p = img.get_pixel(x, y);
                if p[0] > 50 && p[1] > 80 && p[2] > 180 {
                    sum += x as f32;
                    count += 1;
                }
            }
        }
        assert!(count > 0, "no blue paddle pixels found in Breakout surface");
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
        p.init(&StateSnapshot { entries: vec![] }, (480.0, 360.0), 0)?;

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
        p.init(&StateSnapshot { entries: vec![] }, (480.0, 360.0), 0)?;

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

    // Breakout benchmark: the Breakout POC allocates a 900x600 surface,
    // draws the arena/brick grid/paddle/ball through the GPU import, and
    // responds to right-arrow input through the same guest update path.
    #[test]
    fn breakout_surface_lifecycle_and_input() -> wasmtime::Result<()> {
        let mut p = breakout_pane();
        p.init(&StateSnapshot { entries: vec![] }, (940.0, 700.0), 0)?;

        let (w, h) = p.surface_size().expect("surface allocated after init");
        assert_eq!((w, h), (900, 600));

        let before = p.read_surface().expect("surface readback");
        let brick_pixels = before
            .pixels()
            .filter(|px| px[2] > 180 && px[0] > 90 && px[1] > 90)
            .count();
        assert!(
            brick_pixels > 5_000,
            "rendered Breakout surface should include the brick field"
        );

        let cx_before = breakout_paddle_centroid_x(&before);
        p.push_input(key("right"));
        let mut t = 0u64;
        for _ in 0..45 {
            t += 16;
            p.tick(t)?;
        }
        let after = p.read_surface().expect("surface readback");
        let cx_after = breakout_paddle_centroid_x(&after);

        assert!(
            cx_after > cx_before + 80.0,
            "Breakout paddle moved right after held input: {cx_before} -> {cx_after}"
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

    // End-to-end: a held control moves the paddle, and releasing it stops the
    // paddle. Only possible if release edges flow through to the guest.
    #[test]
    fn pong_paddle_stops_on_release() -> wasmtime::Result<()> {
        let mut p = pong_pane();
        p.init(&StateSnapshot { entries: vec![] }, (480.0, 360.0), 0)?;
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
}
