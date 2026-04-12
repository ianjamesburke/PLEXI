/// ProcessApp — runs an external app binary as a subprocess and renders it
/// using the Plexi draw protocol.
///
/// The subprocess speaks the app protocol over stdin/stdout (newline-delimited JSON).
/// ProcessApp implements the `App` trait so it drops in wherever a built-in app
/// would — the rest of Plexi doesn't know or care that it's an external process.

use crate::app_protocol::{DrawCommand, ListItem, Modifiers, PlexiEvent};
use crate::app_trait::{App, AppCommand, AppRenderContext};
use crate::cost_tracker::CostTracker;
use egui::Color32;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

/// Maximum number of undo states to keep in the stack.
const MAX_UNDO_DEPTH: usize = 50;

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
}

/// Snapshot of an app's state buckets.
#[derive(Clone, Debug)]
pub struct AppState {
    pub user_state: serde_json::Value,
    pub derived: serde_json::Value,
    pub session: serde_json::Value,
    pub persistent: serde_json::Value,
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

        // Set up file watcher on the app's parent directory for hot reload.
        let (reload_tx, reload_rx) = mpsc::channel::<()>();
        let watch_dir = bin_path.parent().unwrap_or(cwd).to_path_buf();
        let watcher = Self::setup_watcher(watch_dir, reload_tx);

        let cost_tracker = CostTracker::new(&type_id);
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
        })
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

        // Send shutdown and kill old process.
        self.send_event(&PlexiEvent::Shutdown);
        if let Some(mut child) = self.process.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.stdin = None;
        self.draw_rx = None;

        // Respawn.
        let mut child = match std::process::Command::new(&self.bin_path)
            .args(&self.args)
            .current_dir(&self.cwd)
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

    fn render_draw_commands(
        ui: &mut egui::Ui,
        commands: &[DrawCommand],
        ctx: &AppRenderContext<'_>,
        app_cwd: &std::path::Path,
    ) {
        let origin = ui.min_rect().min;
        let colors = ctx.colors;

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
                // FrameDone handled at the App trait level, not here.
                DrawCommand::RunInTerminal { .. }
                | DrawCommand::Cd { .. }
                | DrawCommand::Log { .. }
                | DrawCommand::State { .. }
                | DrawCommand::CostReport { .. }
                | DrawCommand::Notification { .. }
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
                DrawCommand::Notification { priority, title, body, source_app } => {
                    // Trust the app's self-reported source_app if set, otherwise
                    // fall back to the ProcessApp's own type_id. This guards
                    // against apps that forget to populate the field.
                    let src = if source_app.is_empty() {
                        self.type_id.clone()
                    } else {
                        source_app
                    };
                    crate::notification_log::record(priority, title, body, src);
                }
                other => self.pending_frame.push(other),
            }
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

        // Filter paths against the accept list. Empty accept = accept anything.
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
            // Target exists but no paths matched the accept filter. Still
            // considered "consumed" — the app owns that region and the user's
            // intent was clearly to drop there.
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
