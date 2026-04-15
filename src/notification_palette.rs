/// Notification palette — Cmd+Shift+N.
///
/// Minimum viable UI: lists all notifications newest-first, shows priority
/// dot + source_app + title + time, click to mark read, keyboard nav.
/// Separate from `command_palette.rs` so the list/filter logic stays
/// independent. Modeled structurally on the command palette.

use egui::{Align, Align2, Color32, CornerRadius, Layout, Rect, RichText, Stroke, Vec2};

use crate::app::PlexiApp;
use crate::notification_log::{self, Notification};
use crate::overlays::MODAL_WIDTH;

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

        let total = ordered.len();
        if self.notification_palette_selected >= total.max(1) {
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
        }
        let mut action: Option<Action> = None;

        ctx.input_mut(|input| {
            if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                action = Some(Action::Close);
            }
            if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)
                && total > 0
                && self.notification_palette_selected + 1 < total
            {
                self.notification_palette_selected += 1;
            }
            if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)
                && self.notification_palette_selected > 0
            {
                self.notification_palette_selected -= 1;
            }
            // Enter → dispatch action_type; Delete/Backspace → mark read only.
            if input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                && self.notification_palette_selected < total
            {
                action = Some(Action::ActivateSelected);
            }
            let want_mark = input.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
                || input.consume_key(egui::Modifiers::NONE, egui::Key::Backspace);
            if want_mark && self.notification_palette_selected < total {
                let (log_idx, _) = ordered[self.notification_palette_selected];
                action = Some(Action::MarkReadAt(log_idx));
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

                        if ordered.is_empty() {
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
                                for (row_idx, (log_idx, n)) in ordered.iter().enumerate() {
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
                                        egui::pos2(row_rect.min.x + 12.0, row_rect.center().y),
                                        4.0,
                                        dot_color,
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
                if self.notification_palette_selected < total {
                    let (log_idx, ref n) = ordered[self.notification_palette_selected];
                    match n.action_type.as_str() {
                        "focus" => {
                            // Payload: {"pane_id": u64, "fullscreen": bool}
                            let pane_id = n
                                .action_payload
                                .as_ref()
                                .and_then(|p| p.get("pane_id"))
                                .and_then(|v| v.as_u64());
                            let fullscreen = n
                                .action_payload
                                .as_ref()
                                .and_then(|p| p.get("fullscreen"))
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                            if let Some(pid) = pane_id {
                                self.focus_pane_by_id(pid, fullscreen);
                            }
                        }
                        // "confirm" and "text_input" degrade to mark-read + close
                        // until inline sub-prompt UI is implemented.
                        _ => {}
                    }
                    if let Ok(mut log) = notification_log::global().lock() {
                        log.mark_read(log_idx);
                    }
                    self.show_notification_palette = false;
                }
            }
            None => {}
        }
    }
}

/// Format a timestamp as `HH:MM:SS` local time for the palette subtitle.
fn format_time(ts: &chrono::DateTime<chrono::Utc>) -> String {
    let local: chrono::DateTime<chrono::Local> = chrono::DateTime::from(*ts);
    local.format("%H:%M:%S").to_string()
}
