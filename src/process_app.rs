/// ProcessApp — runs an external app binary as a subprocess and renders it
/// using the Plexi draw protocol.
///
/// The subprocess speaks the app protocol over stdin/stdout (newline-delimited JSON).
/// ProcessApp implements the `App` trait so it drops in wherever a built-in app
/// would — the rest of Plexi doesn't know or care that it's an external process.

use crate::app_protocol::{DrawCommand, ListItem, Modifiers, PlexiEvent};
use crate::app_trait::{App, AppCommand, AppRenderContext};
use egui::Color32;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

pub struct ProcessApp {
    type_id: String,
    display_name: String,
    accepted_exts: Vec<String>,
    process: Option<Child>,
    stdin: Option<ChildStdin>,
    /// Receives draw commands from the subprocess on a background thread.
    draw_rx: Option<Receiver<DrawCommand>>,
    /// Buffered draw commands for the current frame.
    frame: Vec<DrawCommand>,
    /// Pending RunInTerminal / Cd commands collected from the subprocess, to be
    /// drained by the host via take_pending_commands().
    pending_commands: Vec<crate::app_trait::AppCommand>,
    /// Size last sent to the subprocess.
    last_size: egui::Vec2,
    initialized: bool,
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
            pending_commands: Vec::new(),
            last_size: egui::Vec2::ZERO,
            initialized: false,
        })
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

    fn render_draw_commands(ui: &mut egui::Ui, commands: &[DrawCommand], colors: &crate::theme::Colors) {
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
                        egui::FontId::proportional(*size) // egui doesn't have a bold variant directly
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
                | DrawCommand::FrameDone => {}
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
        let new_cmds = self.drain_draw_commands();

        // Commit frame: use new commands if we got any, otherwise re-render last frame.
        // This avoids a blank frame on the first render before the process responds.
        let got_frame_done = new_cmds.iter().any(|c| matches!(c, DrawCommand::FrameDone));
        if got_frame_done {
            // Collect draw commands up to FrameDone; route RunInTerminal/Cd to pending_commands.
            self.frame.clear();
            let mut past_frame_done = false;
            for cmd in new_cmds {
                if past_frame_done {
                    // Commands after FrameDone belong to the next frame — leave them
                    // for the next drain cycle. We can't put them back so just skip.
                    break;
                }
                match cmd {
                    DrawCommand::FrameDone => past_frame_done = true,
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
                    other => self.frame.push(other),
                }
            }
        } else {
            // Merge — append any partial draw commands received; collect side-channel commands.
            for cmd in new_cmds {
                match cmd {
                    DrawCommand::FrameDone => {}
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
                    other => self.frame.push(other),
                }
            }
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
