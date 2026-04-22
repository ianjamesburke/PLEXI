use crate::app_trait::AppCommand;
use crate::tiling::PaneId;
use egui::{Align, Align2, Color32, CornerRadius, Layout, RichText, Stroke, Vec2};

use crate::app::PlexiApp;

pub(crate) const MODAL_WIDTH: f32 = 400.0;
const R6: CornerRadius = CornerRadius::same(6);

impl PlexiApp {
    pub(crate) fn draw_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let active_ctx = &self.contexts[self.active_context];

            // Context dots (page indicators)
            if self.contexts.len() > 1 {
                let dot_radius = 3.5;
                let dot_spacing = 10.0;
                let total_width = (self.contexts.len() as f32) * dot_spacing;
                let (rect, _) = ui.allocate_exact_size(
                    Vec2::new(total_width, ui.available_height()),
                    egui::Sense::hover(),
                );
                let y = rect.center().y;
                let start_x = rect.left() + dot_radius;
                for i in 0..self.contexts.len() {
                    let cx = start_x + (i as f32) * dot_spacing;
                    let color = if i == self.active_context {
                        self.colors.accent
                    } else {
                        self.colors.bg_active
                    };
                    ui.painter()
                        .circle_filled(egui::pos2(cx, y), dot_radius, color);
                }
                ui.add_space(4.0);
            }

            // Context info
            ui.label(
                RichText::new(&active_ctx.name)
                    .size(12.0)
                    .color(self.colors.text_primary)
                    .strong(),
            );
            ui.label(
                RichText::new(active_ctx.path.display().to_string())
                    .size(11.0)
                    .color(self.colors.text_dim)
                    .family(egui::FontFamily::Monospace),
            );
            let pane_count = active_ctx.panes.len();
            ui.label(
                RichText::new(format!(
                    "{} pane{}",
                    pane_count,
                    if pane_count == 1 { "" } else { "s" }
                ))
                .size(11.0)
                .color(self.colors.text_section),
            );

            // Right side — help button + notification badge
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new("?").size(12.0).color(self.colors.text_dim),
                        )
                        .frame(false),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text("Keyboard shortcuts (\u{2318}/)")
                    .clicked()
                {
                    self.show_shortcuts = !self.show_shortcuts;
                }

                let notif_count = self.pending_notifications.len();
                if notif_count > 0 {
                    let badge_text = if notif_count > 9 {
                        "9+".to_string()
                    } else {
                        notif_count.to_string()
                    };
                    let btn = egui::Button::new(
                        RichText::new(format!("\u{1F514} {badge_text}"))
                            .size(12.0)
                            .color(self.colors.accent),
                    )
                    .frame(false);
                    if ui
                        .add(btn)
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text("Notifications (\u{2318}\u{21E7}A)")
                        .clicked()
                    {
                        self.show_notification_panel = !self.show_notification_panel;
                    }
                }
            });
        });
    }

    pub(crate) fn draw_shortcuts_overlay(&self, ctx: &egui::Context) {
        egui::Area::new(egui::Id::new("shortcuts_overlay"))
            .anchor(Align2::RIGHT_TOP, Vec2::new(-16.0, 44.0))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.colors.bg_sidebar)
                    .stroke(Stroke::new(1.0, self.colors.border))
                    .corner_radius(R6)
                    .inner_margin(egui::Margin::symmetric(16, 12))
                    .show(ui, |ui| {
                        ui.set_width(240.0);
                        ui.label(
                            RichText::new("Keyboard Shortcuts")
                                .size(13.0)
                                .color(self.colors.text_primary)
                                .strong(),
                        );
                        ui.add_space(8.0);

                        let shortcuts = [
                            ("\u{2318}P", "Command palette"),
                            ("\u{2318}\u{21E7}R", "Rename pane"),
                            ("\u{2318}T", "New tab"),
                            ("\u{2318}]/[", "Next/prev tab"),
                            ("\u{2318}D", "Split right"),
                            ("\u{2318}\u{21E7}D", "Split down"),
                            ("\u{2318}W", "Close pane"),
                            ("\u{2318}B", "Toggle sidebar"),
                            ("\u{2318}H/J/K/L", "Focus pane"),
                            ("\u{2318}\u{21A9}", "Zoom pane"),
                            ("\u{2318}N", "New context"),
                            ("\u{2318}1-9", "Switch context"),
                            ("\u{2318}/", "This help"),
                            ("\u{2318}Q", "Quit"),
                        ];

                        for (key, desc) in shortcuts {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(key)
                                        .size(11.0)
                                        .color(self.colors.accent)
                                        .family(egui::FontFamily::Monospace),
                                );
                                ui.add_space(8.0);
                                ui.label(
                                    RichText::new(desc).size(11.0).color(self.colors.text_dim),
                                );
                            });
                        }
                    });
            });
    }

    pub(crate) fn draw_rename_pane_overlay(&mut self, ctx: &egui::Context) {
        let pane_id: PaneId = match self.renaming_pane {
            Some(id) => id,
            None => return,
        };

        egui::Area::new(egui::Id::new("rename_pane_overlay"))
            .anchor(Align2::CENTER_TOP, Vec2::new(0.0, 80.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.colors.bg_sidebar)
                    .stroke(Stroke::new(1.0, self.colors.border))
                    .corner_radius(R6)
                    .inner_margin(egui::Margin::symmetric(16, 12))
                    .show(ui, |ui| {
                        ui.set_width(MODAL_WIDTH);
                        ui.label(
                            RichText::new("Rename Pane")
                                .size(13.0)
                                .color(self.colors.text_primary)
                                .strong(),
                        );
                        ui.add_space(6.0);

                        let te_id = egui::Id::new("rename_pane_input");
                        let te = ui.add(
                            egui::TextEdit::singleline(&mut self.rename_buffer)
                                .id(te_id)
                                .desired_width(MODAL_WIDTH)
                                .hint_text("Pane name...")
                                .font(egui::TextStyle::Body),
                        );

                        if te.lost_focus() {
                            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                self.renaming_pane = None;
                            } else {
                                // Apply rename
                                let new_name = self.rename_buffer.trim().to_string();
                                if let Some(pane) =
                                    self.contexts[self.active_context].panes.get_mut(&pane_id)
                                {
                                    if let Some(t) = pane.as_terminal_mut() {
                                        t.name = if new_name.is_empty() {
                                            None
                                        } else {
                                            Some(new_name)
                                        };
                                    }
                                }
                                self.renaming_pane = None;
                            }
                            // Consume Enter/Escape
                            ui.input_mut(|i| {
                                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
                                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
                            });
                        }

                        // Auto-focus and select all
                        if !te.has_focus() {
                            te.request_focus();
                            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), te_id) {
                                state
                                    .cursor
                                    .set_char_range(Some(egui::text::CCursorRange::two(
                                        egui::text::CCursor::new(0),
                                        egui::text::CCursor::new(self.rename_buffer.len()),
                                    )));
                                state.store(ui.ctx(), te_id);
                            }
                        }
                    });
            });
    }

    pub(crate) fn draw_quit_confirm_overlay(&self, ctx: &egui::Context) {
        let count = self.quit_press_count;
        egui::Area::new(egui::Id::new("quit_confirm_overlay"))
            .anchor(Align2::CENTER_BOTTOM, Vec2::new(0.0, -40.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.colors.bg_sidebar)
                    .stroke(Stroke::new(1.0, self.colors.border))
                    .corner_radius(R6)
                    .inner_margin(egui::Margin::symmetric(16, 10))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!(
                                    "\u{2318}Q pressed {} of 3 — press again to quit",
                                    count
                                ))
                                .size(12.0)
                                .color(self.colors.text_dim),
                            );
                            ui.add_space(8.0);
                            for i in 1u8..=3 {
                                let color = if i <= count {
                                    self.colors.accent
                                } else {
                                    self.colors.bg_active
                                };
                                let (rect, _) = ui.allocate_exact_size(
                                    Vec2::new(8.0, 8.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter()
                                    .circle_filled(rect.center(), 4.0, color);
                            }
                        });
                    });
            });
    }

    pub(crate) fn draw_confirm_close(&mut self, ctx: &egui::Context) {
        let mut confirmed = false;
        let mut cancelled = false;

        egui::Area::new(egui::Id::new("confirm_close_overlay"))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.colors.bg_sidebar)
                    .stroke(egui::Stroke::new(1.0, self.colors.border))
                    .corner_radius(R6)
                    .inner_margin(egui::Margin::symmetric(20, 16))
                    .show(ui, |ui| {
                        ui.set_width(MODAL_WIDTH);
                        ui.label(
                            RichText::new("Close pane?")
                                .size(13.0)
                                .color(self.colors.text_primary)
                                .strong(),
                        );
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new("The running process will be terminated.")
                                .size(12.0)
                                .color(self.colors.text_dim),
                        );
                        ui.add_space(12.0);
                        ui.horizontal(|ui| {
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Close")
                                            .size(12.0)
                                            .color(self.colors.text_primary),
                                    )
                                    .fill(self.colors.bg_active),
                                )
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                confirmed = true;
                            }
                            ui.add_space(8.0);
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Cancel")
                                            .size(12.0)
                                            .color(self.colors.text_dim),
                                    )
                                    .frame(false),
                                )
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                cancelled = true;
                            }
                        });

                        // Keyboard handling
                        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            confirmed = true;
                        }
                        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            cancelled = true;
                        }
                    });
            });

        if confirmed {
            self.pending_close = false;
            self.execute_close_pane();
        } else if cancelled {
            self.pending_close = false;
        }
    }

    /// Run palette overlay (Cmd+R). Shows active runs across all panes; BlockedOnUser
    /// runs get a [!] badge and an inline text input for unblocking.
    pub(crate) fn draw_run_palette(&mut self, ctx: &egui::Context) {
        // Collect all active runs from every app pane in every context.
        // Clone needed because we hold &self across the window render.
        let all_runs: Vec<(String, String, String, Option<String>)> = Vec::new(); // (run_id, app_id, status, blocked_prompt)
        for _context in &self.contexts {}

        let mut close = false;
        egui::Window::new("Active Runs")
            .collapsible(false)
            .resizable(true)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .default_size(egui::Vec2::new(500.0, 300.0))
            .show(ctx, |ui| {
                if all_runs.is_empty() {
                    ui.label(egui::RichText::new("No active runs.").italics());
                    ui.add_space(8.0);
                    ui.label("Apps create runs via DrawCommand::RunGet.");
                } else {
                    for (run_id, app_id, status, blocked_prompt) in &all_runs {
                        let badge = if status == "blocked_on_user" {
                            "[!] "
                        } else {
                            "    "
                        };
                        ui.label(egui::RichText::new(format!("{badge}{run_id}")).monospace());
                        ui.label(
                            egui::RichText::new(format!("    app={app_id} status={status}"))
                                .small(),
                        );
                        if let Some(prompt) = blocked_prompt {
                            ui.label(prompt);
                        }
                        ui.separator();
                    }
                }
                ui.add_space(8.0);
                if ui.button("Close  [Cmd+R]").clicked() {
                    close = true;
                }
            });

        if close {
            self.show_run_palette = false;
        }
        // Also close on Escape.
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.show_run_palette = false;
        }
    }

    pub(crate) fn draw_notification_panel(&mut self, ctx: &egui::Context) -> Vec<AppCommand> {
        let mut cmds: Vec<AppCommand> = Vec::new();

        // Close on Escape.
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.show_notification_panel = false;
            return cmds;
        }

        egui::Area::new(egui::Id::new("notification_panel"))
            .order(egui::Order::Foreground)
            .anchor(Align2::RIGHT_TOP, egui::vec2(-8.0, 36.0))
            .show(ctx, |ui| {
                let panel_w = 320.0f32;

                egui::Frame::new()
                    .fill(self.colors.bg_sidebar)
                    .stroke(Stroke::new(1.0, self.colors.border))
                    .corner_radius(R6)
                    .show(ui, |ui| {
                        ui.set_width(panel_w);

                        // Header row
                        ui.horizontal(|ui| {
                            ui.add_space(16.0);
                            ui.label(
                                RichText::new("Notifications")
                                    .size(14.0)
                                    .color(self.colors.text_primary)
                                    .strong(),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.add_space(16.0);
                                if !self.pending_notifications.is_empty() {
                                    let clear_resp = ui.add(
                                        egui::Label::new(
                                            RichText::new("clear all")
                                                .size(11.0)
                                                .color(self.colors.text_dim),
                                        )
                                        .sense(egui::Sense::click()),
                                    );
                                    if clear_resp
                                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                                        .clicked()
                                    {
                                        self.pending_notifications.clear();
                                    }
                                }
                            });
                        });

                        ui.separator();

                        if self.pending_notifications.is_empty() {
                            ui.add_space(16.0);
                            ui.vertical_centered(|ui| {
                                ui.label(
                                    RichText::new("No notifications")
                                        .size(13.0)
                                        .color(self.colors.text_dim),
                                );
                            });
                            ui.add_space(16.0);
                            return;
                        }

                        let mut remove_idx: Option<usize> = None;
                        let mut action_cmd: Option<AppCommand> = None;

                        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                            for (i, notif) in self.pending_notifications.iter().enumerate() {
                                let level_color = match notif.level.as_str() {
                                    "error" => Color32::from_rgb(0xff, 0x55, 0x55),
                                    "warn" => Color32::from_rgb(0xf1, 0xfa, 0x8c),
                                    _ => self.colors.accent,
                                };

                                ui.add_space(8.0);
                                ui.horizontal(|ui| {
                                    ui.add_space(16.0);
                                    // Level dot
                                    let (dot_rect, _) = ui.allocate_exact_size(
                                        egui::vec2(8.0, 8.0),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter()
                                        .circle_filled(dot_rect.center(), 4.0, level_color);
                                    ui.add_space(8.0);
                                    ui.vertical(|ui| {
                                        ui.label(
                                            RichText::new(&notif.title)
                                                .size(13.0)
                                                .strong()
                                                .color(self.colors.text_primary),
                                        );
                                        if !notif.body.is_empty() {
                                            ui.label(
                                                RichText::new(&notif.body)
                                                    .size(12.0)
                                                    .color(self.colors.text_dim),
                                            );
                                        }
                                        ui.horizontal(|ui| {
                                            if notif.actions.is_empty() {
                                                if ui
                                                    .button(
                                                        RichText::new("Dismiss")
                                                            .size(11.0)
                                                            .color(self.colors.text_dim),
                                                    )
                                                    .on_hover_cursor(
                                                        egui::CursorIcon::PointingHand,
                                                    )
                                                    .clicked()
                                                {
                                                    action_cmd =
                                                        Some(AppCommand::DeliverNotifyAction {
                                                            pane_id: notif.sender_pane_id,
                                                            notify_id: notif.notify_id.clone(),
                                                            action_label: "dismiss".to_string(),
                                                        });
                                                    remove_idx = Some(i);
                                                }
                                            } else {
                                                for action in &notif.actions {
                                                    if ui
                                                        .button(
                                                            RichText::new(&action.label)
                                                                .size(11.0),
                                                        )
                                                        .on_hover_cursor(
                                                            egui::CursorIcon::PointingHand,
                                                        )
                                                        .clicked()
                                                    {
                                                        action_cmd = Some(
                                                            AppCommand::DeliverNotifyAction {
                                                                pane_id: notif.sender_pane_id,
                                                                notify_id: notif.notify_id.clone(),
                                                                action_label: action.label.clone(),
                                                            },
                                                        );
                                                        remove_idx = Some(i);
                                                        break;
                                                    }
                                                }
                                            }
                                        });
                                    });
                                });
                                ui.add_space(8.0);
                                ui.separator();
                            }
                        });

                        // Apply removal and collect command outside the borrow on self.pending_notifications.
                        if let Some(idx) = remove_idx {
                            self.pending_notifications.remove(idx);
                        }
                        if let Some(cmd) = action_cmd {
                            cmds.push(cmd);
                        }
                    });
            });

        cmds
    }
}
