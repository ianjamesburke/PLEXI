/// ProcessApp — runs an external app binary as a subprocess and renders it
/// using the Plexi draw protocol.
///
/// The subprocess speaks the app protocol over stdin/stdout (newline-delimited JSON).
/// ProcessApp implements the `App` trait so it drops in wherever a built-in app
/// would — the rest of Plexi doesn't know or care that it's an external process.

use crate::app_permissions::{AppPermissions, Capability, PermissionsLog, check};
use crate::app_protocol::{DrawCommand, Modifiers, PlexiEvent};
use crate::app_trait::{App, AppCommand, AppRenderContext};
use crate::event_log::{self, HostEvent};
use crate::media::{audio_device, video_decoder, AudioSource, VideoSource, PlaybackState};
use crate::runs::RunRegistry;
use crate::typed_pipes::{PipeDirection, TypedPipeRegistry};
use egui::Color32;
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Stdio};
use std::sync::{mpsc::{self, Receiver, TryRecvError}, Arc, Mutex};
use std::thread;

// ---------------------------------------------------------------------------
// PendingPrompt — capability / secret prompts queued for modal presentation
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum PendingPrompt {
    Capability {
        request_id: String,
        capability: String,
    },
    Secret {
        key: String,
    },
}

// ---------------------------------------------------------------------------
// AudioMeter display state
// ---------------------------------------------------------------------------

struct AudioMeterState {
    rect_x: f32,
    rect_y: f32,
    rect_w: f32,
    rect_h: f32,
    pipe_id: String,
}

// ---------------------------------------------------------------------------
// ProcessApp
// ---------------------------------------------------------------------------

pub struct ProcessApp {
    type_id: String,
    display_name: String,
    accepted_exts: Vec<String>,
    process: Option<Child>,
    stdin: Option<ChildStdin>,
    /// Receives draw commands from the subprocess on a background thread.
    draw_rx: Option<Receiver<DrawCommand>>,
    /// The last fully committed frame (commands between two FrameDones).
    /// Always valid — only replaced atomically when a complete new frame arrives.
    frame: Vec<DrawCommand>,
    /// Accumulates draw commands for the frame currently being received.
    /// Committed into `frame` on FrameDone; never shown until complete.
    pending_frame: Vec<DrawCommand>,
    /// Pending RunInTerminal / Cd commands collected from the subprocess, to be
    /// drained by the host via take_pending_commands().
    pending_commands: Vec<crate::app_trait::AppCommand>,
    /// Size last sent to the subprocess.
    last_size: egui::Vec2,
    initialized: bool,
    /// Monotonic frame counter for frame_id tagging.
    /// The UI drives this — incremented each time we send a Render event.
    frame_counter: u64,
    /// SDK identifier from the Ready handshake.
    sdk: Option<String>,
    /// Feature flags the app declared it uses (from Ready).
    features_used: Vec<String>,
    /// workspace_root sent in Init — used to enforce SecretGet scope invariant.
    workspace_root: PathBuf,
    /// Granted capabilities — used to gate media and secret access.
    permissions: AppPermissions,
    /// Typed pipe registry for this app instance. Arc<Mutex<>> so the audio
    /// capture forwarding thread can call write_binary without blocking the UI.
    pipe_registry: Arc<Mutex<TypedPipeRegistry>>,
    /// Run registry for this app instance.
    run_registry: RunRegistry,
    /// Prompts awaiting user decision (capability + secret grants).
    pending_prompts: VecDeque<PendingPrompt>,
    /// Latest status text (shown in pane chrome).
    status_summary: Option<String>,
    /// Active audio meter configs (rendered each frame).
    audio_meters: Vec<AudioMeterState>,
    /// Video decoder (one per ProcessApp — shared across video sources).
    video_decoder: Option<Box<dyn crate::media::VideoDecoder>>,
    /// Active video handles keyed by source string; value is handle_id.
    video_handles: HashMap<String, crate::media::VideoHandle>,
    /// Pending video frames to upload + render on the next render pass.
    /// Each entry: (handle_id, frame, rect x, y, w, h).
    pending_video_frames: Vec<(u64, crate::media::VideoFrame, f32, f32, f32, f32)>,
    /// Cached egui texture handles keyed by video handle_id, for frame-by-frame upload.
    video_textures: HashMap<u64, egui::TextureHandle>,
    /// Active audio playback handles keyed by source string.
    audio_playback_handles: HashMap<String, crate::media::AudioPlaybackHandle>,
    /// Active audio capture handles keyed by pipe_id.
    audio_capture_handles: HashMap<String, crate::media::AudioCaptureHandle>,
    /// Pending PlexiEvents queued to be sent to the app on next flush.
    outbound_events: VecDeque<PlexiEvent>,
    /// Text input buffer for the secret-value modal.
    secret_input_buf: String,
}

impl ProcessApp {
    /// Spawn an app binary at `bin_path`.
    ///
    /// `workspace_root` must be an absolute existing directory — validated here.
    /// `capabilities` are the granted capabilities for this app instance.
    pub fn launch(
        type_id: impl Into<String>,
        display_name: impl Into<String>,
        accepted_exts: Vec<String>,
        bin_path: &PathBuf,
        cwd: &PathBuf,
        args: &[String],
        workspace_root: PathBuf,
        capabilities: std::collections::HashSet<Capability>,
    ) -> Result<Self, std::io::Error> {
        let type_id: String = type_id.into();
        let display_name: String = display_name.into();

        // Validate workspace_root invariant.
        if workspace_root.as_os_str().is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "workspace_root must be non-empty",
            ));
        }
        if !workspace_root.is_absolute() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("workspace_root must be absolute: {}", workspace_root.display()),
            ));
        }
        if !workspace_root.is_dir() {
            log::warn!(
                "ProcessApp: workspace_root '{}' does not exist yet; proceeding",
                workspace_root.display()
            );
        }

        let mut child = std::process::Command::new(bin_path)
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()) // captured and forwarded to Plexi's logger
            .spawn()?;

        let stdin = child.stdin.take().expect("stdin piped");
        let stdout: ChildStdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");

        // Background thread: forward subprocess stderr lines to Plexi's logger.
        let stderr_type_id = type_id.clone();
        thread::spawn(move || {
            let reader = std::io::BufReader::new(stderr);
            for line in std::io::BufRead::lines(reader) {
                match line {
                    Ok(l) if !l.trim().is_empty() => {
                        let target = format!("app::{stderr_type_id}");
                        log::warn!(target: &target, "stderr: {l}");
                    }
                    Err(_) => break,
                    _ => {}
                }
            }
        });

        // Background thread: read draw commands line-by-line and forward via channel.
        // Also handles AppReply::Ready — it's the first line and deserialized here.
        let (draw_tx, draw_rx) = mpsc::channel::<DrawCommand>();
        thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                match line {
                    Ok(l) if !l.trim().is_empty() => {
                        match serde_json::from_str::<DrawCommand>(&l) {
                            Ok(cmd) => {
                                if draw_tx.send(cmd).is_err() {
                                    break; // receiver dropped — Plexi closed the app
                                }
                            }
                            Err(e) => {
                                log::warn!("ProcessApp: malformed draw command: {e} — line: {l}");
                            }
                        }
                    }
                    Err(e) => {
                        log::debug!("ProcessApp stdout closed: {e}");
                        break;
                    }
                    _ => {}
                }
            }
        });

        let permissions = AppPermissions {
            capabilities,
            is_builtin: false,
        };

        // Emit AppSpawned before returning — pane_id unknown at this point (0 as placeholder;
        // the caller will have assigned the real pane_id when it inserts into the pane map).
        event_log::emit(HostEvent::AppSpawned {
            app_id: type_id.clone(),
            type_id: type_id.clone(),
            pane_id: 0,
            timestamp: event_log::now_timestamp(),
        });

        Ok(Self {
            type_id,
            display_name,
            accepted_exts,
            process: Some(child),
            stdin: Some(stdin),
            draw_rx: Some(draw_rx),
            frame: Vec::new(),
            pending_frame: Vec::new(),
            pending_commands: Vec::new(),
            last_size: egui::Vec2::ZERO,
            initialized: false,
            frame_counter: 0,
            sdk: None,
            features_used: Vec::new(),
            workspace_root,
            permissions,
            pipe_registry: Arc::new(Mutex::new(TypedPipeRegistry::new())),
            run_registry: RunRegistry::new(),
            pending_prompts: VecDeque::new(),
            status_summary: None,
            audio_meters: Vec::new(),
            video_decoder: None,
            video_handles: HashMap::new(),
            pending_video_frames: Vec::new(),
            video_textures: HashMap::new(),
            audio_playback_handles: HashMap::new(),
            audio_capture_handles: HashMap::new(),
            outbound_events: VecDeque::new(),
            secret_input_buf: String::new(),
        })
    }

    /// Spawn with minimal args — workspace_root defaults to cwd.
    /// Used for legacy TerminalPane-overlay apps and tests.
    pub fn launch_simple(
        type_id: impl Into<String>,
        display_name: impl Into<String>,
        accepted_exts: Vec<String>,
        bin_path: &PathBuf,
        cwd: &PathBuf,
        args: &[String],
    ) -> Result<Self, std::io::Error> {
        Self::launch(
            type_id,
            display_name,
            accepted_exts,
            bin_path,
            cwd,
            args,
            cwd.clone(),
            std::collections::HashSet::new(),
        )
    }

    /// Return the latest status text for pane chrome rendering.
    pub fn status_summary(&self) -> Option<&str> {
        self.status_summary.as_deref()
    }

    /// Return all active runs for the Run palette.
    pub fn list_runs(&self) -> Vec<&crate::runs::Run> {
        self.run_registry.list_runs()
    }

    /// Drain any pending prompts (capability / secret). The host UI calls this
    /// each frame and renders a modal for the first pending prompt.
    pub fn drain_pending_prompts(&mut self) -> Option<PendingPrompt> {
        self.pending_prompts.pop_front()
    }

    /// Resolve a pending capability prompt and emit `CapabilityDecision` back to the app.
    pub fn resolve_capability(
        &mut self,
        request_id: &str,
        capability_str: &str,
        granted: bool,
        perms_log: &mut PermissionsLog,
    ) {
        let cap = Capability::from(capability_str);
        perms_log.record(&self.type_id, &self.workspace_root, cap, granted);
        if granted {
            self.permissions.capabilities.insert(cap);
        }
        self.outbound_events.push_back(PlexiEvent::CapabilityDecision {
            request_id: request_id.to_string(),
            granted,
        });
    }

    /// Resolve a pending secret prompt and emit `SecretValue` back to the app.
    pub fn resolve_secret(&mut self, key: &str, value: Option<String>) {
        self.outbound_events.push_back(PlexiEvent::SecretValue {
            key: key.to_string(),
            value,
        });
    }

    fn send_event(&mut self, event: &PlexiEvent) {
        let Some(stdin) = self.stdin.as_mut() else {
            return;
        };
        match serde_json::to_string(event) {
            Ok(mut line) => {
                line.push('\n');
                if let Err(e) = stdin.write_all(line.as_bytes()) {
                    log::warn!("ProcessApp: failed to write event: {e}");
                    self.stdin = None; // process probably died
                }
            }
            Err(e) => log::error!("ProcessApp: failed to serialize event: {e}"),
        }
    }

    fn flush_outbound_events(&mut self) {
        while let Some(event) = self.outbound_events.pop_front() {
            self.send_event(&event);
        }
    }

    fn drain_draw_commands(&mut self) -> Vec<DrawCommand> {
        let Some(rx) = self.draw_rx.as_ref() else {
            return vec![];
        };
        let mut cmds = Vec::new();
        loop {
            match rx.try_recv() {
                Ok(cmd) => cmds.push(cmd),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.draw_rx = None;
                    break;
                }
            }
        }
        cmds
    }

    /// Route a v3 out-of-frame draw command to the appropriate subsystem.
    fn route_command(&mut self, cmd: DrawCommand) {
        match cmd {
            // ── Video ──────────────────────────────────────────────────────
            DrawCommand::VideoPlayer { source, x, y, w, h, state } => {
                if !self.video_handles.contains_key(&source) {
                    // Lazily create the decoder on first video open.
                    if self.video_decoder.is_none() {
                        self.video_decoder = Some(video_decoder());
                    }
                    if let Some(decoder) = self.video_decoder.as_mut() {
                        match decoder.open(VideoSource::File(PathBuf::from(&source))) {
                            Ok(handle) => {
                                log::info!(
                                    "ProcessApp[{}]: opened video '{}' {}x{} {}ms",
                                    self.type_id, source, handle.width, handle.height, handle.duration_ms
                                );
                                self.video_handles.insert(source.clone(), handle);
                            }
                            Err(e) => {
                                log::warn!("ProcessApp[{}]: failed to open video '{}': {e}", self.type_id, source);
                                return;
                            }
                        }
                    }
                }

                if let Some(handle) = self.video_handles.get(&source) {
                    let handle_id = handle.id;
                    let playback_state = if state == "play" {
                        Some(PlaybackState::Play)
                    } else if state == "pause" {
                        Some(PlaybackState::Pause)
                    } else if let Some(ms_str) = state.strip_prefix("seek:") {
                        ms_str.parse::<u64>().ok().map(PlaybackState::Seek)
                    } else {
                        None
                    };

                    if let Some(ps) = playback_state {
                        if let Some(decoder) = self.video_decoder.as_mut() {
                            decoder.set_state(handle_id, ps);
                        }
                    }

                    // Pull next frame and queue for upload+render in the render pass
                    // (egui texture upload requires a UI context, not available here).
                    if let Some(decoder) = self.video_decoder.as_mut() {
                        if let Some(frame) = decoder.next_frame(handle_id) {
                            log::debug!(
                                "ProcessApp[{}]: VideoFrame {}x{} pts={}ms source={source}",
                                self.type_id, frame.width, frame.height, frame.pts_ms
                            );
                            // Queue for texture upload in render_draw_commands.
                            self.pending_video_frames.push((handle_id, frame, x, y, w, h));
                        }
                    }
                }
            }

            // ── Audio playback ─────────────────────────────────────────────
            DrawCommand::AudioPlay { source, volume, state } => {
                if state == "stop" {
                    // Stop: drop the handle (Drop impl signals stop_tx).
                    if self.audio_playback_handles.remove(&source).is_some() {
                        log::info!("ProcessApp[{}]: stopped audio '{source}'", self.type_id);
                    }
                    return;
                }

                if !self.audio_playback_handles.contains_key(&source) && state == "play" {
                    let mut device = audio_device();
                    match device.start_playback(AudioSource::File(PathBuf::from(&source)), volume) {
                        Ok(handle) => {
                            log::info!("ProcessApp[{}]: started audio playback '{source}' vol={volume}", self.type_id);
                            self.audio_playback_handles.insert(source.clone(), handle);
                        }
                        Err(e) => {
                            log::warn!("ProcessApp[{}]: audio start_playback failed: {e}", self.type_id);
                        }
                    }
                }
            }

            // ── Audio capture ──────────────────────────────────────────────
            DrawCommand::AudioCapture { pipe_id, sample_rate, buffer_size } => {
                // Capability gate.
                if let crate::app_permissions::PermissionCheck::Denied(reason) =
                    check(&self.permissions, Capability::AudioRecord)
                {
                    log::warn!(
                        "ProcessApp[{}]: AudioCapture denied — {reason}",
                        self.type_id
                    );
                    return;
                }

                if self.audio_capture_handles.contains_key(&pipe_id) {
                    log::warn!("ProcessApp[{}]: AudioCapture pipe '{}' already open", self.type_id, pipe_id);
                    return;
                }

                // Allocate binary pipe.
                let alloc = match self.pipe_registry.lock().unwrap().open_binary(
                    pipe_id.clone(),
                    PipeDirection::Out,
                ) {
                    Ok(a) => a,
                    Err(e) => {
                        log::warn!("ProcessApp[{}]: AudioCapture pipe alloc failed: {e}", self.type_id);
                        return;
                    }
                };

                // Start capture.
                let mut device = audio_device();
                match device.start_capture(sample_rate, buffer_size) {
                    Ok(capture_handle) => {
                        let socket_path = alloc.socket_path.clone();

                        // Forwarding thread: read PCM frames and write to binary pipe.
                        // TypedPipeRegistry is behind Arc<Mutex<>> so this thread can
                        // call write_binary without blocking the main UI thread.
                        let pipe_id_fwd = pipe_id.clone();
                        let type_id_fwd = self.type_id.clone();
                        let registry_arc = Arc::clone(&self.pipe_registry);
                        thread::Builder::new()
                            .name(format!("audio-capture-{pipe_id_fwd}"))
                            .spawn(move || {
                                log::info!("ProcessApp[{type_id_fwd}]: audio capture thread started for pipe '{pipe_id_fwd}'");
                                loop {
                                    match capture_handle.receiver.recv() {
                                        Ok(samples) => {
                                            // Convert f32 PCM to raw bytes (little-endian f32).
                                            let bytes: Vec<u8> = samples
                                                .iter()
                                                .flat_map(|s| s.to_le_bytes())
                                                .collect();
                                            let mut reg = registry_arc.lock().unwrap();
                                            match reg.write_binary(&pipe_id_fwd, &bytes) {
                                                Ok(_) => {}
                                                Err(e) => {
                                                    log::warn!("ProcessApp[{type_id_fwd}]: audio write failed: {e}");
                                                }
                                            }
                                            event_log::emit(HostEvent::PipeWrite {
                                                from_app: type_id_fwd.clone(),
                                                channel: pipe_id_fwd.clone(),
                                                timestamp: event_log::now_timestamp(),
                                            });
                                        }
                                        Err(_) => break,
                                    }
                                }
                                log::info!("ProcessApp[{type_id_fwd}]: audio capture thread exiting for pipe '{pipe_id_fwd}'");
                            })
                            .ok();

                        self.outbound_events.push_back(PlexiEvent::PipeOpened {
                            pipe_id: pipe_id.clone(),
                            socket_path,
                        });
                        log::info!(
                            "ProcessApp[{}]: AudioCapture started on pipe '{pipe_id}' {}hz buf={}",
                            self.type_id, sample_rate, buffer_size
                        );
                    }
                    Err(e) => {
                        log::warn!("ProcessApp[{}]: AudioCapture start_capture failed: {e}", self.type_id);
                        self.pipe_registry.lock().unwrap().close(&pipe_id);
                    }
                }
            }

            // ── Audio meter ────────────────────────────────────────────────
            DrawCommand::AudioMeter { x, y, w, h, pipe_id } => {
                // Update or insert meter state; rendered each frame in render_draw_commands.
                if let Some(existing) = self.audio_meters.iter_mut().find(|m| m.pipe_id == pipe_id) {
                    existing.rect_x = x;
                    existing.rect_y = y;
                    existing.rect_w = w;
                    existing.rect_h = h;
                } else {
                    self.audio_meters.push(AudioMeterState { rect_x: x, rect_y: y, rect_w: w, rect_h: h, pipe_id });
                }
            }

            // ── Capability request ─────────────────────────────────────────
            DrawCommand::CapabilityRequest { request_id, capability } => {
                let cap = Capability::from(capability.as_str());
                if let crate::app_permissions::PermissionCheck::Allowed = check(&self.permissions, cap) {
                    // Already granted — respond immediately.
                    self.outbound_events.push_back(PlexiEvent::CapabilityDecision {
                        request_id,
                        granted: true,
                    });
                } else {
                    // Queue for user prompt.
                    event_log::emit(HostEvent::PermissionPrompted {
                        app_id: self.type_id.clone(),
                        capability: capability.clone(),
                        timestamp: event_log::now_timestamp(),
                    });
                    self.pending_prompts.push_back(PendingPrompt::Capability {
                        request_id,
                        capability,
                    });
                }
            }

            // ── Secret get ─────────────────────────────────────────────────
            DrawCommand::SecretGet { key } => {
                // Gate: app must have SecretsGet capability.
                if let crate::app_permissions::PermissionCheck::Denied(reason) =
                    check(&self.permissions, Capability::SecretsGet)
                {
                    log::warn!("ProcessApp[{}]: SecretGet denied — {reason}", self.type_id);
                    self.outbound_events.push_back(PlexiEvent::SecretValue { key, value: None });
                    return;
                }

                #[cfg(target_os = "macos")]
                {
                    match crate::secrets::get_secret_scoped(&key, &self.type_id, &self.workspace_root) {
                        Some(value) => {
                            self.outbound_events.push_back(PlexiEvent::SecretValue {
                                key,
                                value: Some(value.to_string()),
                            });
                        }
                        None => {
                            // Not found — queue a prompt so the user can supply it.
                            self.pending_prompts.push_back(PendingPrompt::Secret { key });
                        }
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    log::warn!("ProcessApp[{}]: SecretGet not supported on this platform", self.type_id);
                    self.outbound_events.push_back(PlexiEvent::SecretValue { key, value: None });
                }
            }

            // ── Run get ────────────────────────────────────────────────────
            DrawCommand::RunGet { intent, payload } => {
                let run_id = self.run_registry.allocate(&self.type_id, &intent, payload.clone());
                log::info!(
                    "ProcessApp[{}]: RunGet intent='{}' → run_id='{}'",
                    self.type_id, intent, run_id
                );
                event_log::emit(HostEvent::RunCreated {
                    run_id: run_id.clone(),
                    app_id: self.type_id.clone(),
                    timestamp: event_log::now_timestamp(),
                });
                self.outbound_events.push_back(PlexiEvent::RunUpdate {
                    run_id,
                    status: "pending".to_string(),
                    payload: serde_json::Value::Null,
                });
            }

            // ── Run complete ───────────────────────────────────────────────
            DrawCommand::RunComplete { run_id, result } => {
                log::info!(
                    "ProcessApp[{}]: RunComplete run_id='{run_id}' result={result}",
                    self.type_id
                );
                // complete() emits RunCompleted internally.
                self.run_registry.complete(&run_id);
            }

            // ── Notify ─────────────────────────────────────────────────────
            DrawCommand::Notify { level, title, body, actions } => {
                let notif_id = format!("{}-{}", self.type_id, event_log::now_timestamp());
                log::info!(
                    "ProcessApp[{}]: Notify [{level}] '{title}': {body}",
                    self.type_id
                );
                event_log::emit(HostEvent::NotificationEmitted {
                    id: notif_id.clone(),
                    title: title.clone(),
                    urgency: level.clone(),
                    timestamp: event_log::now_timestamp(),
                });

                for action in &actions {
                    log::info!(
                        "ProcessApp[{}]: notify action action_type={} payload={}",
                        self.type_id, action.action_type, action.payload
                    );
                    match action.action_type.as_str() {
                        "resume_run" => {
                            if let Some(run_id) = action.payload.get("run_id").and_then(|v| v.as_str()) {
                                self.run_registry.resume(run_id);
                                self.outbound_events.push_back(PlexiEvent::RunUpdate {
                                    run_id: run_id.to_string(),
                                    status: "pending".to_string(),
                                    payload: serde_json::Value::Null,
                                });
                                event_log::emit(HostEvent::NotificationActioned {
                                    id: notif_id.clone(),
                                    action: "resume_run".to_string(),
                                    timestamp: event_log::now_timestamp(),
                                });
                            }
                        }
                        "open_intent" => {
                            if let Some(intent) = action.payload.get("intent").and_then(|v| v.as_str()) {
                                // Queue a pending intent — the UI pops this next frame and shows a modal.
                                self.pending_commands.push(AppCommand::Notify(
                                    format!("[intent] {intent}")
                                ));
                                event_log::emit(HostEvent::NotificationActioned {
                                    id: notif_id.clone(),
                                    action: "open_intent".to_string(),
                                    timestamp: event_log::now_timestamp(),
                                });
                            }
                        }
                        "run_command" => {
                            if let Some(command) = action.payload.get("command").and_then(|v| v.as_str()) {
                                self.pending_commands.push(AppCommand::RunInTerminal(command.to_string()));
                                event_log::emit(HostEvent::NotificationActioned {
                                    id: notif_id.clone(),
                                    action: "run_command".to_string(),
                                    timestamp: event_log::now_timestamp(),
                                });
                            } else {
                                log::warn!("ProcessApp[{}]: run_command action missing 'command' payload", self.type_id);
                            }
                        }
                        other => {
                            log::warn!("ProcessApp[{}]: unknown notify action_type='{other}'", self.type_id);
                        }
                    }
                }
                // Always push the notification to the host notification system too.
                self.pending_commands.push(AppCommand::Notify(
                    format!("[{level}] {title}: {body}")
                ));
            }

            // ── Pipe open ──────────────────────────────────────────────────
            DrawCommand::PipeOpen { pipe_id, mode, direction } => {
                // Capability gate.
                if let crate::app_permissions::PermissionCheck::Denied(reason) =
                    check(&self.permissions, Capability::PipeOpen)
                {
                    log::warn!("ProcessApp[{}]: PipeOpen denied — {reason}", self.type_id);
                    return;
                }

                let dir = match direction.as_str() {
                    "in" => PipeDirection::In,
                    "out" => PipeDirection::Out,
                    _ => PipeDirection::Duplex,
                };

                if mode == "binary" {
                    match self.pipe_registry.lock().unwrap().open_binary(pipe_id.clone(), dir) {
                        Ok(alloc) => {
                            log::info!(
                                "ProcessApp[{}]: opened binary pipe '{pipe_id}' → {}",
                                self.type_id, alloc.socket_path
                            );
                            self.outbound_events.push_back(PlexiEvent::PipeOpened {
                                pipe_id,
                                socket_path: alloc.socket_path,
                            });
                        }
                        Err(e) => log::warn!("ProcessApp[{}]: PipeOpen binary failed: {e}", self.type_id),
                    }
                } else {
                    // JSON mode.
                    match self.pipe_registry.lock().unwrap().open_json(pipe_id.clone(), dir) {
                        Ok(()) => {
                            log::info!("ProcessApp[{}]: opened JSON pipe '{pipe_id}'", self.type_id);
                        }
                        Err(e) => log::warn!("ProcessApp[{}]: PipeOpen json failed: {e}", self.type_id),
                    }
                }
            }

            // ── Pipe send ──────────────────────────────────────────────────
            DrawCommand::PipeSend { pipe_id, payload } => {
                match self.pipe_registry.lock().unwrap().send_json(&pipe_id, payload.clone()) {
                    Ok(()) => {
                        // Broadcast back as PipeMessage to any subscribed peers.
                        // In the single-app case, this echoes back to the same app for
                        // round-trip testing.
                        // TODO(layer-5): route PipeMessage to peer apps subscribed on this pipe_id;
                        //   requires a global app registry or shared pipe broker visible at the host level.
                        self.outbound_events.push_back(PlexiEvent::PipeMessage {
                            pipe_id,
                            payload,
                        });
                    }
                    Err(e) => log::warn!("ProcessApp[{}]: PipeSend failed: {e}", self.type_id),
                }
            }

            // ── Status summary ─────────────────────────────────────────────
            DrawCommand::StatusSummary { text } => {
                self.status_summary = Some(text);
            }

            // All other commands are visual — pushed to pending_frame upstream.
            _ => unreachable!("route_command called with non-control command"),
        }
    }

    fn render_draw_commands(
        ui: &mut egui::Ui,
        commands: &[DrawCommand],
        colors: &crate::theme::Colors,
        audio_meters: &[AudioMeterState],
    ) {
        let origin = ui.min_rect().min;

        for cmd in commands {
            match cmd {
                DrawCommand::Rect { x, y, w, h, fill, radius } => {
                    let rect = egui::Rect::from_min_size(
                        egui::pos2(origin.x + x, origin.y + y),
                        egui::vec2(*w, *h),
                    );
                    let color = parse_color(fill).unwrap_or(colors.bg_active);
                    ui.painter().rect_filled(rect, *radius, color);
                }

                DrawCommand::Text { x, y, text, size, color, monospace, bold } => {
                    let color = parse_color(color).unwrap_or(colors.text_primary);
                    let font_id = if *monospace {
                        egui::FontId::monospace(*size)
                    } else if *bold {
                        egui::FontId::proportional(*size)
                    } else {
                        egui::FontId::proportional(*size)
                    };
                    ui.painter().text(
                        egui::pos2(origin.x + x, origin.y + y),
                        egui::Align2::LEFT_TOP,
                        text,
                        font_id,
                        color,
                    );
                }

                DrawCommand::Line { x1, y1, x2, y2, color, width } => {
                    let color = parse_color(color).unwrap_or(colors.bg_active);
                    ui.painter().line_segment(
                        [
                            egui::pos2(origin.x + x1, origin.y + y1),
                            egui::pos2(origin.x + x2, origin.y + y2),
                        ],
                        egui::Stroke::new(*width, color),
                    );
                }

                DrawCommand::List { items, selected, item_height } => {
                    let row_h = if *item_height > 0.0 { *item_height } else { 20.0 };
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for (i, item) in items.iter().enumerate() {
                                let is_sel = i == *selected;
                                let (row_rect, _) = ui.allocate_exact_size(
                                    egui::vec2(ui.available_width(), row_h),
                                    egui::Sense::hover(),
                                );
                                if is_sel {
                                    ui.painter().rect_filled(row_rect, 2.0, colors.bg_active);
                                }
                                let icon = if item.is_dir { "▶ " } else { "  " };
                                let label = format!("{}{}", icon, item.label);
                                ui.painter().text(
                                    egui::pos2(row_rect.min.x + 8.0, row_rect.center().y),
                                    egui::Align2::LEFT_CENTER,
                                    &label,
                                    egui::FontId::monospace(12.0),
                                    if is_sel { colors.text_primary } else { colors.text_dim },
                                );
                                if let Some(sec) = &item.secondary {
                                    ui.painter().text(
                                        egui::pos2(row_rect.max.x - 8.0, row_rect.center().y),
                                        egui::Align2::RIGHT_CENTER,
                                        sec,
                                        egui::FontId::proportional(10.0),
                                        colors.text_dim,
                                    );
                                }
                            }
                        });
                }

                // RunInTerminal / Cd / Log / FrameDone handled at the App trait level, not here.
                DrawCommand::RunInTerminal { .. }
                | DrawCommand::Cd { .. }
                | DrawCommand::Log { .. }
                | DrawCommand::FrameDone { .. } => {}

                // v3 media + control commands — routed via route_command() before reaching here.
                // They don't reach this render path (they're drained in the update loop).
                DrawCommand::VideoPlayer { .. }
                | DrawCommand::AudioMeter { .. }
                | DrawCommand::CapabilityRequest { .. }
                | DrawCommand::SecretGet { .. }
                | DrawCommand::RunGet { .. }
                | DrawCommand::RunComplete { .. }
                | DrawCommand::Notify { .. }
                | DrawCommand::AudioPlay { .. }
                | DrawCommand::AudioCapture { .. }
                | DrawCommand::PipeOpen { .. }
                | DrawCommand::PipeSend { .. }
                | DrawCommand::StatusSummary { .. } => {}
            }
        }

        // Render audio meters.
        // TODO(layer-5): proper meter rendering — read PCM peak from pipe ring and
        //   draw a bar proportional to amplitude. Requires exposing a peak-read API
        //   on TypedPipeRegistry. For now, draw a static placeholder rect.
        for meter in audio_meters {
            let rect = egui::Rect::from_min_size(
                egui::pos2(origin.x + meter.rect_x, origin.y + meter.rect_y),
                egui::vec2(meter.rect_w, meter.rect_h),
            );
            // Background.
            ui.painter().rect_filled(rect, 2.0, Color32::from_rgb(30, 30, 46));
            // Placeholder fill bar (static 30% — real amplitude in Layer 4).
            let fill_h = meter.rect_h * 0.3;
            let fill_rect = egui::Rect::from_min_size(
                egui::pos2(rect.min.x, rect.max.y - fill_h),
                egui::vec2(meter.rect_w, fill_h),
            );
            ui.painter().rect_filled(fill_rect, 2.0, Color32::from_rgb(166, 227, 161));
        }
    }

    /// Show a modal egui window for the first pending prompt (capability or secret).
    /// Blocks user interaction with the pane until the user grants or denies.
    fn show_prompt_modal(
        ui: &mut egui::Ui,
        pending_prompts: &mut VecDeque<PendingPrompt>,
        outbound_events: &mut VecDeque<PlexiEvent>,
        permissions: &mut AppPermissions,
        type_id: &str,
        workspace_root: &PathBuf,
        secret_input_buf: &mut String,
    ) {
        let Some(prompt) = pending_prompts.front() else { return };

        let mut granted = false;
        let mut denied = false;

        egui::Window::new("Plexi needs permission")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ui.ctx(), |ui| {
                match prompt {
                    PendingPrompt::Capability { capability, .. } => {
                        ui.label(format!("App \"{}\" is requesting access to:", type_id));
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new(capability).monospace().strong());
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new(format!(
                            "Workspace: {}",
                            workspace_root.display()
                        )).small());
                    }
                    PendingPrompt::Secret { key } => {
                        ui.label(format!("App \"{}\" needs a secret value for:", type_id));
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new(key).monospace().strong());
                        ui.add_space(8.0);
                        ui.label("Enter value:");
                        ui.text_edit_singleline(secret_input_buf);
                    }
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("Grant").clicked() {
                        granted = true;
                    }
                    if ui.button("Deny").clicked() {
                        denied = true;
                    }
                });
            });

        if granted || denied {
            match pending_prompts.pop_front() {
                Some(PendingPrompt::Capability { request_id, capability }) => {
                    if granted {
                        let cap = Capability::from(capability.as_str());
                        permissions.capabilities.insert(cap);
                    }
                    outbound_events.push_back(PlexiEvent::CapabilityDecision {
                        request_id,
                        granted,
                    });
                }
                Some(PendingPrompt::Secret { key }) => {
                    let value = if granted && !secret_input_buf.is_empty() {
                        Some(secret_input_buf.clone())
                    } else {
                        None
                    };
                    secret_input_buf.clear();
                    outbound_events.push_back(PlexiEvent::SecretValue { key, value });
                }
                None => {}
            }
        }
    }
}

impl App for ProcessApp {
    fn type_id(&self) -> &'static str {
        // SAFETY: type_id is set at construction and never changes.
        // We leak a clone to get a &'static str. This is fine for the small number
        // of ProcessApp instances that will exist.
        Box::leak(self.type_id.clone().into_boxed_str())
    }

    fn display_name(&self) -> String {
        self.display_name.clone()
    }

    fn accepted_extensions(&self) -> &[&str] {
        &[] // dynamic; checked at registry level
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &AppRenderContext<'_>) {
        let size = ui.available_size();

        // Flush any outbound events queued last frame (e.g. CapabilityDecision).
        self.flush_outbound_events();

        // Send Init on first render.
        if !self.initialized {
            self.initialized = true;
            self.last_size = size;
            let cap_strings: Vec<String> = self.permissions.capabilities
                .iter()
                .map(|c| c.to_string())
                .collect();
            self.send_event(&PlexiEvent::Init {
                protocol: "pgap/3".to_string(),
                app_id: self.type_id.clone(),
                workspace_root: self.workspace_root.clone(),
                capabilities: cap_strings,
                feature_flags: vec!["media_v1".into(), "pane_groups_v1".into()],
            });
        }

        // Send Resize if size changed.
        if (size - self.last_size).length() > 1.0 {
            self.last_size = size;
            self.send_event(&PlexiEvent::Resize { width: size.x, height: size.y });
        }

        // Request a new frame.
        self.frame_counter += 1;
        let frame_id = self.frame_counter;
        self.send_event(&PlexiEvent::Render {
            frame_id,
            rect: crate::app_protocol::Rect { x: 0.0, y: 0.0, w: size.x, h: size.y },
        });

        // Drain all draw commands that arrived since last frame.
        let new_cmds = self.drain_draw_commands();

        // First pass: handle AppReply::Ready (the Ready message is parsed by the
        // background thread as a DrawCommand parse — it will fail, but we log
        // it via the stdout reader. There's no separate channel for Ready; in the
        // v3 design the background reader would need to distinguish the first line.
        // For now, Ready handshake is best-effort: the background reader logs a
        // parse warning for the Ready JSON but keeps running. This is acceptable
        // for Layer 3 — Layer 4 can split the handshake into a dedicated first-line
        // read on the thread before starting the draw-command loop.
        // TODO(layer-5): proper Ready handshake — read first line synchronously
        //   before starting the draw-command loop to capture sdk + features_used.
        //   Currently the Ready JSON arrives in the draw-command channel and logs
        //   a parse warning (harmless). Fix requires splitting the stdout reader
        //   into a handshake phase and a draw-command phase.

        for cmd in new_cmds {
            match cmd {
                DrawCommand::FrameDone { frame_id: done_id } => {
                    if done_id != frame_id {
                        log::warn!(
                            "ProcessApp[{}]: FrameDone frame_id={done_id} expected={frame_id}",
                            self.type_id
                        );
                    }
                    // Commit: swap pending into frame, reset pending for next frame.
                    std::mem::swap(&mut self.frame, &mut self.pending_frame);
                    self.pending_frame.clear();
                }
                DrawCommand::RunInTerminal { command } => {
                    self.pending_commands.push(crate::app_trait::AppCommand::RunInTerminal(command));
                }
                DrawCommand::Cd { path } => {
                    self.pending_commands.push(crate::app_trait::AppCommand::Cd(std::path::PathBuf::from(path)));
                }
                DrawCommand::Log { level, message } => {
                    let target = format!("app::{}", self.type_id);
                    match level.as_str() {
                        "error" => log::error!(target: &target, "{message}"),
                        "warn"  => log::warn!(target: &target, "{message}"),
                        "debug" => log::debug!(target: &target, "{message}"),
                        _       => log::info!(target: &target, "{message}"),
                    }
                }
                // v3 out-of-frame control commands — route to subsystems.
                cmd @ (DrawCommand::VideoPlayer { .. }
                    | DrawCommand::AudioPlay { .. }
                    | DrawCommand::AudioCapture { .. }
                    | DrawCommand::AudioMeter { .. }
                    | DrawCommand::CapabilityRequest { .. }
                    | DrawCommand::SecretGet { .. }
                    | DrawCommand::RunGet { .. }
                    | DrawCommand::RunComplete { .. }
                    | DrawCommand::Notify { .. }
                    | DrawCommand::PipeOpen { .. }
                    | DrawCommand::PipeSend { .. }
                    | DrawCommand::StatusSummary { .. }) => {
                    self.route_command(cmd);
                }
                other => self.pending_frame.push(other),
            }
        }

        // Handle any pending prompts — show real egui modal with Grant/Deny buttons.
        if !self.pending_prompts.is_empty() {
            let mut pending_prompts = std::mem::take(&mut self.pending_prompts);
            let mut outbound_events = std::mem::take(&mut self.outbound_events);
            let mut permissions = std::mem::take(&mut self.permissions);
            let mut secret_input_buf = std::mem::take(&mut self.secret_input_buf);
            let type_id = self.type_id.clone();
            let workspace_root = self.workspace_root.clone();
            Self::show_prompt_modal(ui, &mut pending_prompts, &mut outbound_events, &mut permissions, &type_id, &workspace_root, &mut secret_input_buf);
            self.pending_prompts = pending_prompts;
            self.outbound_events = outbound_events;
            self.permissions = permissions;
            self.secret_input_buf = secret_input_buf;
        }

        // Upload + render any pending video frames (requires egui ctx).
        let pending_frames = std::mem::take(&mut self.pending_video_frames);
        for (handle_id, frame, vx, vy, vw, vh) in pending_frames {
            let color_image = egui::ColorImage::from_rgba_unmultiplied(
                [frame.width as usize, frame.height as usize],
                &frame.rgba,
            );
            let tex_name = format!("video_frame_{handle_id}_{}", frame.pts_ms);
            let tex = ui.ctx().load_texture(
                &tex_name,
                color_image,
                egui::TextureOptions::LINEAR,
            );
            // Render at the requested rect.
            let origin = ui.min_rect().min;
            let rect = egui::Rect::from_min_size(
                egui::pos2(origin.x + vx, origin.y + vy),
                egui::vec2(vw, vh),
            );
            ui.painter().image(
                tex.id(),
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
            self.video_textures.insert(handle_id, tex);
        }

        // Render the current frame.
        let frame_clone = self.frame.clone();
        let meters: Vec<_> = self.audio_meters.iter().map(|m| AudioMeterState {
            rect_x: m.rect_x, rect_y: m.rect_y, rect_w: m.rect_w, rect_h: m.rect_h,
            pipe_id: m.pipe_id.clone(),
        }).collect();
        egui::Frame::new()
            .fill(ctx.colors.terminal_bg)
            .show(ui, |ui| {
                Self::render_draw_commands(ui, &frame_clone, ctx.colors, &meters);
            });

        // Poll the subprocess at ~60 fps.
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
    }

    fn handle_key(&mut self, input: &egui::InputState) -> bool {
        let mut consumed = false;
        for event in &input.events {
            match event {
                egui::Event::Key { key, pressed: true, modifiers, .. } => {
                    let is_bare_letter = matches!(key,
                        egui::Key::A | egui::Key::B | egui::Key::C | egui::Key::D |
                        egui::Key::E | egui::Key::F | egui::Key::G | egui::Key::H |
                        egui::Key::I | egui::Key::J | egui::Key::K | egui::Key::L |
                        egui::Key::M | egui::Key::N | egui::Key::O | egui::Key::P |
                        egui::Key::Q | egui::Key::R | egui::Key::S | egui::Key::T |
                        egui::Key::U | egui::Key::V | egui::Key::W | egui::Key::X |
                        egui::Key::Y | egui::Key::Z
                    ) && !modifiers.any();
                    if !is_bare_letter {
                        self.send_event(&PlexiEvent::Key {
                            key: format!("{key:?}"),
                            modifiers: Modifiers {
                                shift: modifiers.shift,
                                ctrl: modifiers.ctrl,
                                alt: modifiers.alt,
                                cmd: modifiers.command,
                            },
                        });
                    }
                    consumed = true;
                }
                egui::Event::Text(text) => {
                    for ch in text.chars() {
                        if ch.is_control() {
                            continue;
                        }
                        self.send_event(&PlexiEvent::Key {
                            key: ch.to_string(),
                            modifiers: Modifiers::default(),
                        });
                    }
                    consumed = true;
                }
                _ => {}
            }
        }
        consumed
    }

    fn take_pending_commands(&mut self) -> Vec<AppCommand> {
        std::mem::take(&mut self.pending_commands)
    }

    fn on_command(&mut self, cmd: &str) -> Option<AppCommand> {
        self.send_event(&PlexiEvent::Command { text: cmd.to_string() });
        None
    }

    fn serialize_state(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type_id": self.type_id,
        }))
    }
}

impl Drop for ProcessApp {
    fn drop(&mut self) {
        self.send_event(&PlexiEvent::Shutdown);
        event_log::emit(HostEvent::AppClosed {
            app_id: self.type_id.clone(),
            type_id: self.type_id.clone(),
            pane_id: 0,
            reason: None,
            timestamp: event_log::now_timestamp(),
        });
        if let Some(mut child) = self.process.take() {
            let _ = child.wait();
            let _ = child.kill();
        }
    }
}

/// Parse a hex color string like `"#1e1e2e"` or `"#cdd6f4"` into Color32.
fn parse_color(hex: &str) -> Option<Color32> {
    let hex = hex.trim_start_matches('#');
    if hex.len() == 6 {
        let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
        Some(Color32::from_rgb(r, g, b))
    } else if hex.len() == 8 {
        let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
        let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
        Some(Color32::from_rgba_premultiplied(r, g, b, a))
    } else {
        None
    }
}
