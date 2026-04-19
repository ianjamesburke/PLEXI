//! ProcessApp — runs an external app binary as a subprocess and renders it
//! using the Plexi draw protocol.
//!
//! The subprocess speaks the app protocol over stdin/stdout (newline-delimited JSON).
//! ProcessApp implements the `App` trait so it drops in wherever a built-in app
//! would — the rest of Plexi doesn't know or care that it's an external process.
//!
//! Internal layout:
//! - `mod.rs`      — struct, lifecycle (launch/drop), App trait impl
//! - `routing.rs`  — `route_command()`: dispatch DrawCommands to subsystems
//! - `render.rs`   — `render_draw_commands()`: paint committed frames into egui
//! - `prompts.rs`  — `show_prompt_modal()`: capability/secret grant UI

mod prompts;
mod render;
mod routing;

use crate::app_permissions::{AppPermissions, Capability, PermissionsLog};
use crate::app_protocol::{DrawCommand, Modifiers, PlexiEvent};
use crate::app_trait::{App, AppCommand, AppRenderContext};
use crate::event_log::{self, HostEvent};
use crate::runs::RunRegistry;
use crate::typed_pipes::TypedPipeRegistry;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Stdio};
use std::sync::{
    mpsc::{self, Receiver, TryRecvError},
    Arc, Mutex,
};
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
// ProcessApp
// ---------------------------------------------------------------------------

pub struct ProcessApp {
    pub(crate) type_id: String,
    display_name: String,
    accepted_exts: Vec<String>,
    process: Option<Child>,
    pub(crate) stdin: Option<ChildStdin>,
    /// Receives draw commands from the subprocess on a background thread.
    draw_rx: Option<Receiver<DrawCommand>>,
    /// The last fully committed frame (commands between two FrameDones).
    pub(crate) frame: Vec<DrawCommand>,
    /// Accumulates draw commands for the frame currently being received.
    pending_frame: Vec<DrawCommand>,
    /// Pending host app commands collected from the subprocess.
    pub(crate) pending_commands: Vec<AppCommand>,
    last_size: egui::Vec2,
    initialized: bool,
    frame_counter: u64,
    sdk: Option<String>,
    features_used: Vec<String>,
    /// workspace_root sent in Init — scopes all SecretGet calls.
    pub(crate) workspace_root: PathBuf,
    /// Granted capabilities for this app instance.
    pub(crate) permissions: AppPermissions,
    /// Typed pipe registry.
    pub(crate) pipe_registry: Arc<Mutex<TypedPipeRegistry>>,
    pub(crate) run_registry: RunRegistry,
    pub(crate) pending_prompts: VecDeque<PendingPrompt>,
    pub(crate) status_summary: Option<String>,
    pub(crate) outbound_events: VecDeque<PlexiEvent>,
    pub(crate) secret_input_buf: String,
    keyboard_capture: bool,
}

impl ProcessApp {
    /// Spawn an app binary at `bin_path`.
    ///
    /// `workspace_root` must be an absolute existing directory — validated here.
    #[allow(clippy::too_many_arguments)]
    pub fn launch(
        type_id: impl Into<String>,
        display_name: impl Into<String>,
        accepted_exts: Vec<String>,
        bin_path: &PathBuf,
        cwd: &PathBuf,
        args: &[String],
        workspace_root: PathBuf,
        capabilities: std::collections::HashSet<Capability>,
        keyboard_capture: bool,
    ) -> Result<Self, std::io::Error> {
        let type_id: String = type_id.into();
        let display_name: String = display_name.into();

        if workspace_root.as_os_str().is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "workspace_root must be non-empty",
            ));
        }
        if !workspace_root.is_absolute() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "workspace_root must be absolute: {}",
                    workspace_root.display()
                ),
            ));
        }
        if !workspace_root.is_dir() {
            log::warn!(
                "ProcessApp: workspace_root '{}' does not exist yet; proceeding",
                workspace_root.display()
            );
        }

        // STEP-9: environment isolation (spec invariant I-6).
        // Clear the inherited environment and whitelist only vars the app
        // legitimately needs. Strips ANTHROPIC_API_KEY and every other
        // host credential — apps must go through the secret broker.
        const ENV_WHITELIST: &[&str] = &["HOME", "PATH", "LANG", "LC_ALL", "TERM", "USER", "SHELL"];
        let mut cmd = std::process::Command::new(bin_path);
        cmd.args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_clear();
        for var in ENV_WHITELIST {
            if let Ok(v) = std::env::var(var) {
                cmd.env(var, v);
            }
        }
        // Pass through every PLEXI_* var (harness knobs, mock-device selectors).
        for (k, v) in std::env::vars() {
            if k.starts_with("PLEXI_") {
                cmd.env(k, v);
            }
        }
        let mut child = cmd.spawn()?;

        let stdin = child.stdin.take().expect("stdin piped");
        let stdout: ChildStdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");

        // Background thread: forward subprocess stderr to Plexi's logger.
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
        let (draw_tx, draw_rx) = mpsc::channel::<DrawCommand>();
        thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                match line {
                    Ok(l) if !l.trim().is_empty() => {
                        match serde_json::from_str::<DrawCommand>(&l) {
                            Ok(cmd) => {
                                if draw_tx.send(cmd).is_err() {
                                    break;
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
            outbound_events: VecDeque::new(),
            secret_input_buf: String::new(),
            keyboard_capture,
        })
    }

    /// Spawn with minimal args — workspace_root defaults to cwd.
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
            false,
        )
    }

    pub fn status_summary(&self) -> Option<&str> {
        self.status_summary.as_deref()
    }

    pub fn list_runs(&self) -> Vec<&crate::runs::Run> {
        self.run_registry.list_runs()
    }

    pub fn drain_pending_prompts(&mut self) -> Option<PendingPrompt> {
        self.pending_prompts.pop_front()
    }

    pub fn resolve_capability(
        &mut self,
        request_id: &str,
        capability_str: &str,
        granted: bool,
        perms_log: &mut PermissionsLog,
    ) {
        match Capability::try_from(capability_str) {
            Ok(cap) => {
                perms_log.record(&self.type_id, &self.workspace_root, cap, granted);
                if granted {
                    self.permissions.capabilities.insert(cap);
                }
            }
            Err(e) => {
                log::warn!("ProcessApp[{}]: resolve_capability: {e}", self.type_id);
            }
        }
        self.outbound_events
            .push_back(PlexiEvent::CapabilityDecision {
                request_id: request_id.to_string(),
                granted,
            });
    }

    pub fn resolve_secret(&mut self, key: &str, value: Option<String>) {
        self.outbound_events.push_back(PlexiEvent::SecretValue {
            key: key.to_string(),
            value,
        });
    }

    pub(crate) fn send_event(&mut self, event: &PlexiEvent) {
        let Some(stdin) = self.stdin.as_mut() else {
            return;
        };
        match serde_json::to_string(event) {
            Ok(mut line) => {
                line.push('\n');
                if let Err(e) = stdin.write_all(line.as_bytes()) {
                    log::warn!("ProcessApp: failed to write event: {e}");
                    self.stdin = None;
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
}

impl App for ProcessApp {
    fn type_id(&self) -> &'static str {
        Box::leak(self.type_id.clone().into_boxed_str())
    }

    fn display_name(&self) -> String {
        self.display_name.clone()
    }

    fn keyboard_capture(&self) -> bool {
        self.keyboard_capture
    }

    fn queue_outbound_event(&mut self, event: crate::app_protocol::PlexiEvent) {
        self.outbound_events.push_back(event);
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &AppRenderContext<'_>) {
        let size = ui.available_size();

        self.flush_outbound_events();

        if !self.initialized {
            self.initialized = true;
            self.last_size = size;
            let cap_strings: Vec<String> = self
                .permissions
                .capabilities
                .iter()
                .map(|c| c.to_string())
                .collect();
            self.send_event(&PlexiEvent::Init {
                protocol: "pgap/3".to_string(),
                app_id: self.type_id.clone(),
                workspace_root: self.workspace_root.clone(),
                capabilities: cap_strings,
                feature_flags: vec!["pane_groups_v1".into()],
            });
        }

        if (size - self.last_size).length() > 1.0 {
            self.last_size = size;
            self.send_event(&PlexiEvent::Resize {
                width: size.x,
                height: size.y,
            });
        }

        self.frame_counter += 1;
        let frame_id = self.frame_counter;
        self.send_event(&PlexiEvent::Render {
            frame_id,
            rect: crate::app_protocol::Rect {
                x: 0.0,
                y: 0.0,
                w: size.x,
                h: size.y,
            },
        });

        let new_cmds = self.drain_draw_commands();

        // TODO(layer-5): proper Ready handshake — read first line synchronously
        // before starting the draw-command loop to capture sdk + features_used.

        for cmd in new_cmds {
            match cmd {
                DrawCommand::FrameDone { frame_id: done_id } => {
                    if done_id != frame_id {
                        log::warn!(
                            "ProcessApp[{}]: FrameDone frame_id={done_id} expected={frame_id}",
                            self.type_id
                        );
                    }
                    std::mem::swap(&mut self.frame, &mut self.pending_frame);
                    self.pending_frame.clear();
                }
                DrawCommand::Log { level, message } => {
                    let target = format!("app::{}", self.type_id);
                    match level.as_str() {
                        "error" => log::error!(target: &target, "{message}"),
                        "warn" => log::warn!(target: &target, "{message}"),
                        "debug" => log::debug!(target: &target, "{message}"),
                        _ => log::info!(target: &target, "{message}"),
                    }
                }
                cmd @ (DrawCommand::CapabilityRequest { .. }
                | DrawCommand::SecretGet { .. }
                | DrawCommand::RunGet { .. }
                | DrawCommand::RunComplete { .. }
                | DrawCommand::Notify { .. }
                | DrawCommand::PipeOpen { .. }
                | DrawCommand::PipeSend { .. }
                | DrawCommand::StatusSummary { .. }
                | DrawCommand::SpawnApp { .. }
                | DrawCommand::HttpRequest { .. }
                | DrawCommand::AudioPlay { .. }
                | DrawCommand::AudioCapture { .. }) => {
                    self.route_command(cmd);
                }
                other => self.pending_frame.push(other),
            }
        }

        if !self.pending_prompts.is_empty() {
            let mut pending_prompts = std::mem::take(&mut self.pending_prompts);
            let mut outbound_events = std::mem::take(&mut self.outbound_events);
            let mut permissions = std::mem::take(&mut self.permissions);
            let mut secret_input_buf = std::mem::take(&mut self.secret_input_buf);
            let type_id = self.type_id.clone();
            let workspace_root = self.workspace_root.clone();
            prompts::show_prompt_modal(
                ui,
                &mut pending_prompts,
                &mut outbound_events,
                &mut permissions,
                &type_id,
                &workspace_root,
                &mut secret_input_buf,
            );
            self.pending_prompts = pending_prompts;
            self.outbound_events = outbound_events;
            self.permissions = permissions;
            self.secret_input_buf = secret_input_buf;
        }

        // Render the current committed frame.
        let frame_clone = self.frame.clone();
        egui::Frame::new()
            .fill(ctx.colors.terminal_bg)
            .show(ui, |ui| {
                render::render_draw_commands(ui, &frame_clone, ctx.colors);
            });

        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));
    }

    fn handle_key(&mut self, input: &egui::InputState) -> bool {
        let mut consumed = false;
        for event in &input.events {
            match event {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    let is_bare_letter = matches!(
                        key,
                        egui::Key::A
                            | egui::Key::B
                            | egui::Key::C
                            | egui::Key::D
                            | egui::Key::E
                            | egui::Key::F
                            | egui::Key::G
                            | egui::Key::H
                            | egui::Key::I
                            | egui::Key::J
                            | egui::Key::K
                            | egui::Key::L
                            | egui::Key::M
                            | egui::Key::N
                            | egui::Key::O
                            | egui::Key::P
                            | egui::Key::Q
                            | egui::Key::R
                            | egui::Key::S
                            | egui::Key::T
                            | egui::Key::U
                            | egui::Key::V
                            | egui::Key::W
                            | egui::Key::X
                            | egui::Key::Y
                            | egui::Key::Z
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

    fn sync_cwd(&mut self, new_cwd: &std::path::Path) {
        self.outbound_events.push_back(PlexiEvent::PathChanged {
            cwd: new_cwd.to_path_buf(),
        });
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
