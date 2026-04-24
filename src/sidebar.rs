use crate::context::ContextMenuAction;
use egui::{Align, Color32, CornerRadius, Layout, Rect, RichText, Stroke, Vec2};

use crate::app::PlexiApp;

impl PlexiApp {
    pub(crate) fn draw_sidebar(&mut self, ui: &mut egui::Ui) {
        let sidebar_width = ui.available_width();

        // Branding
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.label(
                RichText::new("PLEXI")
                    .size(16.0)
                    .color(self.colors.text_primary)
                    .strong(),
            );
        });
        ui.add_space(12.0);

        // Divider
        let rect = ui.cursor();
        ui.painter().line_segment(
            [
                egui::pos2(rect.min.x, rect.min.y),
                egui::pos2(rect.min.x + sidebar_width, rect.min.y),
            ],
            Stroke::new(1.0, self.colors.border),
        );
        ui.add_space(4.0);

        // Contexts section header with "+" button
        ui.add_space(8.0);
        let mut add_clicked = false;
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.label(
                RichText::new("Contexts")
                    .size(10.0)
                    .color(self.colors.text_section),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(12.0);
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new("+").size(12.0).color(self.colors.text_dim),
                        )
                        .frame(false),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text("New context")
                    .clicked()
                {
                    add_clicked = true;
                }
            });
        });
        ui.add_space(4.0);

        // Context list — iterate by index to avoid borrow issues with rename_buffer
        let num_contexts = self.contexts.len();
        let mut clicked_context: Option<usize> = None;
        let mut double_clicked_context: Option<usize> = None;
        let mut delete_context: Option<usize> = None;
        let mut menu_action: Option<(usize, ContextMenuAction)> = None;

        for i in 0..num_contexts {
            let is_active = i == self.active_context;
            let is_renaming = self.renaming_context == Some(i);

            // Reserve the row rect first for the background interaction
            let row_rect = ui.cursor();
            let row_rect = Rect::from_min_size(row_rect.min, Vec2::new(sidebar_width, 26.0));

            // Create the row interaction FIRST so buttons painted later get priority
            let row_response = ui.interact(
                row_rect,
                egui::Id::new(("ctx_row", i)),
                egui::Sense::click(),
            );
            if row_response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            let hover = ui.rect_contains_pointer(row_rect);

            let inner = ui.allocate_ui_with_layout(
                Vec2::new(sidebar_width, 26.0),
                Layout::left_to_right(Align::Center),
                |ui| -> Option<egui::Response> {
                    let rect = ui.max_rect();

                    let fill = if is_active {
                        self.colors.bg_active
                    } else if hover {
                        self.colors.bg_hover
                    } else {
                        Color32::TRANSPARENT
                    };
                    ui.painter().rect_filled(rect, CornerRadius::ZERO, fill);

                    if is_active {
                        ui.painter().rect_filled(
                            Rect::from_min_size(rect.min, Vec2::new(3.0, rect.height())),
                            CornerRadius::ZERO,
                            self.colors.accent,
                        );
                    }

                    ui.add_space(20.0);

                    if is_renaming {
                        let te_id = egui::Id::new(("rename_ctx", i));
                        let te = ui.add(
                            egui::TextEdit::singleline(&mut self.rename_buffer)
                                .id(te_id)
                                .desired_width(sidebar_width - 56.0)
                                .font(egui::TextStyle::Body),
                        );
                        if te.lost_focus() {
                            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                self.renaming_context = None;
                            } else {
                                // Apply rename
                                let new_name = self.rename_buffer.trim().to_string();
                                if !new_name.is_empty() {
                                    self.contexts[i].name = new_name;
                                }
                                self.renaming_context = None;
                            }
                            // Consume Enter/Escape so it doesn't leak to the terminal
                            ui.input_mut(|i| {
                                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
                                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
                            });
                        }
                        // Auto-focus and select all on first frame
                        if te.gained_focus() || !te.has_focus() {
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
                        None
                    } else {
                        let text_color = if is_active {
                            self.colors.text_primary
                        } else {
                            self.colors.text_dim
                        };
                        if i < 9 {
                            ui.label(
                                RichText::new(format!("{}", i + 1))
                                    .size(11.0)
                                    .color(self.colors.text_dim),
                            );
                        }
                        let label_resp = ui
                            .add(
                                egui::Label::new(
                                    RichText::new(&self.contexts[i].name)
                                        .size(12.0)
                                        .color(text_color),
                                )
                                .sense(egui::Sense::click()),
                            )
                            .on_hover_cursor(egui::CursorIcon::PointingHand);

                        // Per-context notification badge.
                        // Active context: count visible notifs (context-scoped for active + globals).
                        // Inactive contexts: count only context-scoped notifs for that context
                        // (globals appear only on the active badge to avoid triple-counting).
                        let badge_count = if is_active {
                            self.visible_notification_count()
                        } else {
                            self.context_notification_count(i)
                        };
                        if badge_count > 0 && !hover {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.add_space(8.0);
                                let badge_text = if badge_count > 9 {
                                    "9+".to_string()
                                } else {
                                    badge_count.to_string()
                                };
                                ui.label(
                                    RichText::new(badge_text)
                                        .size(10.0)
                                        .color(self.colors.accent),
                                );
                            });
                        }

                        // Delete button on hover when 2+ contexts
                        if hover && num_contexts > 1 {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.add_space(8.0);
                                let x_btn = ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("\u{2715}")
                                                .size(13.0)
                                                .color(self.colors.text_dim),
                                        )
                                        .frame(false),
                                    )
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .on_hover_text("Delete context");
                                if x_btn.clicked() {
                                    delete_context = Some(i);
                                }
                            });
                        }
                        Some(label_resp)
                    }
                },
            );
            let label_response = inner.inner;

            if !is_renaming {
                // Only process row clicks if the delete button didn't consume the click
                if delete_context.is_none() {
                    row_response.context_menu(|ui| {
                        if ui.button("Rename").clicked() {
                            menu_action = Some((i, ContextMenuAction::Rename));
                            ui.close_menu();
                        }
                        ui.separator();
                        if i > 0 {
                            if ui.button("Move to Top").clicked() {
                                menu_action = Some((i, ContextMenuAction::MoveToTop));
                                ui.close_menu();
                            }
                            if ui.button("Move Up").clicked() {
                                menu_action = Some((i, ContextMenuAction::MoveUp));
                                ui.close_menu();
                            }
                        }
                        if i < num_contexts - 1 && ui.button("Move Down").clicked() {
                            menu_action = Some((i, ContextMenuAction::MoveDown));
                            ui.close_menu();
                        }
                        if num_contexts > 1 {
                            ui.separator();
                            if ui.button("Delete").clicked() {
                                menu_action = Some((i, ContextMenuAction::Delete));
                                ui.close_menu();
                            }
                        }
                    });

                    // Use label response for clicks on the name text (it has
                    // event priority over row_response in egui's interaction
                    // stack). Fall back to row_response for clicks on the row
                    // background outside the label.
                    let on_label = label_response.as_ref();
                    if on_label.is_some_and(|r| r.double_clicked()) {
                        double_clicked_context = Some(i);
                    } else if on_label.is_some_and(|r| r.clicked()) || row_response.clicked() {
                        clicked_context = Some(i);
                    }
                }
            }
        }

        // Handle collected actions after the loop
        if let Some((i, action)) = menu_action {
            match action {
                ContextMenuAction::Rename => {
                    self.renaming_context = Some(i);
                    self.rename_buffer = self.contexts[i].name.clone();
                }
                ContextMenuAction::MoveToTop => {
                    let ctx = self.contexts.remove(i);
                    self.contexts.insert(0, ctx);
                    if self.active_context == i {
                        self.active_context = 0;
                    } else if self.active_context < i {
                        self.active_context += 1;
                    }
                }
                ContextMenuAction::MoveUp => {
                    self.contexts.swap(i, i - 1);
                    if self.active_context == i {
                        self.active_context = i - 1;
                    } else if self.active_context == i - 1 {
                        self.active_context = i;
                    }
                }
                ContextMenuAction::MoveDown => {
                    self.contexts.swap(i, i + 1);
                    if self.active_context == i {
                        self.active_context = i + 1;
                    } else if self.active_context == i + 1 {
                        self.active_context = i;
                    }
                }
                ContextMenuAction::Delete => {
                    self.delete_context(i);
                }
            }
        } else if let Some(i) = delete_context {
            self.delete_context(i);
        } else if let Some(i) = double_clicked_context {
            self.renaming_context = Some(i);
            self.rename_buffer = self.contexts[i].name.clone();
        } else if let Some(i) = clicked_context {
            self.active_context = i;
        }

        if add_clicked {
            self.new_context();
        }
    }
}
