use crate::context::WindowMenuAction;
use egui::{Align, Color32, CornerRadius, Layout, Rect, RichText, Stroke, Vec2};

use crate::app::PlexiApp;

const ROW_HEIGHT: f32 = 26.0;

/// Returns the insertion slot (0 = before first row, n = after last row) that
/// the mouse Y is closest to, based on actual rendered rects.
fn drop_slot_from_rects(rects: &[Rect], mouse_y: f32) -> usize {
    for (i, rect) in rects.iter().enumerate() {
        if mouse_y <= rect.center().y {
            return i;
        }
    }
    rects.len()
}

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

        // Workspace list
        let num_contexts = self.contexts.len();
        let mut clicked_workspace: Option<usize> = None;
        let mut delete_context: Option<usize> = None;
        let mut menu_action: Option<(usize, WindowMenuAction)> = None;
        let mut row_rects: Vec<Rect> = Vec::with_capacity(num_contexts);
        let mut drag_released = false;

        for i in 0..num_contexts {
            let is_active = i == self.active_context;
            let is_renaming = self.renaming_window == Some(i);
            let is_dragging = self.drag_context == Some(i);
            let mut label_double_clicked = false;

            let row_rect = ui.cursor();
            let row_rect = Rect::from_min_size(row_rect.min, Vec2::new(sidebar_width, ROW_HEIGHT));
            row_rects.push(row_rect);

            let row_response = ui.interact(
                row_rect,
                egui::Id::new(("ctx_row", i)),
                egui::Sense::click_and_drag(),
            );

            if row_response.drag_started() && !is_renaming {
                self.drag_context = Some(i);
            }
            if row_response.drag_stopped() {
                drag_released = true;
            }

            let cursor_icon = if is_dragging {
                egui::CursorIcon::Grabbing
            } else if row_response.hovered() && !is_renaming {
                if self.drag_context.is_none() {
                    egui::CursorIcon::Grab
                } else {
                    egui::CursorIcon::Grabbing
                }
            } else {
                egui::CursorIcon::Default
            };
            if row_response.hovered() || is_dragging {
                ui.ctx().set_cursor_icon(cursor_icon);
            }

            let hover = ui.rect_contains_pointer(row_rect);
            let row_alpha = if is_dragging { 0.4 } else { 1.0 };

            ui.allocate_ui_with_layout(
                Vec2::new(sidebar_width, ROW_HEIGHT),
                Layout::left_to_right(Align::Center),
                |ui| {
                    let rect = ui.max_rect();

                    let fill = if is_active {
                        self.colors.bg_active
                    } else if hover && !is_dragging {
                        self.colors.bg_hover
                    } else {
                        Color32::TRANSPARENT
                    };
                    let fill = Color32::from_rgba_unmultiplied(
                        fill.r(),
                        fill.g(),
                        fill.b(),
                        (fill.a() as f32 * row_alpha) as u8,
                    );
                    ui.painter().rect_filled(rect, CornerRadius::ZERO, fill);

                    if is_active {
                        let accent = self.colors.accent;
                        let accent = Color32::from_rgba_unmultiplied(
                            accent.r(),
                            accent.g(),
                            accent.b(),
                            (accent.a() as f32 * row_alpha) as u8,
                        );
                        ui.painter().rect_filled(
                            Rect::from_min_size(rect.min, Vec2::new(3.0, rect.height())),
                            CornerRadius::ZERO,
                            accent,
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
                                self.renaming_window = None;
                            } else {
                                let new_name = self.rename_buffer.trim().to_string();
                                if !new_name.is_empty() {
                                    self.contexts[i].name = new_name;
                                }
                                self.renaming_window = None;
                            }
                            ui.input_mut(|i| {
                                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
                                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
                            });
                        }
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
                    } else {
                        let text_color = if is_active {
                            self.colors.text_primary
                        } else {
                            self.colors.text_dim
                        };
                        let text_color = Color32::from_rgba_unmultiplied(
                            text_color.r(),
                            text_color.g(),
                            text_color.b(),
                            (text_color.a() as f32 * row_alpha) as u8,
                        );
                        if i < 9 {
                            ui.label(
                                RichText::new(format!("{}", i + 1))
                                    .size(11.0)
                                    .color(self.colors.text_dim),
                            );
                        }
                        let label_resp = ui.add(
                            egui::Label::new(
                                RichText::new(&self.contexts[i].name)
                                    .size(12.0)
                                    .color(text_color),
                            )
                            .sense(egui::Sense::click()),
                        );
                        if label_resp.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
                        }
                        if label_resp.double_clicked() {
                            label_double_clicked = true;
                        }

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

                        // Delete button on hover when 2+ contexts (suppress during drag)
                        if hover && num_contexts > 1 && self.drag_context.is_none() {
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
                    }
                },
            );

            if !is_renaming {
                if delete_context.is_none() {
                    row_response.context_menu(|ui| {
                        if ui.button("Rename").clicked() {
                            menu_action = Some((i, WindowMenuAction::Rename));
                            ui.close_menu();
                        }
                        ui.separator();
                        if i > 0 {
                            if ui.button("Move to Top").clicked() {
                                menu_action = Some((i, WindowMenuAction::MoveToTop));
                                ui.close_menu();
                            }
                            if ui.button("Move Up").clicked() {
                                menu_action = Some((i, WindowMenuAction::MoveUp));
                                ui.close_menu();
                            }
                        }
                        if i < num_contexts - 1 {
                            if ui.button("Move Down").clicked() {
                                menu_action = Some((i, WindowMenuAction::MoveDown));
                                ui.close_menu();
                            }
                            if ui.button("Move to Bottom").clicked() {
                                menu_action = Some((i, WindowMenuAction::MoveToBottom));
                                ui.close_menu();
                            }
                        }
                        if num_contexts > 1 {
                            ui.separator();
                            if ui.button("Delete").clicked() {
                                menu_action = Some((i, WindowMenuAction::Delete));
                                ui.close_menu();
                            }
                        }
                    });

                    // Double-click on the text label → rename; single click anywhere → switch context
                    if label_double_clicked && self.drag_context.is_none() {
                        menu_action = Some((i, WindowMenuAction::Rename));
                    } else if row_response.clicked() && self.drag_context.is_none() {
                        clicked_workspace = Some(i);
                    }
                }
            }
        }

        // Compute drop slot from actual rendered rects + current mouse position.
        // This is done after the loop so we use the real geometry, not row-height math.
        let drop_index: Option<usize> = if self.drag_context.is_some() {
            ui.input(|i| i.pointer.hover_pos())
                .map(|pos| drop_slot_from_rects(&row_rects, pos.y))
        } else {
            None
        };

        // Handle drag release: perform the reorder then clear drag state.
        if drag_released {
            if let (Some(src), Some(dst)) = (self.drag_context, drop_index) {
                if dst != src && dst != src + 1 {
                    let effective_dst = if dst > src { dst - 1 } else { dst };
                    let ctx = self.contexts.remove(src);
                    self.contexts.insert(effective_dst, ctx);
                    if self.active_context == src {
                        self.active_context = effective_dst;
                    } else if src < self.active_context && effective_dst >= self.active_context {
                        self.active_context -= 1;
                    } else if src > self.active_context && effective_dst <= self.active_context {
                        self.active_context += 1;
                    }
                }
            }
            self.drag_context = None;
        }

        // Draw the drop indicator line at the actual gap between rects.
        if let (Some(src), Some(dst)) = (self.drag_context, drop_index) {
            if dst != src && dst != src + 1 {
                let line_y = if dst == 0 {
                    row_rects[0].min.y
                } else if dst >= row_rects.len() {
                    row_rects[row_rects.len() - 1].max.y
                } else {
                    row_rects[dst - 1].max.y
                };
                let x0 = row_rects[0].min.x;
                ui.painter().line_segment(
                    [egui::pos2(x0, line_y), egui::pos2(x0 + sidebar_width, line_y)],
                    Stroke::new(2.0, self.colors.accent),
                );
            }
        }

        // Handle collected actions after the loop
        if let Some((i, action)) = menu_action {
            match action {
                WindowMenuAction::Rename => {
                    self.renaming_window = Some(i);
                    self.rename_buffer = self.contexts[i].name.clone();
                }
                WindowMenuAction::MoveToTop => {
                    let ctx = self.contexts.remove(i);
                    self.contexts.insert(0, ctx);
                    if self.active_context == i {
                        self.active_context = 0;
                    } else if self.active_context < i {
                        self.active_context += 1;
                    }
                }
                WindowMenuAction::MoveUp => {
                    self.contexts.swap(i, i - 1);
                    if self.active_context == i {
                        self.active_context = i - 1;
                    } else if self.active_context == i - 1 {
                        self.active_context = i;
                    }
                }
                WindowMenuAction::MoveDown => {
                    self.contexts.swap(i, i + 1);
                    if self.active_context == i {
                        self.active_context = i + 1;
                    } else if self.active_context == i + 1 {
                        self.active_context = i;
                    }
                }
                WindowMenuAction::MoveToBottom => {
                    let last = num_contexts - 1;
                    let ctx = self.contexts.remove(i);
                    self.contexts.push(ctx);
                    if self.active_context == i {
                        self.active_context = last;
                    } else if self.active_context > i {
                        self.active_context -= 1;
                    }
                }
                WindowMenuAction::Delete => {
                    self.delete_context(i);
                }
            }
        } else if let Some(i) = delete_context {
            self.delete_context(i);
        } else if let Some(i) = clicked_workspace {
            self.switch_workspace(i);
        }

        if add_clicked {
            self.new_context();
        }
    }
}
