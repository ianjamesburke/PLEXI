/// Notification palette — Cmd+Shift+N.
///
/// Lists notifications newest-first (top section) and backlog notes from
/// `~/.plexi-alpha/backlog/` (bottom section, urgency = "backlog").
/// Backlog items are scanned fresh each time the palette opens; dismissing
/// one moves the `.md` file to `backlog/.dismissed/`.
///
/// Separate from `command_palette.rs` so the list/filter logic stays
/// independent. Modeled structurally on the command palette.

use std::path::PathBuf;

use egui::{Align, Align2, Color32, CornerRadius, Layout, Rect, RichText, Stroke, Vec2};

use crate::app::PlexiApp;
use crate::app_protocol::NotificationAction;
use crate::notification_log::{self, Notification};
use crate::overlays::MODAL_WIDTH;

/// A backlog note scanned from `~/.plexi-alpha/backlog/`.
#[derive(Clone, Debug)]
struct BacklogItem {
    /// Filename (no directory prefix) — used as a stable ID for dismiss.
    filename: String,
    /// Full path to the file (for dismiss/move operations).
    path: PathBuf,
    /// First non-empty, non-header line used as the display title.
    title: String,
    /// File modification time, displayed as a relative age.
    modified: std::time::SystemTime,
}

/// Scan `~/.plexi-alpha/backlog/` for `.md` files. Returns items sorted
/// oldest-first (by mtime) so the user naturally sees stale notes near the
/// top and recent ones at the bottom. Skips `.dismissed/` subdirectory.
fn scan_backlog() -> Vec<BacklogItem> {
    let dir = crate::config::config_dir().join("backlog");
    let mut items: Vec<BacklogItem> = Vec::new();

    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return items, // dir doesn't exist yet — normal on first run
    };

    for entry in entries.flatten() {
        let path = entry.path();
        // Only `.md` files at the top level (not `.dismissed/`)
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }

        let filename = match path.file_name().and_then(|f| f.to_str()) {
            Some(f) => f.to_string(),
            None => continue,
        };

        let modified = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::UNIX_EPOCH);

        // Extract title: skip the `# Quick Note — …` header, use next non-empty line.
        let title = extract_backlog_title(&path);

        items.push(BacklogItem { filename, path, title, modified });
    }

    // Oldest first so long-ignored notes rise to the top.
    items.sort_by_key(|i| i.modified);
    items
}

/// Read the file and return the first non-empty, non-`#`-prefixed line as the
/// display title. Falls back to the filename stem if the file can't be parsed.
fn extract_backlog_title(path: &PathBuf) -> String {
    let content = std::fs::read_to_string(path).unwrap_or_default();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let title = trimmed.to_string();
        return if title.len() > 80 { format!("{}…", &title[..79]) } else { title };
    }
    // Fallback: filename without extension.
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("note")
        .to_string()
}

/// Move a backlog file to `backlog/.dismissed/<filename>`.
fn dismiss_backlog_file(path: &PathBuf) {
    let dismissed_dir = match path.parent() {
        Some(p) => p.join(".dismissed"),
        None => {
            log::warn!("backlog_dismiss: no parent dir for {:?}", path);
            return;
        }
    };
    if let Err(e) = std::fs::create_dir_all(&dismissed_dir) {
        log::warn!("backlog_dismiss: failed to create .dismissed dir: {e}");
        return;
    }
    let filename = match path.file_name() {
        Some(f) => f,
        None => {
            log::warn!("backlog_dismiss: no filename in {:?}", path);
            return;
        }
    };
    let dest = dismissed_dir.join(filename);
    match std::fs::rename(path, &dest) {
        Ok(()) => log::info!("backlog_dismiss: moved {:?} → {:?}", path, dest),
        Err(e) => log::warn!("backlog_dismiss: rename failed: {e}"),
    }
}

impl PlexiApp {
    pub(crate) fn draw_notification_palette(&mut self, ctx: &egui::Context) {
        // Snapshot the log under lock, then drop the guard before doing any UI
        // work. Mutations (mark_read, mark_all_read) re-acquire the lock via
        // the global helper. Avoids holding the mutex across egui calls.
        let notifications: Vec<Notification> = match notification_log::global().lock() {
            Ok(log) => log.list().to_vec(),
            Err(e) => {
                log::error!("notification_palette: mutex poisoned: {e}");
                return;
            }
        };

        // Newest first — the log itself is append-order (oldest first).
        let mut ordered: Vec<(usize, Notification)> = notifications
            .iter()
            .enumerate()
            .map(|(i, n)| (i, n.clone()))
            .collect();
        ordered.reverse();

        // Scan backlog dir fresh each open — cheap, and avoids needing a watcher.
        let backlog_items: Vec<BacklogItem> = scan_backlog();

        let total = ordered.len();
        let total_rows = total + backlog_items.len();
        if self.notification_palette_selected >= total_rows.max(1) {
            self.notification_palette_selected = 0;
        }

        // ── Keyboard nav ────────────────────────────────────────────────────
        enum Action {
            Close,
            MarkReadAt(usize),
            MarkAllRead,
            /// Enter pressed: dispatch based on the selected notification's
            /// `action_type`, then mark it read and close.
            ActivateSelected,
            /// Dismiss the backlog item at the given index in `backlog_items`.
            DismissBacklog(usize),
        }
        let mut action: Option<Action> = None;

        ctx.input_mut(|input| {
            if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                action = Some(Action::Close);
            }
            if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)
                && total_rows > 0
                && self.notification_palette_selected + 1 < total_rows
            {
                self.notification_palette_selected += 1;
            }
            if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)
                && self.notification_palette_selected > 0
            {
                self.notification_palette_selected -= 1;
            }
            // Enter on a notification → dispatch action; on a backlog item → open in editor.
            if input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                && self.notification_palette_selected < total_rows
            {
                action = Some(Action::ActivateSelected);
            }
            // Delete/Backspace on notification → mark read; on backlog item → dismiss.
            let want_dismiss = input.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
                || input.consume_key(egui::Modifiers::NONE, egui::Key::Backspace);
            if want_dismiss && self.notification_palette_selected < total_rows {
                let sel = self.notification_palette_selected;
                if sel < total {
                    let (log_idx, _) = ordered[sel];
                    action = Some(Action::MarkReadAt(log_idx));
                } else {
                    action = Some(Action::DismissBacklog(sel - total));
                }
            }
            // `d` also dismisses a focused backlog item (matches the backlog-triage UX).
            if input.consume_key(egui::Modifiers::NONE, egui::Key::D)
                && self.notification_palette_selected >= total
                && self.notification_palette_selected < total_rows
            {
                action = Some(Action::DismissBacklog(self.notification_palette_selected - total));
            }
        });

        // ── Render ──────────────────────────────────────────────────────────
        let screen_rect = ctx.screen_rect();
        egui::Area::new(egui::Id::new("notif_palette_scrim"))
            .fixed_pos(screen_rect.min)
            .show(ctx, |ui| {
                ui.painter().rect_filled(screen_rect, 0.0, Color32::from_black_alpha(120));
                let scrim_response = ui.allocate_rect(screen_rect, egui::Sense::click());
                if scrim_response.clicked() {
                    action = Some(Action::Close);
                }
            });

        let mut click_action: Option<Action> = None;
        egui::Area::new(egui::Id::new("notification_palette"))
            .anchor(Align2::CENTER_TOP, Vec2::new(0.0, 80.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.colors.bg_sidebar)
                    .stroke(Stroke::new(1.0, self.colors.border))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.set_width(MODAL_WIDTH);

                        // Header: title + mark-all-read button.
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Notifications")
                                    .size(13.0)
                                    .color(self.colors.text_primary)
                                    .strong(),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("Mark all read")
                                                .size(10.0)
                                                .color(self.colors.text_dim),
                                        )
                                        .frame(false),
                                    )
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .clicked()
                                {
                                    click_action = Some(Action::MarkAllRead);
                                }
                            });
                        });
                        ui.add_space(6.0);

                        if ordered.is_empty() && backlog_items.is_empty() {
                            ui.label(
                                RichText::new("No notifications yet")
                                    .size(11.0)
                                    .color(self.colors.text_dim),
                            );
                            return;
                        }

                        egui::ScrollArea::vertical()
                            .max_height(360.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                // ── Notifications ──────────────────────────
                                for (row_idx, (log_idx, n)) in ordered.iter().enumerate() {
                                    let is_selected = row_idx == self.notification_palette_selected;
                                    let fill = if is_selected {
                                        self.colors.bg_active
                                    } else {
                                        Color32::TRANSPARENT
                                    };

                                    // Run-card notifications get taller rows.
                                    let row_h = if n.run_id.is_some() { 58.0 } else { 42.0 };
                                    let row_rect = Rect::from_min_size(
                                        ui.cursor().min,
                                        Vec2::new(MODAL_WIDTH, row_h),
                                    );
                                    ui.painter().rect_filled(row_rect, CornerRadius::same(4), fill);

                                    // Urgency / read dot — small circle left-side.
                                    let dot_color = if n.read {
                                        self.colors.text_section
                                    } else {
                                        match n.urgency.as_str() {
                                            "high" => Color32::from_rgb(0xdd, 0x77, 0x55),
                                            "medium" => Color32::from_rgb(0xf9, 0xc8, 0x6a),
                                            _ => self.colors.text_dim, // "low" or unknown
                                        }
                                    };
                                    ui.painter().circle_filled(
                                        egui::pos2(row_rect.min.x + 12.0, row_rect.min.y + 21.0),
                                        4.0,
                                        dot_color,
                                    );

                                    ui.allocate_ui_with_layout(
                                        Vec2::new(MODAL_WIDTH, row_h),
                                        Layout::left_to_right(Align::Center),
                                        |ui| {
                                            ui.add_space(24.0);
                                            ui.vertical(|ui| {
                                                ui.add_space(4.0);
                                                ui.horizontal(|ui| {
                                                    ui.label(
                                                        RichText::new(&n.source_app)
                                                            .size(10.0)
                                                            .color(self.colors.text_dim),
                                                    );
                                                    ui.label(
                                                        RichText::new("\u{203A}")
                                                            .size(10.0)
                                                            .color(self.colors.text_dim),
                                                    );
                                                    let title_color = if n.read {
                                                        self.colors.text_dim
                                                    } else {
                                                        self.colors.text_primary
                                                    };
                                                    ui.label(
                                                        RichText::new(&n.title)
                                                            .size(12.0)
                                                            .color(title_color),
                                                    );
                                                });
                                                let sub = match &n.body {
                                                    Some(b) if !b.is_empty() => {
                                                        format!("{}  \u{00B7}  {}", format_time(&n.timestamp), b)
                                                    }
                                                    _ => format_time(&n.timestamp),
                                                };
                                                ui.label(
                                                    RichText::new(sub)
                                                        .size(9.0)
                                                        .color(self.colors.text_dim),
                                                );
                                                // Run card: show run_id pill when present.
                                                if let Some(run_id) = &n.run_id {
                                                    ui.add_space(2.0);
                                                    ui.horizontal(|ui| {
                                                        let run_label = format!("\u{25B6} run:{}", &run_id[..run_id.len().min(8)]);
                                                        ui.label(
                                                            RichText::new(run_label)
                                                                .size(9.0)
                                                                .color(Color32::from_rgb(0x6a, 0xb0, 0xf9))
                                                                .monospace(),
                                                        );
                                                        // Action label for run-associated notifications.
                                                        let action_label = match &n.action {
                                                            Some(NotificationAction::ResumeRun { .. }) => Some("Resume"),
                                                            Some(NotificationAction::OpenIntent { .. }) => Some("Open"),
                                                            Some(NotificationAction::Focus { .. }) => Some("Focus"),
                                                            _ => None,
                                                        };
                                                        if let Some(label) = action_label {
                                                            ui.label(
                                                                RichText::new(format!("· {label}"))
                                                                    .size(9.0)
                                                                    .color(self.colors.text_dim),
                                                            );
                                                        }
                                                    });
                                                }
                                            });
                                        },
                                    );

                                    let r = ui.interact(
                                        row_rect,
                                        egui::Id::new(("notif_row", row_idx)),
                                        egui::Sense::click(),
                                    );
                                    if r.clicked() {
                                        click_action = Some(Action::MarkReadAt(*log_idx));
                                    }
                                    if r.hovered() {
                                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                    }
                                }

                                // ── Backlog items ──────────────────────────
                                if !backlog_items.is_empty() {
                                    if !ordered.is_empty() {
                                        ui.add_space(6.0);
                                        ui.separator();
                                        ui.add_space(4.0);
                                    }
                                    ui.label(
                                        RichText::new("Backlog")
                                            .size(10.0)
                                            .color(self.colors.text_section)
                                            .strong(),
                                    );
                                    ui.add_space(4.0);

                                    for (bl_idx, item) in backlog_items.iter().enumerate() {
                                        let row_idx = total + bl_idx;
                                        let is_selected = row_idx == self.notification_palette_selected;
                                        let fill = if is_selected {
                                            self.colors.bg_active
                                        } else {
                                            Color32::TRANSPARENT
                                        };

                                        let row_rect = Rect::from_min_size(
                                            ui.cursor().min,
                                            Vec2::new(MODAL_WIDTH, 42.0),
                                        );
                                        ui.painter().rect_filled(row_rect, CornerRadius::same(4), fill);

                                        // Backlog dot: dim gray (lowest urgency).
                                        ui.painter().circle_filled(
                                            egui::pos2(row_rect.min.x + 12.0, row_rect.min.y + 21.0),
                                            4.0,
                                            self.colors.text_section,
                                        );

                                        ui.allocate_ui_with_layout(
                                            Vec2::new(MODAL_WIDTH, 42.0),
                                            Layout::left_to_right(Align::Center),
                                            |ui| {
                                                ui.add_space(24.0);
                                                ui.vertical(|ui| {
                                                    ui.add_space(4.0);
                                                    ui.horizontal(|ui| {
                                                        ui.label(
                                                            RichText::new("backlog")
                                                                .size(10.0)
                                                                .color(self.colors.text_section),
                                                        );
                                                        ui.label(
                                                            RichText::new("\u{203A}")
                                                                .size(10.0)
                                                                .color(self.colors.text_dim),
                                                        );
                                                        ui.label(
                                                            RichText::new(&item.title)
                                                                .size(12.0)
                                                                .color(self.colors.text_primary),
                                                        );
                                                    });
                                                    let age = format_age(item.modified);
                                                    ui.label(
                                                        RichText::new(format!("{age}  ·  d to dismiss"))
                                                            .size(9.0)
                                                            .color(self.colors.text_dim),
                                                    );
                                                });
                                            },
                                        );

                                        let r = ui.interact(
                                            row_rect,
                                            egui::Id::new(("backlog_row", bl_idx)),
                                            egui::Sense::click(),
                                        );
                                        if r.clicked() {
                                            // Open the note in the system editor on click.
                                            let p = item.path.clone();
                                            if let Err(e) = std::process::Command::new("open").arg(&p).spawn() {
                                                log::warn!("backlog: failed to open {:?}: {e}", p);
                                            }
                                        }
                                        if r.hovered() {
                                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                        }
                                    }
                                }
                            });
                    });
            });

        // Click wins over keyboard — replace action if a click fired.
        if click_action.is_some() {
            action = click_action;
        }

        match action {
            Some(Action::Close) => {
                self.show_notification_palette = false;
            }
            Some(Action::MarkReadAt(idx)) => {
                if let Ok(mut log) = notification_log::global().lock() {
                    log.mark_read(idx);
                }
            }
            Some(Action::MarkAllRead) => {
                if let Ok(mut log) = notification_log::global().lock() {
                    log.mark_all_read();
                }
            }
            Some(Action::ActivateSelected) => {
                let sel = self.notification_palette_selected;
                if sel < total {
                    let (log_idx, ref n) = ordered[sel];
                    dispatch_notification_action(self, n);
                    if let Ok(mut log) = notification_log::global().lock() {
                        log.mark_read(log_idx);
                    }
                    self.show_notification_palette = false;
                } else {
                    // Backlog item selected: open in system editor.
                    let bl_idx = sel - total;
                    if let Some(item) = backlog_items.get(bl_idx) {
                        if let Err(e) = std::process::Command::new("open").arg(&item.path).spawn() {
                            log::warn!("backlog: failed to open {:?}: {e}", item.path);
                        }
                    }
                }
            }
            Some(Action::DismissBacklog(bl_idx)) => {
                if let Some(item) = backlog_items.get(bl_idx) {
                    dismiss_backlog_file(&item.path);
                }
                // Adjust selection so it doesn't go out of bounds after dismiss.
                if self.notification_palette_selected > 0 {
                    self.notification_palette_selected -= 1;
                }
            }
            None => {}
        }
    }
}

/// Dispatch the structured action on a notification. Called on Enter press or
/// button click. Falls back to mark-read-only for action types that require
/// additional UI (Confirm, TextInput) until inline sub-prompt UI is implemented.
fn dispatch_notification_action(app: &mut PlexiApp, n: &Notification) {
    // Resolve the effective action: prefer the v2.0 structured field; fall back
    // to legacy action_type/action_payload for JSONL entries from older builds.
    let effective_action: Option<NotificationAction> = n.action.clone().or_else(|| {
        n.action_type.as_deref().and_then(|at| match at {
            "focus" => {
                let pane_id = n.action_payload.as_ref()?.get("pane_id")?.as_u64()?;
                let fullscreen = n
                    .action_payload
                    .as_ref()
                    .and_then(|p| p.get("fullscreen"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                Some(NotificationAction::Focus { pane_id, fullscreen })
            }
            _ => Some(NotificationAction::Dismiss),
        })
    });

    match effective_action {
        Some(NotificationAction::Focus { pane_id, fullscreen }) => {
            app.focus_pane_by_id(pane_id, fullscreen);
        }
        Some(NotificationAction::ResumeRun { run_id }) => {
            // TODO(#229): wire ResumeRun once run wakeup path is defined.
            log::info!("notification_palette: ResumeRun {run_id} (wakeup path pending)");
        }
        Some(NotificationAction::OpenIntent { open_intent }) => {
            // TODO(#229): wire OpenIntent dispatch once app-spawn-from-palette is implemented.
            log::info!("notification_palette: OpenIntent {:?} (dispatch pending)", open_intent);
        }
        Some(NotificationAction::RunCommand { app_id, command, args }) => {
            // TODO(#229): wire RunCommand once inter-app command channel exists.
            log::info!(
                "notification_palette: RunCommand app={app_id} cmd={command} args={args:?} (dispatch pending)"
            );
        }
        Some(NotificationAction::ExternalUrl { url }) => {
            log::info!("notification_palette: opening ExternalUrl {url}");
            if let Err(e) = std::process::Command::new("open").arg(&url).spawn() {
                log::warn!("notification_palette: failed to open URL {url}: {e}");
            }
        }
        Some(NotificationAction::Confirm { .. }) | Some(NotificationAction::TextInput { .. }) => {
            // TODO(#229): inline sub-prompt UI for Confirm/TextInput.
            // For now, mark-read + close is the fallback (handled by caller).
            log::debug!("notification_palette: Confirm/TextInput action deferred to future UI");
        }
        Some(NotificationAction::Dismiss) | None => {
            // Nothing to do — caller marks read and closes.
        }
    }
}

/// Format a timestamp as `HH:MM:SS` local time for the palette subtitle.
fn format_time(ts: &chrono::DateTime<chrono::Utc>) -> String {
    let local: chrono::DateTime<chrono::Local> = chrono::DateTime::from(*ts);
    local.format("%H:%M:%S").to_string()
}

/// Format a `SystemTime` as a human-readable age (e.g. "2h ago", "3d ago").
fn format_age(modified: std::time::SystemTime) -> String {
    let elapsed = modified.elapsed().unwrap_or_default();
    let secs = elapsed.as_secs();
    if secs < 60 {
        "just now".to_string()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}
