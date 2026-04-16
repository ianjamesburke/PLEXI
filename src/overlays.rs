use crate::tiling::PaneId;
use egui::{Align, Align2, CornerRadius, Layout, RichText, Stroke, Vec2};

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
            // Depth indicator (Z-axis)
            let depth = self.depth_stack.len();
            if depth > 0 {
                ui.add_space(6.0);
                ui.label(
                    RichText::new(format!("Z{depth}"))
                        .size(10.0)
                        .color(self.colors.accent)
                        .strong()
                        .family(egui::FontFamily::Monospace),
                );
            }

            ui.add_space(8.0);
            if ui
                .small_button("Depth tree")
                .on_hover_text("Open the recursive .plexi tree")
                .clicked()
            {
                self.open_depth_tree();
            }

            // Right side — help button + notification indicator
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

                // Notification unread indicator. Shown whenever the log has
                // loaded at least once — click to open the palette (Cmd+Shift+N).
                let unread = crate::notification_log::unread_count();
                let label = if unread > 0 {
                    format!("\u{1F514} {unread}")
                } else {
                    "\u{1F514}".to_string()
                };
                let color = if unread > 0 {
                    self.colors.accent
                } else {
                    self.colors.text_dim
                };
                ui.add_space(6.0);
                if ui
                    .add(
                        egui::Button::new(RichText::new(label).size(12.0).color(color))
                            .frame(false),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text("Notifications (\u{2318}\u{21E7}N)")
                    .clicked()
                {
                    self.show_notification_palette = !self.show_notification_palette;
                    if self.show_notification_palette {
                        self.notification_palette_selected = 0;
                    }
                }
            });
        });
    }

    pub(crate) fn draw_depth_breadcrumb(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let mono = egui::FontFamily::Monospace;

            // Walk the depth stack — each entry is (context_idx, path).
            // The first entry's path is the root workspace.
            for (i, (_, path)) in self.depth_stack.iter().enumerate() {
                if i > 0 {
                    ui.label(
                        RichText::new("\u{203A}")
                            .size(11.0)
                            .color(self.colors.text_dim),
                    );
                }
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "~".into());
                ui.label(
                    RichText::new(name)
                        .size(11.0)
                        .color(self.colors.text_dim)
                        .family(mono.clone()),
                );
            }

            // Current level (active context)
            ui.label(
                RichText::new("\u{203A}")
                    .size(11.0)
                    .color(self.colors.text_dim),
            );
            let current = &self.contexts[self.active_context];
            let current_name = current
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| current.path.display().to_string());
            ui.label(
                RichText::new(current_name)
                    .size(11.0)
                    .color(self.colors.accent)
                    .strong()
                    .family(mono),
            );

            // Right-aligned ascend hint
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new("\u{2318}\u{238B} ascend")
                        .size(10.0)
                        .color(self.colors.text_dim)
                        .family(egui::FontFamily::Monospace),
                );
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
                            ("\u{2318}\u{21E7}L", "Lock/unlock pane"),
                            ("\u{2318}B", "Toggle sidebar"),
                            ("\u{2318}H/J/K/L", "Focus pane"),
                            ("\u{2318}\u{21A9}", "Zoom pane"),
                            ("\u{2318}N", "New context"),
                            ("\u{2318}1-9", "Switch context"),
                            ("\u{2318}\u{21E7}E", "Depth tree"),
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

    /// Show a modal when an app requests a capability it didn't declare.
    /// Buttons: Allow once / Always allow / Deny.
    pub(crate) fn draw_capability_prompt_overlay(&mut self, ctx: &egui::Context) {
        let prompt = match &self.active_capability_prompt {
            Some(p) => p.clone(),
            None => return,
        };

        // Dark scrim behind the modal.
        egui::Area::new(egui::Id::new("capability_prompt_scrim"))
            .anchor(Align2::LEFT_TOP, Vec2::ZERO)
            .order(egui::Order::Background)
            .show(ctx, |ui| {
                let screen = ctx.screen_rect();
                ui.painter().rect_filled(screen, 0.0, egui::Color32::from_black_alpha(160));
            });

        let mut decision: Option<&str> = None;

        egui::Area::new(egui::Id::new("capability_prompt_modal"))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.colors.bg_sidebar)
                    .stroke(Stroke::new(1.0, self.colors.border))
                    .corner_radius(R6)
                    .inner_margin(egui::Margin::symmetric(20, 16))
                    .show(ui, |ui| {
                        ui.set_width(380.0);

                        ui.label(
                            RichText::new(format!("\"{}\" wants permission", prompt.app_name))
                                .size(14.0)
                                .color(self.colors.text_primary)
                                .strong(),
                        );
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(format!("Capability: {}", prompt.capability_label))
                                .size(12.0)
                                .color(self.colors.text_primary.gamma_multiply(0.65)),
                        );
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(
                                "This app did not declare this capability in its manifest.\n\
                                 \"Always allow\" records the decision so you won't be asked again.",
                            )
                            .size(11.0)
                            .color(self.colors.text_primary.gamma_multiply(0.65)),
                        );

                        ui.add_space(14.0);
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.button("Deny").clicked() {
                                decision = Some("deny");
                            }
                            ui.add_space(6.0);
                            if ui.button("Always allow").clicked() {
                                decision = Some("always_allow");
                            }
                            ui.add_space(6.0);
                            if ui.button("Allow once").clicked() {
                                decision = Some("allow_once");
                            }
                        });

                        // Escape = deny.
                        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            decision = Some("deny");
                        }
                    });
            });

        if let Some(d) = decision {
            // Persist "always_allow" and "deny" decisions; "allow_once" is ephemeral.
            if d == "always_allow" || d == "deny" {
                if let Ok(mut store) = self.permission_store.lock() {
                    store.record(&prompt.app_id, &prompt.capability, d);
                }
            }
            log::info!(
                "capability_prompt: app='{}' capability='{}' decision='{}'",
                prompt.app_id, prompt.capability, d
            );
            // Dismiss the modal and advance to the next prompt.
            self.active_capability_prompt = self.pending_capability_prompts.pop_front();
        }
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
                                    pane.name = if new_name.is_empty() {
                                        None
                                    } else {
                                        Some(new_name)
                                    };
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
}
