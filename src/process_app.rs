/// ProcessApp — runs an external app binary as a subprocess and renders it
/// using the Plexi draw protocol.
///
/// The subprocess speaks the app protocol over stdin/stdout (newline-delimited JSON).
/// ProcessApp implements the `App` trait so it drops in wherever a built-in app
/// would — the rest of Plexi doesn't know or care that it's an external process.

use crate::app_protocol::{DrawCommand, ListItem, Modifiers, PendingSpawn, PlexiEvent};
use crate::app_trait::{App, AppCommand, AppRenderContext};
use crate::cost_tracker::CostTracker;
use egui::Color32;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Maximum number of undo states to keep in the stack.
const MAX_UNDO_DEPTH: usize = 50;

pub struct ProcessApp {
    type_id: String,
    display_name: String,
    accepted_exts: Vec<String>,
    /// Directory the app was launched in — used as the scope root for secret
    /// resolution (walk-up search from this directory to $HOME).
    scope_root: PathBuf,
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
    /// Pending `spawn_app` requests collected from the subprocess, to be
    /// drained by the host via take_pending_spawns() each frame.
    pending_spawns: Vec<PendingSpawn>,
    /// Pending PipeWrite commands collected from the subprocess, to be drained
    /// by the host's pipe dispatcher via take_pipe_writes() each frame.
    pending_pipe_writes: Vec<(String, serde_json::Value)>,
    /// Size last sent to the subprocess.
    last_size: egui::Vec2,
    initialized: bool,
    /// Protocol version from the app manifest.
    protocol_version: u32,
    /// Open intent passed at spawn time.
    open_intent: Option<crate::app_protocol::OpenIntent>,
    /// Pane id this app is running in (used for bus events).
    pub pane_id: u64,
    /// Shared event log for bus emission.
    event_log: Option<Arc<crate::event_log::EventLog>>,
    /// Shared run store.
    run_store: Option<Arc<Mutex<crate::run_store::RunStore>>>,
    /// Sender used to write PlexiEvents back to this app (for async responses).
    event_back_tx: Option<mpsc::SyncSender<PlexiEvent>>,
    /// Pending events to send back to the app (RunCreated, EventData, etc.)
    pending_back_events: Vec<PlexiEvent>,
    // Hot reload state
    bin_path: PathBuf,
    cwd: PathBuf,
    args: Vec<String>,
    /// Receives file-change notifications from the watcher thread.
    reload_rx: Option<Receiver<()>>,
    /// Kept alive so the watcher thread doesn't stop.
    _watcher: Option<RecommendedWatcher>,
    /// Debounce: ignore reload signals within 200ms of the last reload.
    last_reload: Instant,
    /// Last known app state (from State draw command response).
    last_state: Option<AppState>,
    /// Undo stack — previous states, oldest first.
    undo_stack: Vec<AppState>,
    /// Redo stack — states popped by undo, newest first.
    redo_stack: Vec<AppState>,
    /// Whether we're waiting for a state snapshot (to push onto undo before restoring).
    pending_undo: bool,
    /// Whether we're waiting for a state snapshot (to push onto redo before restoring).
    pending_redo: bool,
    /// Per-app cost tracker for LLM API usage.
    cost_tracker: CostTracker,
    /// Timestamp of the last Render event sent, used to compute delta_time.
    last_render_time: Instant,
    /// Whether this app should receive MouseMove events (opt-in via manifest or DrawCommand).
    mouse_tracking: bool,
    /// Pending cursor icon override requested by the app this frame.
    pending_cursor: Option<egui::CursorIcon>,
    /// If this app was spawned by another app, the spawning app's type_id.
    /// Re-injected as PLEXI_PARENT_APP_ID on hot-reload so the app always
    /// knows it was spawned, not opened directly.
    parent_app_id: Option<String>,
    /// Transform stack for PushTransform/PopTransform. Each entry: (scale_x, scale_y, tx, ty, rotate, ox, oy).
    transform_stack: Vec<(f32, f32, f32, f32, f32, f32, f32)>,
}

/// Snapshot of an app's state buckets.
#[derive(Clone, Debug)]
pub struct AppState {
    pub user_state: serde_json::Value,
    pub derived: serde_json::Value,
    pub session: serde_json::Value,
    pub persistent: serde_json::Value,
}

/// Probe well-known locations for a Python interpreter that is >= 3.10.
///
/// macOS GUI app bundles do not inherit the user's shell PATH, so
/// `/usr/bin/env python3` resolves to Apple's frozen system Python (3.9).
/// We probe explicit paths — first match that reports version >= 3.10 wins.
/// Falls back to `python3` (shebang-resolved) if none of the known paths work,
/// so the old behaviour is preserved rather than failing silently.
fn find_python() -> std::ffi::OsString {
    let candidates = [
        "/opt/homebrew/bin/python3",   // Apple Silicon Homebrew
        "/usr/local/bin/python3",      // Intel Homebrew
        "/opt/homebrew/bin/python3.13",
        "/opt/homebrew/bin/python3.12",
        "/opt/homebrew/bin/python3.11",
        "/opt/homebrew/bin/python3.10",
        "/usr/local/bin/python3.13",
        "/usr/local/bin/python3.12",
        "/usr/local/bin/python3.11",
        "/usr/local/bin/python3.10",
    ];

    for candidate in &candidates {
        let path = std::path::Path::new(candidate);
        if !path.exists() {
            continue;
        }
        let ok = std::process::Command::new(candidate)
            .args(["-c", "import sys; v=sys.version_info; exit(0 if v>=(3,10) else 1)"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            log::debug!("process_app: using Python interpreter: {candidate}");
            return std::ffi::OsString::from(candidate);
        }
    }

    log::warn!(
        "process_app: no Python >= 3.10 found at known paths; \
         falling back to `python3` (may be system 3.9 in GUI context). \
         Install Python 3.10+ via: brew install python@3.13"
    );
    std::ffi::OsString::from("python3")
}

impl ProcessApp {
    /// Spawn an app binary at `bin_path`.
    ///
    /// `parent_app_id` — when `Some`, the app was spawned by another app rather
    /// than opened directly. Two extra env vars are injected:
    /// - `PLEXI_LAUNCH_MODE=spawned`
    /// - `PLEXI_PARENT_APP_ID=<parent_app_id>`
    /// Apps read these to branch on standalone-vs-embedded behaviour.
    pub fn launch(
        type_id: impl Into<String>,
        display_name: impl Into<String>,
        accepted_exts: Vec<String>,
        bin_path: &PathBuf,
        cwd: &PathBuf,
        args: &[String],
        mouse_tracking: bool,
        parent_app_id: Option<&str>,
    ) -> Result<Self, std::io::Error> {
        Self::launch_with_intent(type_id, display_name, accepted_exts, bin_path, cwd, args, None, 1, 0, mouse_tracking, parent_app_id)
    }

    /// Spawn with an OpenIntent, protocol version, pane_id, mouse_tracking, and optional parent.
    pub fn launch_with_intent(
        type_id: impl Into<String>,
        display_name: impl Into<String>,
        accepted_exts: Vec<String>,
        bin_path: &PathBuf,
        cwd: &PathBuf,
        args: &[String],
        open_intent: Option<crate::app_protocol::OpenIntent>,
        protocol_version: u32,
        pane_id: u64,
        mouse_tracking: bool,
        parent_app_id: Option<&str>,
    ) -> Result<Self, std::io::Error> {
        let type_id: String = type_id.into();
        let display_name: String = display_name.into();

        // For Python scripts, bypass the shebang and use an explicit interpreter
        // so macOS GUI bundles (which don't inherit shell PATH) always get >= 3.10.
        let (cmd, extra_args): (std::ffi::OsString, Vec<std::ffi::OsString>) =
            if bin_path.extension().and_then(|e| e.to_str()) == Some("py") {
                (find_python(), vec![bin_path.as_os_str().to_owned()])
            } else {
                (bin_path.as_os_str().to_owned(), vec![])
            };

        let mut cmd_builder = std::process::Command::new(&cmd);
        cmd_builder
            .args(&extra_args)
            .args(args)
            .current_dir(cwd)
            .env("PLEXI_APP_ID", &type_id)
            .env("PLEXI_APPS_DIR", crate::app_registry::apps_dir().as_os_str());
        if let Some(parent_id) = parent_app_id {
            cmd_builder
                .env("PLEXI_LAUNCH_MODE", "spawned")
                .env("PLEXI_PARENT_APP_ID", parent_id);
        } else {
            cmd_builder.env("PLEXI_LAUNCH_MODE", "direct");
        }
        // Create a new process group so we can reap the entire subtree on shutdown.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd_builder.process_group(0);
        }
        let mut child = cmd_builder
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

        // Set up file watcher on the app's parent directory for hot reload.
        let (reload_tx, reload_rx) = mpsc::channel::<()>();
        let watch_dir = bin_path.parent().unwrap_or(cwd).to_path_buf();
        let watcher = Self::setup_watcher(watch_dir, reload_tx);

        let cost_tracker = CostTracker::new(&type_id);
        Ok(Self {
            type_id,
            display_name,
            accepted_exts,
            scope_root: cwd.clone(),
            process: Some(child),
            stdin: Some(stdin),
            draw_rx: Some(draw_rx),
            frame: Vec::new(),
            pending_frame: Vec::new(),
            pending_commands: Vec::new(),
            pending_spawns: Vec::new(),
            pending_pipe_writes: Vec::new(),
            last_size: egui::Vec2::ZERO,
            initialized: false,
            protocol_version,
            open_intent,
            pane_id,
            event_log: None,
            run_store: None,
            event_back_tx: None,
            pending_back_events: Vec::new(),
            bin_path: bin_path.clone(),
            cwd: cwd.clone(),
            args: args.to_vec(),
            reload_rx: Some(reload_rx),
            _watcher: watcher,
            last_reload: Instant::now(),
            last_state: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            pending_undo: false,
            pending_redo: false,
            cost_tracker,
            last_render_time: Instant::now(),
            mouse_tracking,
            pending_cursor: None,
            parent_app_id: parent_app_id.map(|s| s.to_string()),
            transform_stack: Vec::new(),
        })
    }

    /// Wire shared infrastructure into this app after construction.
    pub fn wire(
        &mut self,
        event_log: Arc<crate::event_log::EventLog>,
        run_store: Arc<Mutex<crate::run_store::RunStore>>,
    ) {
        self.event_log = Some(event_log);
        self.run_store = Some(run_store);
    }

    /// Create a file watcher that sends a signal on any .py file change in the directory.
    fn setup_watcher(watch_dir: PathBuf, reload_tx: mpsc::Sender<()>) -> Option<RecommendedWatcher> {
        let mut watcher = match notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
            if let Ok(event) = res {
                // Only trigger on file modifications/creates for .py files.
                let dominated_by_python = event.paths.iter().any(|p| {
                    p.extension()
                        .map(|e| e == "py")
                        .unwrap_or(false)
                });
                if dominated_by_python {
                    let _ = reload_tx.send(());
                }
            }
        }) {
            Ok(w) => w,
            Err(e) => {
                log::warn!("ProcessApp: failed to create file watcher: {e}");
                return None;
            }
        };

        if let Err(e) = watcher.watch(&watch_dir, RecursiveMode::Recursive) {
            log::warn!("ProcessApp: failed to watch {:?}: {e}", watch_dir);
            return None;
        }

        log::info!("ProcessApp: watching {:?} for hot reload", watch_dir);
        Some(watcher)
    }

    /// Check if a reload was requested and debounce.
    fn check_reload(&mut self) -> bool {
        let Some(rx) = self.reload_rx.as_ref() else {
            return false;
        };
        let mut got_signal = false;
        // Drain all pending signals.
        while rx.try_recv().is_ok() {
            got_signal = true;
        }
        if got_signal && self.last_reload.elapsed() > Duration::from_millis(200) {
            self.last_reload = Instant::now();
            true
        } else {
            false
        }
    }

    /// Kill the current subprocess and respawn it, preserving the watcher.
    fn restart(&mut self) {
        log::info!("ProcessApp[{}]: hot-reloading app", self.type_id);

        // Send shutdown and kill old process. Close stdin/draw_rx first so
        // the child sees EOF and the reader threads exit cleanly before we reap.
        self.send_event(&PlexiEvent::Shutdown);
        self.stdin = None;
        self.draw_rx = None;
        if let Some(mut child) = self.process.take() {
            let _ = child.kill();
            let _ = child.wait();
        }

        // Respawn — same Python interpreter logic as initial launch.
        let (cmd, extra_args): (std::ffi::OsString, Vec<std::ffi::OsString>) =
            if self.bin_path.extension().and_then(|e| e.to_str()) == Some("py") {
                (find_python(), vec![self.bin_path.as_os_str().to_owned()])
            } else {
                (self.bin_path.as_os_str().to_owned(), vec![])
            };
        let mut cmd_builder = std::process::Command::new(&cmd);
        cmd_builder
            .args(&extra_args)
            .args(&self.args)
            .current_dir(&self.cwd)
            .env("PLEXI_APP_ID", &self.type_id)
            .env("PLEXI_APPS_DIR", crate::app_registry::apps_dir().as_os_str());
        if let Some(parent_id) = &self.parent_app_id {
            cmd_builder
                .env("PLEXI_LAUNCH_MODE", "spawned")
                .env("PLEXI_PARENT_APP_ID", parent_id);
        } else {
            cmd_builder.env("PLEXI_LAUNCH_MODE", "direct");
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd_builder.process_group(0);
        }
        let mut child = match cmd_builder
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                log::error!("ProcessApp[{}]: failed to respawn: {e}", self.type_id);
                return;
            }
        };

        let stdin = child.stdin.take().expect("stdin piped");
        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");

        // Stderr forwarding thread.
        let stderr_type_id = self.type_id.clone();
        thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
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

        // Stdout draw-command reader thread.
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

        self.process = Some(child);
        self.stdin = Some(stdin);
        self.draw_rx = Some(draw_rx);
        self.frame.clear();
        self.pending_frame.clear();
        self.initialized = false; // will re-send Init on next ui() call
        self.last_render_time = Instant::now();
        self.mouse_tracking = false; // new process must opt in again
        self.pending_cursor = None;
    }

    /// Request the app's current state snapshot.
    pub fn request_state(&mut self) {
        self.send_event(&PlexiEvent::GetState);
    }

    /// Restore a previously captured state to the app.
    pub fn restore_state(&mut self, state: &AppState) {
        self.send_event(&PlexiEvent::SetState {
            user_state: state.user_state.clone(),
            derived: state.derived.clone(),
            session: state.session.clone(),
            persistent: state.persistent.clone(),
        });
    }

    /// Trigger undo: request current state (will be pushed to redo), then pop undo stack.
    fn do_undo(&mut self) {
        if self.undo_stack.is_empty() {
            return;
        }
        self.pending_undo = true;
        self.request_state();
    }

    /// Trigger redo: request current state (will be pushed to undo), then pop redo stack.
    fn do_redo(&mut self) {
        if self.redo_stack.is_empty() {
            return;
        }
        self.pending_redo = true;
        self.request_state();
    }

    /// Push the current state onto the undo stack (called before user actions).
    fn push_undo(&mut self, state: AppState) {
        if self.undo_stack.len() >= MAX_UNDO_DEPTH {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(state);
        self.redo_stack.clear();
    }

    /// Handle a State response from the app — drives undo/redo state machine.
    fn handle_state_response(&mut self, state: AppState) {
        if self.pending_undo {
            self.pending_undo = false;
            // Push current state to redo, pop undo and restore.
            self.redo_stack.push(state);
            if let Some(prev) = self.undo_stack.pop() {
                self.restore_state(&prev);
                self.last_state = Some(prev);
            }
        } else if self.pending_redo {
            self.pending_redo = false;
            // Push current state to undo, pop redo and restore.
            self.undo_stack.push(state);
            if let Some(next) = self.redo_stack.pop() {
                self.restore_state(&next);
                self.last_state = Some(next);
            }
        } else {
            // Normal state snapshot — push to undo stack for future undo.
            if let Some(prev) = self.last_state.take() {
                self.push_undo(prev);
            }
            self.last_state = Some(state);
        }
    }

    /// Total cost accumulated by this app in the current session.
    pub fn session_cost_usd(&self) -> f64 {
        self.cost_tracker.session_total_usd()
    }

    fn send_event(&mut self, event: &PlexiEvent) {
        // Debug-mode event tracing. Render/Resize are excluded — they fire every
        // frame and would bury actual signal. Everything else (keys, clicks, init,
        // commands, pipe data) is logged so bugs can be reproduced from the log.
        if log::log_enabled!(log::Level::Debug) {
            match event {
                PlexiEvent::Render { .. } | PlexiEvent::Resize { .. } => {}
                PlexiEvent::Key { key, modifiers } => {
                    let mods = {
                        let mut parts = Vec::new();
                        if modifiers.shift { parts.push("Shift"); }
                        if modifiers.alt   { parts.push("Alt"); }
                        if modifiers.ctrl  { parts.push("Ctrl"); }
                        if parts.is_empty() { String::new() } else { format!("{}+", parts.join("+")) }
                    };
                    log::debug!("app[{}]: key {}{}", self.type_id, mods, key);
                }
                _ => {
                    log::debug!("app[{}]: event {:?}", self.type_id, event);
                }
            }
        }

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

    /// Drain all pending PipeWrite commands queued from the subprocess since
    /// the last call. Returns (channel, value) pairs for the host to route.
    pub fn take_pipe_writes(&mut self) -> Vec<(String, serde_json::Value)> {
        std::mem::take(&mut self.pending_pipe_writes)
    }

    /// Send a PipeData event to this app's subprocess, delivering a value that
    /// arrived from a connected app on the named channel.
    pub fn send_pipe_data(&mut self, from_app: &str, channel: &str, value: &serde_json::Value) {
        self.send_event(&PlexiEvent::PipeData {
            from_app: from_app.to_string(),
            channel: channel.to_string(),
            value: value.clone(),
        });
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

    fn render_draw_commands(
        ui: &mut egui::Ui,
        commands: &[DrawCommand],
        ctx: &AppRenderContext<'_>,
        app_cwd: &std::path::Path,
    ) {
        let origin = ui.min_rect().min;
        let colors = ctx.colors;
        // Transform stack: (scale_x, scale_y, tx, ty). Rotate not supported in v2.1.
        let mut transform_stack: Vec<(f32, f32, f32, f32)> = Vec::new();

        // Apply transform stack to a point.
        let apply_tx = |stack: &Vec<(f32, f32, f32, f32)>, x: f32, y: f32| -> (f32, f32) {
            let mut px = x;
            let mut py = y;
            for &(sx, sy, tx, ty) in stack {
                px = px * sx + tx;
                py = py * sy + ty;
            }
            (px, py)
        };

        for cmd in commands {
            match cmd {
                DrawCommand::PushTransform { scale_x, scale_y, translate_x, translate_y, rotate, .. } => {
                    if *rotate != 0.0 {
                        log::warn!("ProcessApp: rotation transforms not supported in v2.1 (rotate={rotate}), skipping");
                    }
                    transform_stack.push((*scale_x, *scale_y, *translate_x, *translate_y));
                    continue;
                }
                DrawCommand::PopTransform => {
                    if transform_stack.pop().is_none() {
                        log::warn!("ProcessApp: PopTransform called on empty transform stack");
                    }
                    continue;
                }
                // MeasureText is resolved before rendering — skip here.
                DrawCommand::MeasureText { .. } => continue,
                _ => {}
            }

            match cmd {
                DrawCommand::Rect { x, y, w, h, fill, radius } => {
                    let (tx, ty) = apply_tx(&transform_stack, *x, *y);
                    // Scale w/h by the cumulative product of all scales in the stack.
                    let (cum_sx, cum_sy) = transform_stack.iter()
                        .fold((1.0_f32, 1.0_f32), |(ax, ay), &(sx, sy, _, _)| (ax * sx, ay * sy));
                    let (sw, sh) = (*w * cum_sx, *h * cum_sy);
                    let rect = egui::Rect::from_min_size(
                        egui::pos2(origin.x + tx, origin.y + ty),
                        egui::vec2(sw, sh),
                    );
                    let color = parse_color(fill).unwrap_or(colors.bg_active);
                    ui.painter().rect_filled(rect, *radius, color);
                }

                DrawCommand::Text { x, y, text, size, color, monospace, bold, align } => {
                    let color = parse_color(color).unwrap_or(colors.text_primary);
                    let font_id = if *monospace {
                        egui::FontId::monospace(*size)
                    } else if *bold {
                        egui::FontId::proportional(*size) // egui doesn't have a bold variant directly
                    } else {
                        egui::FontId::proportional(*size)
                    };
                    let (tx, ty) = apply_tx(&transform_stack, *x, *y);
                    let anchor = match align.as_deref() {
                        Some("center") => egui::Align2::CENTER_TOP,
                        Some("right") => egui::Align2::RIGHT_TOP,
                        _ => egui::Align2::LEFT_TOP,
                    };
                    ui.painter().text(
                        egui::pos2(origin.x + tx, origin.y + ty),
                        anchor,
                        text,
                        font_id,
                        color,
                    );
                }

                DrawCommand::Line { x1, y1, x2, y2, color, width } => {
                    let color = parse_color(color).unwrap_or(colors.bg_active);
                    let (tx1, ty1) = apply_tx(&transform_stack, *x1, *y1);
                    let (tx2, ty2) = apply_tx(&transform_stack, *x2, *y2);
                    ui.painter().line_segment(
                        [
                            egui::pos2(origin.x + tx1, origin.y + ty1),
                            egui::pos2(origin.x + tx2, origin.y + ty2),
                        ],
                        egui::Stroke::new(*width, color),
                    );
                }

                DrawCommand::DropTarget { x, y, w, h, label, .. } => {
                    // Drop targets are invisible by default. When files are being
                    // dragged from outside Plexi, draw a subtle highlight + label so
                    // the user can see where they can drop.
                    let rect = egui::Rect::from_min_size(
                        egui::pos2(origin.x + x, origin.y + y),
                        egui::vec2(*w, *h),
                    );
                    let hovering_with_files = ui.input(|i| !i.raw.hovered_files.is_empty());
                    if hovering_with_files {
                        // Subtle fill + outline to signal "droppable here".
                        let fill = Color32::from_rgba_unmultiplied(
                            colors.accent.r(),
                            colors.accent.g(),
                            colors.accent.b(),
                            28,
                        );
                        ui.painter().rect_filled(rect, 4.0, fill);
                        ui.painter().rect_stroke(
                            rect,
                            4.0,
                            egui::Stroke::new(1.5, colors.accent),
                            egui::StrokeKind::Inside,
                        );
                        if let Some(label_text) = label {
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                label_text,
                                egui::FontId::proportional(12.0),
                                colors.text_primary,
                            );
                        }
                    }
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

                DrawCommand::Image { path, x, y, w, h, fit, rounding } => {
                    let rect = egui::Rect::from_min_size(
                        egui::pos2(origin.x + x, origin.y + y),
                        egui::vec2(*w, *h),
                    );
                    let resolved = resolve_app_path(app_cwd, path);
                    let status = ctx.media_cache.borrow_mut().get_image(ui.ctx(), &resolved);
                    match status {
                        crate::media_cache::ImageStatus::Ready(tex) => {
                            paint_image(
                                ui.painter(),
                                &tex,
                                rect,
                                fit.as_deref().unwrap_or("contain"),
                                rounding.unwrap_or(0.0),
                                colors,
                            );
                        }
                        crate::media_cache::ImageStatus::Error(msg) => {
                            paint_error_placeholder(ui.painter(), rect, &msg, colors);
                        }
                    }
                }

                DrawCommand::VideoThumbnail {
                    path, x, y, w, h, show_play_button, timestamp_seconds,
                } => {
                    let rect = egui::Rect::from_min_size(
                        egui::pos2(origin.x + x, origin.y + y),
                        egui::vec2(*w, *h),
                    );
                    let resolved = resolve_app_path(app_cwd, path);
                    let ts = timestamp_seconds.unwrap_or(0.0);
                    let status = ctx
                        .media_cache
                        .borrow_mut()
                        .get_video_thumbnail(ui.ctx(), &resolved, ts);
                    match status {
                        crate::media_cache::ThumbStatus::Ready(tex) => {
                            paint_image(ui.painter(), &tex, rect, "cover", 0.0, colors);
                        }
                        crate::media_cache::ThumbStatus::Pending => {
                            paint_loading_placeholder(ui.painter(), rect, colors);
                        }
                        crate::media_cache::ThumbStatus::Error(msg) => {
                            paint_error_placeholder(ui.painter(), rect, &msg, colors);
                        }
                    }
                    if show_play_button.unwrap_or(true) {
                        paint_play_button(ui.painter(), rect);
                    }
                    // Click opens the original video in the system default player.
                    let response = ui.interact(
                        rect,
                        egui::Id::new(("plexi-video-thumb", path.as_str())),
                        egui::Sense::click(),
                    );
                    if response.clicked() {
                        log::info!("ProcessApp: opening video {:?}", resolved);
                        let _ = std::process::Command::new("open").arg(&resolved).spawn();
                    }
                }

                DrawCommand::FileGrid {
                    path, filter, paths, x, y, w, h,
                    item_size, columns, show_labels,
                } => {
                    let rect = egui::Rect::from_min_size(
                        egui::pos2(origin.x + x, origin.y + y),
                        egui::vec2(*w, *h),
                    );
                    let cell = item_size.unwrap_or(96.0).max(32.0);
                    let labels = show_labels.unwrap_or(true);
                    let label_h = if labels { 14.0 } else { 0.0 };
                    let gap = 8.0;

                    // Collect file paths from either explicit list or directory walk.
                    let files: Vec<PathBuf> = if let Some(list) = paths {
                        list.iter().map(|p| resolve_app_path(app_cwd, p)).collect()
                    } else if let Some(dir) = path {
                        list_dir_filtered(&resolve_app_path(app_cwd, dir), filter.as_deref())
                    } else {
                        Vec::new()
                    };

                    let cols = columns
                        .map(|c| c.max(1) as usize)
                        .unwrap_or_else(|| {
                            ((rect.width() + gap) / (cell + gap)).floor().max(1.0) as usize
                        });

                    // Paint items.
                    for (i, file) in files.iter().enumerate() {
                        let col = i % cols;
                        let row = i / cols;
                        let cell_x = rect.min.x + col as f32 * (cell + gap);
                        let cell_y = rect.min.y + row as f32 * (cell + label_h + gap);
                        if cell_y + cell + label_h > rect.max.y {
                            break; // clip anything past the grid rect
                        }
                        let thumb_rect = egui::Rect::from_min_size(
                            egui::pos2(cell_x, cell_y),
                            egui::vec2(cell, cell),
                        );

                        let kind = classify_file(file);
                        match kind {
                            FileKind::Image => {
                                let status = ctx.media_cache.borrow_mut().get_image(ui.ctx(), file);
                                match status {
                                    crate::media_cache::ImageStatus::Ready(tex) => {
                                        paint_image(ui.painter(), &tex, thumb_rect, "cover", 4.0, colors);
                                    }
                                    crate::media_cache::ImageStatus::Error(msg) => {
                                        paint_error_placeholder(ui.painter(), thumb_rect, &msg, colors);
                                    }
                                }
                            }
                            FileKind::Video => {
                                let status = ctx.media_cache.borrow_mut().get_video_thumbnail(
                                    ui.ctx(), file, 0.0,
                                );
                                match status {
                                    crate::media_cache::ThumbStatus::Ready(tex) => {
                                        paint_image(ui.painter(), &tex, thumb_rect, "cover", 4.0, colors);
                                    }
                                    crate::media_cache::ThumbStatus::Pending => {
                                        paint_loading_placeholder(ui.painter(), thumb_rect, colors);
                                    }
                                    crate::media_cache::ThumbStatus::Error(msg) => {
                                        paint_error_placeholder(ui.painter(), thumb_rect, &msg, colors);
                                    }
                                }
                                paint_play_button(ui.painter(), thumb_rect);
                            }
                            FileKind::Other => {
                                paint_generic_file_icon(ui.painter(), thumb_rect, file, colors);
                            }
                        }

                        if labels {
                            let name = file
                                .file_name()
                                .and_then(|s| s.to_str())
                                .unwrap_or("?");
                            let truncated = truncate_for_width(name, cell);
                            ui.painter().text(
                                egui::pos2(thumb_rect.center().x, thumb_rect.max.y + 2.0),
                                egui::Align2::CENTER_TOP,
                                truncated,
                                egui::FontId::proportional(10.0),
                                colors.text_dim,
                            );
                        }

                        // Click: open the file in the system default handler.
                        let response = ui.interact(
                            thumb_rect,
                            egui::Id::new(("plexi-file-grid", file.as_path())),
                            egui::Sense::click(),
                        );
                        if response.clicked() {
                            log::info!("ProcessApp: opening file {:?}", file);
                            let _ = std::process::Command::new("open").arg(file).spawn();
                        }
                    }
                }

                // RunInTerminal / Cd / Log / State / CostReport / Notification /
                // SetCursor / MouseTracking / SpawnApp / PipeWrite / PipeSubscribe /
                // SecretGet / RunCreate / RunUpdate / RunComplete / EventSubscribe /
                // PipeListWires / FrameDone handled at the App trait level, not here.
                DrawCommand::RunInTerminal { .. }
                | DrawCommand::Cd { .. }
                | DrawCommand::Log { .. }
                | DrawCommand::State { .. }
                | DrawCommand::CostReport { .. }
                | DrawCommand::Notification { .. }
                | DrawCommand::SetCursor { .. }
                | DrawCommand::MouseTracking { .. }
                | DrawCommand::StatusSummary { .. }
                | DrawCommand::SpawnApp { .. }
                | DrawCommand::PipeWrite { .. }
                | DrawCommand::PipeSubscribe { .. }
                | DrawCommand::SecretGet { .. }
                | DrawCommand::RunCreate { .. }
                | DrawCommand::RunUpdate { .. }
                | DrawCommand::RunComplete { .. }
                | DrawCommand::RunGet { .. }
                | DrawCommand::EventSubscribe { .. }
                | DrawCommand::PipeListWires
                | DrawCommand::PushTransform { .. }
                | DrawCommand::PopTransform
                | DrawCommand::MeasureText { .. }
                | DrawCommand::FrameDone => {}
            }
        }
    }
}

/// What type of thumbnail to show for a file in a FileGrid.
enum FileKind {
    Image,
    Video,
    Other,
}

fn classify_file(path: &std::path::Path) -> FileKind {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "webp" | "bmp" | "gif" | "tiff" | "tif" => FileKind::Image,
        "mp4" | "mov" | "webm" | "mkv" | "m4v" | "avi" => FileKind::Video,
        _ => FileKind::Other,
    }
}

/// Resolve a (possibly relative) path emitted by an app against the app's cwd.
/// Matches the subprocess's own view of the filesystem.
fn resolve_app_path(app_cwd: &std::path::Path, raw: &str) -> PathBuf {
    let p = PathBuf::from(raw);
    if p.is_absolute() {
        p
    } else {
        app_cwd.join(p)
    }
}

/// List files in a directory (non-recursive), optionally filtered by a set of
/// simple glob patterns or bare extensions. Returns paths sorted by filename.
fn list_dir_filtered(dir: &std::path::Path, filter: Option<&[String]>) -> Vec<PathBuf> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            log::warn!("ProcessApp: file_grid read_dir {:?}: {e}", dir);
            return Vec::new();
        }
    };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| filter.map_or(true, |f| path_matches_filter(p, f)))
        .collect();
    out.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    out
}

/// Match a path against a user-supplied filter list. Supports three forms:
///   - `"*.png"`    — extension wildcard
///   - `"png"`      — bare extension
///   - any other pattern is substring-matched against the filename
fn path_matches_filter(path: &std::path::Path, patterns: &[String]) -> bool {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_lowercase())
        .unwrap_or_default();
    for p in patterns {
        let p = p.trim();
        if let Some(rest) = p.strip_prefix("*.") {
            if ext == rest.to_lowercase() {
                return true;
            }
        } else if !p.contains('*') && !p.contains('.') {
            if ext == p.to_lowercase() {
                return true;
            }
        } else if name.contains(p.trim_start_matches('*').trim_end_matches('*')) {
            return true;
        }
    }
    false
}

/// Paint a texture into `rect` respecting the given fit mode.
fn paint_image(
    painter: &egui::Painter,
    tex: &egui::TextureHandle,
    rect: egui::Rect,
    fit: &str,
    rounding: f32,
    colors: &crate::theme::Colors,
) {
    let tex_size = tex.size_vec2();
    // Destination rect inside `rect`, computed per-fit.
    let (dest, uv) = match fit {
        "fill" => (rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0))),
        "cover" => {
            let src_aspect = tex_size.x / tex_size.y;
            let dst_aspect = rect.width() / rect.height();
            // Crop the source UV to match the destination aspect.
            let (u_min, u_max, v_min, v_max) = if src_aspect > dst_aspect {
                // source wider — crop horizontally
                let crop = dst_aspect / src_aspect;
                let half = (1.0 - crop) * 0.5;
                (half, 1.0 - half, 0.0, 1.0)
            } else {
                // source taller — crop vertically
                let crop = src_aspect / dst_aspect;
                let half = (1.0 - crop) * 0.5;
                (0.0, 1.0, half, 1.0 - half)
            };
            (
                rect,
                egui::Rect::from_min_max(
                    egui::pos2(u_min, v_min),
                    egui::pos2(u_max, v_max),
                ),
            )
        }
        _ => {
            // "contain" (default) — fit inside preserving aspect, letterbox.
            let src_aspect = tex_size.x / tex_size.y;
            let dst_aspect = rect.width() / rect.height();
            let inner = if src_aspect > dst_aspect {
                // fit to width
                let h = rect.width() / src_aspect;
                let y = rect.center().y - h * 0.5;
                egui::Rect::from_min_size(egui::pos2(rect.min.x, y), egui::vec2(rect.width(), h))
            } else {
                let w = rect.height() * src_aspect;
                let x = rect.center().x - w * 0.5;
                egui::Rect::from_min_size(egui::pos2(x, rect.min.y), egui::vec2(w, rect.height()))
            };
            // Fill the surrounding letterbox area with the terminal bg so the
            // image doesn't show whatever was painted under it.
            painter.rect_filled(rect, rounding, colors.terminal_bg);
            (inner, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)))
        }
    };

    let mut mesh = egui::Mesh::with_texture(tex.id());
    mesh.add_rect_with_uv(dest, uv, egui::Color32::WHITE);
    if rounding > 0.0 {
        // egui's mesh doesn't round natively — clip via a rounded rect by
        // drawing a rounded mask frame over the corners using the terminal bg.
        // This is a pragmatic approximation; a real mask would need a shader.
        painter.rect_filled(rect, rounding, egui::Color32::TRANSPARENT);
    }
    painter.add(egui::Shape::mesh(mesh));
}

fn paint_error_placeholder(
    painter: &egui::Painter,
    rect: egui::Rect,
    msg: &str,
    colors: &crate::theme::Colors,
) {
    painter.rect_filled(rect, 2.0, Color32::from_rgb(60, 30, 30));
    painter.rect_stroke(
        rect,
        2.0,
        egui::Stroke::new(1.0, Color32::from_rgb(180, 70, 70)),
        egui::StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "×",
        egui::FontId::proportional(rect.height().min(rect.width()) * 0.5),
        Color32::from_rgb(240, 160, 160),
    );
    log::debug!("ProcessApp: placeholder for {msg}");
    let _ = colors;
}

fn paint_loading_placeholder(
    painter: &egui::Painter,
    rect: egui::Rect,
    colors: &crate::theme::Colors,
) {
    painter.rect_filled(rect, 2.0, colors.bg_active);
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "…",
        egui::FontId::proportional(rect.height().min(rect.width()) * 0.4),
        colors.text_dim,
    );
}

fn paint_play_button(painter: &egui::Painter, rect: egui::Rect) {
    let center = rect.center();
    let r = rect.width().min(rect.height()) * 0.22;
    // Dark circle behind the triangle for contrast.
    painter.circle_filled(center, r, Color32::from_black_alpha(170));
    // Equilateral triangle pointing right, optically centered.
    let tri_size = r * 0.9;
    let optical_shift = tri_size * 0.15; // triangles look off-center without this
    let p0 = egui::pos2(center.x - tri_size * 0.45 + optical_shift, center.y - tri_size * 0.55);
    let p1 = egui::pos2(center.x - tri_size * 0.45 + optical_shift, center.y + tri_size * 0.55);
    let p2 = egui::pos2(center.x + tri_size * 0.55 + optical_shift, center.y);
    painter.add(egui::Shape::convex_polygon(
        vec![p0, p1, p2],
        Color32::WHITE,
        egui::Stroke::NONE,
    ));
}

fn paint_generic_file_icon(
    painter: &egui::Painter,
    rect: egui::Rect,
    path: &std::path::Path,
    colors: &crate::theme::Colors,
) {
    painter.rect_filled(rect, 4.0, colors.bg_active);
    painter.rect_stroke(
        rect,
        4.0,
        egui::Stroke::new(1.0, colors.text_dim),
        egui::StrokeKind::Inside,
    );
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_uppercase())
        .unwrap_or_else(|| "FILE".to_string());
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        ext,
        egui::FontId::monospace(rect.height().min(rect.width()) * 0.22),
        colors.text_primary,
    );
}

/// Rough truncation for a fixed pixel width. egui has no width-measurement API
/// available without a font atlas lookup, so approximate with 6px/char.
fn truncate_for_width(s: &str, width_px: f32) -> String {
    let max_chars = (width_px / 6.0).max(4.0) as usize;
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
        out.push('…');
        out
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
        // Check for hot-reload signal before rendering.
        if self.check_reload() {
            self.restart();
        }

        let size = ui.available_size();

        // Send Init on first render.
        if !self.initialized {
            self.initialized = true;
            self.last_size = size;
            self.send_event(&PlexiEvent::Init {
                width: size.x,
                height: size.y,
                pixels_per_point: ui.ctx().pixels_per_point(),
                protocol_version: crate::app_protocol::HOST_PROTOCOL_VERSION,
                open_intent: self.open_intent.clone(),
                capability_manifest: None,
            });
        }

        // Send Resize if size changed.
        if (size - self.last_size).length() > 1.0 {
            self.last_size = size;
            self.send_event(&PlexiEvent::Resize { width: size.x, height: size.y });
        }

        // Request a new frame.
        let delta_time = self.last_render_time.elapsed().as_secs_f32();
        self.last_render_time = Instant::now();
        self.send_event(&PlexiEvent::Render {
            width: size.x,
            height: size.y,
            delta_time,
            mode: crate::app_protocol::RenderMode::Full,
        });

        // Drain all draw commands that arrived since last frame (including response
        // to the Render we just sent — they come async so we take whatever is ready).
        //
        // Two-buffer design: `pending_frame` accumulates commands for the frame
        // currently being received. On FrameDone it is atomically swapped into
        // `frame` (the last fully committed frame). This guarantees `frame` is
        // always a complete, valid snapshot — partial frames never reach the painter.
        // If multiple FrameDones arrive in one drain, the LAST complete frame wins.
        let new_cmds = self.drain_draw_commands();
        for cmd in new_cmds {
            match cmd {
                DrawCommand::FrameDone => {
                    // Commit: swap pending into frame, reset pending for next frame.
                    std::mem::swap(&mut self.frame, &mut self.pending_frame);
                    self.pending_frame.clear();
                    // Reset transform stack at frame boundary.
                    self.transform_stack.clear();
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
                DrawCommand::State { user_state, derived, session, persistent } => {
                    self.handle_state_response(AppState {
                        user_state,
                        derived,
                        session,
                        persistent,
                    });
                }
                DrawCommand::CostReport {
                    app_id: _, service, model,
                    input_tokens, output_tokens, cost_usd,
                    operation_id, timestamp,
                } => {
                    self.cost_tracker.record(
                        &service, &model,
                        input_tokens, output_tokens, cost_usd,
                        operation_id.as_deref(), timestamp.as_deref(),
                    );
                }
                DrawCommand::Notification {
                    id,
                    title,
                    body,
                    source_app,
                    urgency,
                    expires_at,
                    visible_after,
                    run_id,
                    action,
                } => {
                    let urgency_str = urgency.clone();
                    // Emit event log entry for notification.
                    crate::event_log::emit(crate::event_log::HostEvent::NotificationEmitted {
                        id: id.clone(),
                        title: title.clone(),
                        urgency: urgency_str.clone(),
                        timestamp: crate::event_log::now_timestamp(),
                    });
                    log::info!(
                        "app::{} notification [{id}]: {title}{}",
                        self.type_id,
                        body.as_deref().map(|b| format!(" — {b}")).unwrap_or_default()
                    );
                    // Trust the app's self-reported source_app if set.
                    let src = source_app
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| self.type_id.clone());
                    let source_tag = format!("app:{}", self.type_id);
                    crate::notification_log::record(
                        title,
                        body,
                        src,
                        urgency_str,
                        expires_at,
                        visible_after,
                        run_id,
                        action,
                        Some(source_tag),
                    );
                }
                DrawCommand::SetCursor { cursor } => {
                    let icon = match cursor.as_str() {
                        "pointer"   => egui::CursorIcon::PointingHand,
                        "grab"      => egui::CursorIcon::Grab,
                        "grabbing"  => egui::CursorIcon::Grabbing,
                        "crosshair" => egui::CursorIcon::Crosshair,
                        "text"      => egui::CursorIcon::Text,
                        _           => egui::CursorIcon::Default,
                    };
                    self.pending_cursor = Some(icon);
                }
                DrawCommand::MouseTracking { enabled } => {
                    self.mouse_tracking = enabled;
                }
                DrawCommand::StatusSummary { summary: _ } => {
                    // Summary-only metadata is reserved for parent/depth views.
                    // Do not include it in the committed visual frame.
                }
                DrawCommand::SpawnApp {
                    app_id,
                    args,
                    parent,
                    layout,
                    lifecycle,
                    linked,
                    wire_channels,
                } => {
                    let target = format!("app::{}", self.type_id);
                    log::debug!(
                        target: &target,
                        "spawn_app requested: target={app_id} parent={parent:?} layout={layout:?} \
                         lifecycle={lifecycle:?} linked={linked} channels={wire_channels:?}"
                    );
                    self.pending_spawns.push(PendingSpawn {
                        app_id,
                        args,
                        parent,
                        layout,
                        lifecycle,
                        linked,
                        wire_channels,
                    });
                }
                DrawCommand::PipeWrite { channel, value } => {
                    // Emit event log entry for pipe write.
                    crate::event_log::emit_pipe_write(self.type_id.clone(), channel.clone());
                    // Queue for the host pipe dispatcher.
                    let target = format!("app::{}", self.type_id);
                    log::debug!(target: &target, "pipe_write: channel={channel:?}");
                    self.pending_pipe_writes.push((channel, value));
                }
                DrawCommand::PipeSubscribe { channel: _ } => {
                    // Phase 0 no-op: silently consume. Forward-compat only.
                }
                DrawCommand::SecretGet { name } => {
                    let dir_str = self.scope_root.display().to_string();
                    let value = crate::secrets::resolve_secret(&name, &self.type_id, &dir_str)
                        .map(|z| z.to_string());
                    self.send_event(&PlexiEvent::SecretResponse {
                        name: name.clone(),
                        value,
                    });
                }
                DrawCommand::RunCreate { head_task, payload, parent_run_id, notification_title } => {
                    let run_id = if let Some(rs) = &self.run_store {
                        let caller = crate::app_protocol::Caller {
                            app_id: self.type_id.clone(),
                            pane_id: Some(self.pane_id),
                            source: crate::app_protocol::CallerSource::Spawn,
                        };
                        let id = rs.lock().unwrap().create(
                            head_task.clone(),
                            payload,
                            caller.clone(),
                            parent_run_id,
                        );
                        crate::event_log::emit(crate::event_log::HostEvent::RunCreated {
                            run_id: id.clone(),
                            app_id: self.type_id.clone(),
                            timestamp: crate::event_log::now_timestamp(),
                        });
                        let _ = notification_title;
                        id
                    } else {
                        format!("run_nostore_{}", self.pane_id)
                    };
                    self.pending_back_events.push(PlexiEvent::RunCreated { run_id });
                }
                DrawCommand::RunUpdate { run_id, status, head_task, payload } => {
                    if let Some(rs) = &self.run_store {
                        let status_tag = format!("{:?}", status).to_lowercase();
                        rs.lock().unwrap().update(&run_id, status, head_task, payload);
                        crate::event_log::emit(crate::event_log::HostEvent::RunUpdated {
                            run_id,
                            status: status_tag,
                            timestamp: crate::event_log::now_timestamp(),
                        });
                    }
                }
                DrawCommand::RunComplete { run_id, outcome } => {
                    if let Some(rs) = &self.run_store {
                        let outcome_str = format!("{:?}", outcome).to_lowercase();
                        rs.lock().unwrap().complete(&run_id, outcome);
                        crate::event_log::emit(crate::event_log::HostEvent::RunCompleted {
                            run_id,
                            status: outcome_str,
                            timestamp: crate::event_log::now_timestamp(),
                        });
                    }
                }
                DrawCommand::RunGet { run_id } => {
                    let run = self.run_store.as_ref()
                        .and_then(|rs| rs.lock().ok())
                        .and_then(|store| store.get(&run_id).cloned());
                    self.send_event(&PlexiEvent::RunState { run_id, run });
                }
                DrawCommand::EventSubscribe { kinds: _, scope: _ } => {
                    // Phase 0 no-op: accepted for forward compatibility.
                    // Full subscription tracking and EventData delivery will land
                    // in a follow-up PR once the event routing layer is built.
                    log::debug!("ProcessApp[{}]: EventSubscribe received (Phase 0 no-op)", self.type_id);
                }
                DrawCommand::PipeListWires => {
                    // Response handled at app.rs level; no-op in ProcessApp.
                }
                DrawCommand::PushTransform { .. } => {
                    // Keep in pending_frame so the renderer can apply transforms inline.
                    self.pending_frame.push(cmd);
                }
                DrawCommand::PopTransform => {
                    self.pending_frame.push(DrawCommand::PopTransform);
                }
                DrawCommand::MeasureText { request_id, text, size, monospace, bold } => {
                    // Store in pending_frame; resolved to a TextMetrics event in ui() where we have egui context.
                    self.pending_frame.push(DrawCommand::MeasureText { request_id, text, size, monospace, bold });
                }
                other => self.pending_frame.push(other),
            }
        }

        // Handle MeasureText commands — requires egui context.
        // Scan pending_frame for MeasureText, respond immediately, then remove them.
        let measure_cmds: Vec<DrawCommand> = self.pending_frame
            .iter()
            .filter(|c| matches!(c, DrawCommand::MeasureText { .. }))
            .cloned()
            .collect();
        self.pending_frame.retain(|c| !matches!(c, DrawCommand::MeasureText { .. }));
        for cmd in measure_cmds {
            if let DrawCommand::MeasureText { request_id, text, size, monospace, .. } = cmd {
                let font_id = if monospace {
                    egui::FontId::monospace(size)
                } else {
                    egui::FontId::proportional(size)
                };
                let text_size = ui.ctx().fonts(|f| {
                    f.layout_no_wrap(text.clone(), font_id, egui::Color32::WHITE).rect.size()
                });
                self.pending_back_events.push(PlexiEvent::TextMetrics {
                    request_id,
                    width: text_size.x,
                    height: text_size.y,
                    ascent: text_size.y * 0.8,
                });
            }
        }

        // Send any pending back-events (RunCreated, etc.) to the app.
        let back_events: Vec<PlexiEvent> = std::mem::take(&mut self.pending_back_events);
        for event in back_events {
            self.send_event(&event);
        }

        // Apply cursor override requested by the app this frame, then clear it.
        if let Some(icon) = self.pending_cursor.take() {
            ui.ctx().set_cursor_icon(icon);
        }

        // Render the current frame.
        let frame_clone = self.frame.clone();
        let cwd = self.cwd.clone();
        egui::Frame::new()
            .fill(ctx.colors.terminal_bg)
            .show(ui, |ui| {
                Self::render_draw_commands(ui, &frame_clone, ctx, &cwd);
            });

        // Poll the subprocess at ~60 fps. Using request_repaint() with no delay
        // causes unlimited repaints and visible flickering.
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
    }

    fn handle_key(&mut self, input: &egui::InputState) -> bool {
        let mut consumed = false;
        for event in &input.events {
            match event {
                egui::Event::Key { key, pressed: true, modifiers, .. } => {
                    // Skip letter keys (A–Z) — they are also fired as Event::Text with the
                    // correct case and modifiers applied. Forwarding both would cause apps to
                    // receive every letter keypress twice. Control/navigation keys (Backspace,
                    // Enter, arrows, F-keys, etc.) are not fired as Event::Text, so they must
                    // still be forwarded here. Modifier-held letter combos (Cmd+S, Ctrl+C, etc.)
                    // are NOT fired as Event::Text either, so they're safe to forward.
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
                    // Forward each typed character as a Key event with the character as the key
                    // name. This covers letters, digits, and symbols with correct case applied.
                    // Apps receive printable input by checking `len(key) == 1 and key.isprintable()`.
                    for ch in text.chars() {
                        if ch.is_control() {
                            continue; // control chars come through Event::Key
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

    fn take_pending_spawns(&mut self) -> Vec<PendingSpawn> {
        std::mem::take(&mut self.pending_spawns)
    }

    fn take_pipe_writes(&mut self) -> Vec<(String, serde_json::Value)> {
        ProcessApp::take_pipe_writes(self)
    }

    fn send_pipe_data(&mut self, from_app: &str, channel: &str, value: &serde_json::Value) {
        ProcessApp::send_pipe_data(self, from_app, channel, value);
    }

    fn handle_drop(&mut self, local_pos: egui::Pos2, paths: &[PathBuf]) -> bool {
        // Find the topmost DropTarget in the last committed frame whose rect
        // contains the drop position. Iterate in reverse so later-declared
        // targets (painted on top) take precedence.
        let hit = self.frame.iter().rev().find_map(|cmd| {
            if let DrawCommand::DropTarget { id, x, y, w, h, accept, .. } = cmd {
                let rect = egui::Rect::from_min_size(
                    egui::pos2(*x, *y),
                    egui::vec2(*w, *h),
                );
                if rect.contains(local_pos) {
                    Some((id.clone(), accept.clone()))
                } else {
                    None
                }
            } else {
                None
            }
        });

        let Some((target_id, accept)) = hit else {
            return false;
        };

        let accept_lower: Vec<String> =
            accept.iter().map(|s| s.trim_start_matches('.').to_ascii_lowercase()).collect();
        let filtered: Vec<String> = paths
            .iter()
            .filter(|p| {
                if accept_lower.is_empty() {
                    return true;
                }
                match p.extension().and_then(|e| e.to_str()) {
                    Some(ext) => accept_lower.iter().any(|a| a == &ext.to_ascii_lowercase()),
                    None => false,
                }
            })
            .map(|p| p.display().to_string())
            .collect();

        if filtered.is_empty() {
            log::debug!(
                "ProcessApp[{}]: drop on '{}' filtered out all {} path(s)",
                self.type_id, target_id, paths.len()
            );
            return true;
        }

        self.send_event(&PlexiEvent::Drop {
            target_id,
            paths: filtered,
        });
        true
    }


    fn on_command(&mut self, cmd: &str) -> Option<AppCommand> {
        self.send_event(&PlexiEvent::Command { text: cmd.to_string() });
        // Commands are dispatched to the app; we don't also run them in the terminal
        // unless the app sends back a RunInTerminal draw command.
        None
    }

    fn serialize_state(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type_id": self.type_id,
        }))
    }

    fn mouse_tracking_enabled(&self) -> bool {
        self.mouse_tracking
    }

    fn send_mouse_down(&mut self, x: f32, y: f32, button: &str) {
        self.send_event(&PlexiEvent::MouseDown { x, y, button: button.to_string() });
    }

    fn send_mouse_up(&mut self, x: f32, y: f32, button: &str) {
        self.send_event(&PlexiEvent::MouseUp { x, y, button: button.to_string() });
    }

    fn send_mouse_move(&mut self, x: f32, y: f32) {
        if self.mouse_tracking {
            self.send_event(&PlexiEvent::MouseMove { x, y });
        }
    }

    fn send_scroll(&mut self, x: f32, y: f32, delta_x: f32, delta_y: f32) {
        self.send_event(&PlexiEvent::Scroll { x, y, delta_x, delta_y });
    }

    fn undo(&mut self) {
        self.do_undo();
    }

    fn redo(&mut self) {
        self.do_redo();
    }
}

impl Drop for ProcessApp {
    fn drop(&mut self) {
        self.send_event(&PlexiEvent::Shutdown);
        // Close stdin first so the child sees EOF and can exit cleanly rather
        // than blocking on its read loop. Close draw_rx so the stdout reader
        // thread sees send() fail and exits without spinning.
        self.stdin = None;
        self.draw_rx = None;
        if let Some(mut child) = self.process.take() {
            // Kill then wait — kill() is safe if the child already exited,
            // and wait() reaps the zombie so the process doesn't linger.
            let _ = child.kill();
            let _ = child.wait();
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
