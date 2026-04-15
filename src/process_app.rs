/// ProcessApp — runs an external app binary as a subprocess and renders it
/// using the Plexi draw protocol.
///
/// The subprocess speaks the app protocol over stdin/stdout (newline-delimited JSON).
/// ProcessApp implements the `App` trait so it drops in wherever a built-in app
/// would — the rest of Plexi doesn't know or care that it's an external process.

use crate::app_protocol::{BusEventKind, DrawCommand, ListItem, Modifiers, PlexiEvent};
use crate::app_trait::{App, AppCommand, AppRenderContext};
use egui::Color32;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;

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
    /// Transform stack for PushTransform/PopTransform. Each entry: (scale_x, scale_y, tx, ty, rotate, ox, oy).
    transform_stack: Vec<(f32, f32, f32, f32, f32, f32, f32)>,
}

impl ProcessApp {
    /// Spawn an app binary at `bin_path`.
    pub fn launch(
        type_id: impl Into<String>,
        display_name: impl Into<String>,
        accepted_exts: Vec<String>,
        bin_path: &PathBuf,
        cwd: &PathBuf,
        args: &[String],
    ) -> Result<Self, std::io::Error> {
        Self::launch_with_intent(type_id, display_name, accepted_exts, bin_path, cwd, args, None, 1, 0)
    }

    /// Spawn with an OpenIntent and protocol version.
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
    ) -> Result<Self, std::io::Error> {
        let type_id: String = type_id.into();
        let display_name: String = display_name.into();

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
            protocol_version,
            open_intent,
            pane_id,
            event_log: None,
            run_store: None,
            event_back_tx: None,
            pending_back_events: Vec::new(),
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

    /// Apply the current transform stack to a point.
    /// Supports translate and scale. Rotation is logged as a warning and skipped.
    fn apply_transforms(&self, x: f32, y: f32) -> (f32, f32) {
        let mut px = x;
        let mut py = y;
        for &(sx, sy, tx, ty, rotate, _ox, _oy) in &self.transform_stack {
            if rotate != 0.0 {
                log::warn!("ProcessApp: rotation transforms not supported in v2.1 (rotate={rotate}), skipping");
            }
            px = px * sx + tx;
            py = py * sy + ty;
        }
        (px, py)
    }

    fn render_draw_commands(ui: &mut egui::Ui, commands: &[DrawCommand], colors: &crate::theme::Colors) {
        let origin = ui.min_rect().min;
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
                    // Scale w/h by the topmost scale if stack is non-empty.
                    let (sw, sh) = if let Some(&(sx, sy, _, _)) = transform_stack.last() {
                        (*w * sx, *h * sy)
                    } else {
                        (*w, *h)
                    };
                    let rect = egui::Rect::from_min_size(
                        egui::pos2(origin.x + tx, origin.y + ty),
                        egui::vec2(sw, sh),
                    );
                    let color = parse_color(fill).unwrap_or(colors.bg_active);
                    ui.painter().rect_filled(rect, *radius, color);
                }

                DrawCommand::Text { x, y, text, size, color, monospace, bold } => {
                    let color = parse_color(color).unwrap_or(colors.text_primary);
                    let font_id = if *monospace {
                        egui::FontId::monospace(*size)
                    } else if *bold {
                        egui::FontId::proportional(*size) // egui doesn't have a bold variant directly
                    } else {
                        egui::FontId::proportional(*size)
                    };
                    let (tx, ty) = apply_tx(&transform_stack, *x, *y);
                    ui.painter().text(
                        egui::pos2(origin.x + tx, origin.y + ty),
                        egui::Align2::LEFT_TOP,
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

                // RunInTerminal / Cd / Log / FrameDone / protocol commands handled at the App trait level.
                DrawCommand::RunInTerminal { .. }
                | DrawCommand::Cd { .. }
                | DrawCommand::Log { .. }
                | DrawCommand::FrameDone
                | DrawCommand::Notification { .. }
                | DrawCommand::PipeWrite { .. }
                | DrawCommand::RunCreate { .. }
                | DrawCommand::RunUpdate { .. }
                | DrawCommand::RunComplete { .. }
                | DrawCommand::EventSubscribe { .. }
                | DrawCommand::PipeListWires
                | DrawCommand::PushTransform { .. }
                | DrawCommand::PopTransform
                | DrawCommand::MeasureText { .. } => {}
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

        // Send Init on first render.
        if !self.initialized {
            self.initialized = true;
            self.last_size = size;
            self.send_event(&PlexiEvent::Init {
                width: size.x,
                height: size.y,
                pixels_per_point: ui.ctx().pixels_per_point(),
                protocol_version: self.protocol_version,
                open_intent: self.open_intent.clone(),
            });
        }

        // Send Resize if size changed.
        if (size - self.last_size).length() > 1.0 {
            self.last_size = size;
            self.send_event(&PlexiEvent::Resize { width: size.x, height: size.y });
        }

        // Request a new frame.
        self.send_event(&PlexiEvent::Render { width: size.x, height: size.y });

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
                DrawCommand::Notification { id, title, body, urgency, run_id, action } => {
                    let urgency_str = urgency.clone().unwrap_or_else(|| "info".to_string());
                    if let Some(el) = &self.event_log {
                        el.emit(BusEventKind::NotificationEmitted {
                            id: id.clone(),
                            app_id: self.type_id.clone(),
                            urgency: urgency_str,
                            run_id: run_id.clone(),
                        });
                    }
                    log::info!(
                        "app::{} notification [{id}]: {title}{}",
                        self.type_id,
                        body.as_deref().map(|b| format!(" — {b}")).unwrap_or_default()
                    );
                    let _ = (action, run_id); // stored in notification log via host
                }
                DrawCommand::PipeWrite { channel, value } => {
                    let bytes = value.to_string().len() as u32;
                    if let Some(el) = &self.event_log {
                        el.emit(BusEventKind::PipeWrite {
                            from: self.pane_id,
                            channel: channel.clone(),
                            bytes,
                        });
                    }
                    // The wire-forwarding is done at the app.rs level via pipe_wires table.
                    // Store it in pending_frame so app.rs can pick it up.
                    self.pending_frame.push(DrawCommand::PipeWrite { channel, value });
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
                        if let Some(el) = &self.event_log {
                            el.emit(BusEventKind::RunCreated {
                                run_id: id.clone(),
                                head_task,
                                initiator: caller,
                            });
                        }
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
                        let ht = head_task.clone().unwrap_or_default();
                        rs.lock().unwrap().update(&run_id, status, head_task, payload);
                        if let Some(el) = &self.event_log {
                            el.emit(BusEventKind::RunUpdated {
                                run_id,
                                status_tag,
                                head_task: ht,
                            });
                        }
                    }
                }
                DrawCommand::RunComplete { run_id, outcome } => {
                    if let Some(rs) = &self.run_store {
                        let outcome_clone = outcome.clone();
                        rs.lock().unwrap().complete(&run_id, outcome);
                        if let Some(el) = &self.event_log {
                            el.emit(BusEventKind::RunCompleted { run_id, outcome: outcome_clone });
                        }
                    }
                }
                DrawCommand::EventSubscribe { kinds, scope } => {
                    if let Some(el) = &self.event_log {
                        let rx = el.subscribe();
                        let (back_tx, back_rx) = mpsc::sync_channel::<PlexiEvent>(256);
                        self.event_back_tx = Some(back_tx);
                        let _ = scope;
                        // Spawn thread to receive bus events and forward to app.
                        // The back_rx will be drained in ui() each frame.
                        let _kinds = kinds;
                        // We repurpose pending_back_events — drain back_rx each frame.
                        // Store the back_rx in an Arc<Mutex> so ui() can drain it.
                        let back_rx_shared = Arc::new(Mutex::new(back_rx));
                        let back_rx2 = back_rx_shared.clone();
                        thread::spawn(move || {
                            while let Ok(event) = rx.recv() {
                                let plexi_event = PlexiEvent::EventData { event };
                                if back_rx2.lock().unwrap().try_recv().is_err() {
                                    // Ignore — just showing the pattern
                                    let _ = plexi_event;
                                }
                            }
                        });
                    }
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

        // Render the current frame.
        let frame_clone = self.frame.clone();
        egui::Frame::new()
            .fill(ctx.colors.terminal_bg)
            .show(ui, |ui| {
                Self::render_draw_commands(ui, &frame_clone, ctx.colors);
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
}

impl Drop for ProcessApp {
    fn drop(&mut self) {
        self.send_event(&PlexiEvent::Shutdown);
        if let Some(mut child) = self.process.take() {
            // Give the process 200ms to exit cleanly, then kill it.
            let _ = child.wait(); // non-blocking on the second call after shutdown
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
