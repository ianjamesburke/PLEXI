use crate::context::WindowMenuAction;
use egui::{Align, Color32, CornerRadius, Layout, Rect, RichText, Sense, Stroke, Vec2};

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
            let is_dragging = self.drag_context == Some(i);

            let row_rect = {
                let c = ui.cursor();
                Rect::from_min_size(c.min, Vec2::new(sidebar_width, ROW_HEIGHT))
            };
            row_rects.push(row_rect);

            let hover = ui.rect_contains_pointer(row_rect);
            let row_alpha = if is_dragging { 0.4_f32 } else { 1.0_f32 };

            let row_response = ui
                .allocate_ui_with_layout(
                    Vec2::new(sidebar_width, ROW_HEIGHT),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        let row_resp = ui.interact(
                            ui.max_rect(),
                            egui::Id::new(("ctx_row", i)),
                            Sense::click_and_drag(),
                        );

                        let rect = ui.max_rect();

                        // Background
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

                        // Active accent bar
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
                        ui.add(
                            egui::Label::new(
                                RichText::new(&self.contexts[i].name)
                                    .size(12.0)
                                    .color(text_color),
                            )
                            .sense(Sense::hover()),
                        );

                        // Notification badge
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

                        // Delete button — hover only, not during drag, 2+ contexts
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

                        row_resp
                    },
                )
                .inner;

            if row_response.drag_started() {
                self.drag_context = Some(i);
            }
            if row_response.drag_stopped() {
                drag_released = true;
            }

            if hover || is_dragging {
                let icon = if is_dragging || self.drag_context.is_some() {
                    egui::CursorIcon::Grabbing
                } else {
                    egui::CursorIcon::Grab
                };
                ui.ctx().set_cursor_icon(icon);
            }

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
                        if i > 0 || i < num_contexts - 1 {
                            ui.separator();
                        }
                        if ui.button("Delete").clicked() {
                            menu_action = Some((i, WindowMenuAction::Delete));
                            ui.close_menu();
                        }
                    }
                });

                if row_response.clicked() && self.drag_context.is_none() {
                    clicked_workspace = Some(i);
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
