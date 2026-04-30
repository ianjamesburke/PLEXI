use crate::context::ContextMenuAction;
use egui::{Align, Color32, CornerRadius, Layout, Rect, RichText, Stroke, Vec2};

use crate::app::PlexiApp;

const ROW_HEIGHT: f32 = 26.0;

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
        let num_workspaces = self.workspaces.len();
        let mut clicked_workspace: Option<usize> = None;
        let mut delete_workspace: Option<usize> = None;
        let mut menu_action: Option<(usize, ContextMenuAction)> = None;

        // Record the Y position where the first row starts, for drop-target math.
        let list_top_y = ui.cursor().min.y;

        // Compute drop target index from mouse position while a drag is active.
        let drop_index: Option<usize> = if self.drag_context.is_some() {
            ui.input(|i| i.pointer.hover_pos()).map(|pos| {
                let slot = ((pos.y - list_top_y) / ROW_HEIGHT).round() as isize;
                slot.clamp(0, num_workspaces as isize) as usize
            })
        } else {
            None
        };

        for i in 0..num_workspaces {
            let is_active = i == self.active_workspace;
            let is_renaming = self.renaming_context == Some(i);
            let is_dragging = self.drag_context == Some(i);

            let row_rect = ui.cursor();
            let row_rect = Rect::from_min_size(row_rect.min, Vec2::new(sidebar_width, ROW_HEIGHT));

            let row_response = ui.interact(
                row_rect,
                egui::Id::new(("ctx_row", i)),
                egui::Sense::click_and_drag(),
            );

            // Start drag
            if row_response.drag_started() && !is_renaming {
                self.drag_context = Some(i);
            }

            // Release drag: perform reorder
            if row_response.drag_stopped() {
                if let (Some(src), Some(dst)) = (self.drag_context, drop_index) {
                    if dst != src && dst != src + 1 {
                        let effective_dst = if dst > src { dst - 1 } else { dst };
                        let ws = self.workspaces.remove(src);
                        self.workspaces.insert(effective_dst, ws);
                        if self.active_workspace == src {
                            self.active_workspace = effective_dst;
                        } else if src < self.active_workspace && effective_dst >= self.active_workspace {
                            self.active_workspace -= 1;
                        } else if src > self.active_workspace && effective_dst <= self.active_workspace {
                            self.active_workspace += 1;
                        }
                    }
                }
                self.drag_context = None;
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

            // Dim the row being dragged
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
                                self.renaming_context = None;
                            } else {
                                let new_name = self.rename_buffer.trim().to_string();
                                if !new_name.is_empty() {
                                    self.workspaces[i].name = new_name;
                                }
                                self.renaming_context = None;
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
                        ui.add(
                            egui::Label::new(
                                RichText::new(&self.workspaces[i].name)
                                    .size(12.0)
                                    .color(text_color),
                            )
                            .sense(egui::Sense::hover()),
                        );

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
                        if hover && num_workspaces > 1 && self.drag_context.is_none() {
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
                                    delete_workspace = Some(i);
                                }
                            });
                        }
                    }
                },
            );

            if !is_renaming {
                if delete_workspace.is_none() {
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
                        if i < num_workspaces - 1 {
                            if ui.button("Move Down").clicked() {
                                menu_action = Some((i, ContextMenuAction::MoveDown));
                                ui.close_menu();
                            }
                            if ui.button("Move to Bottom").clicked() {
                                menu_action = Some((i, ContextMenuAction::MoveToBottom));
                                ui.close_menu();
                            }
                        }
                        if num_workspaces > 1 {
                            ui.separator();
                            if ui.button("Delete").clicked() {
                                menu_action = Some((i, ContextMenuAction::Delete));
                                ui.close_menu();
                            }
                        }
                    });

                    // Only switch context on click, not on drag release
                    if row_response.clicked() && self.drag_context.is_none() {
                        clicked_workspace = Some(i);
                    }
                }
            }
        }

        // Draw the drop indicator line while dragging
        if let (Some(src), Some(dst)) = (self.drag_context, drop_index) {
            if dst != src && dst != src + 1 {
                let line_y = list_top_y + dst as f32 * ROW_HEIGHT;
                let x0 = ui.cursor().min.x;
                ui.painter().line_segment(
                    [egui::pos2(x0, line_y), egui::pos2(x0 + sidebar_width, line_y)],
                    Stroke::new(2.0, self.colors.accent),
                );
            }
        }

        // Handle collected actions after the loop
        if let Some((i, action)) = menu_action {
            match action {
                ContextMenuAction::Rename => {
                    self.renaming_context = Some(i);
                    self.rename_buffer = self.workspaces[i].name.clone();
                }
                ContextMenuAction::MoveToTop => {
                    let ws = self.workspaces.remove(i);
                    self.workspaces.insert(0, ws);
                    if self.active_workspace == i {
                        self.active_workspace = 0;
                    } else if self.active_workspace < i {
                        self.active_workspace += 1;
                    }
                }
                ContextMenuAction::MoveUp => {
                    self.workspaces.swap(i, i - 1);
                    if self.active_workspace == i {
                        self.active_workspace = i - 1;
                    } else if self.active_workspace == i - 1 {
                        self.active_workspace = i;
                    }
                }
                ContextMenuAction::MoveDown => {
                    self.workspaces.swap(i, i + 1);
                    if self.active_workspace == i {
                        self.active_workspace = i + 1;
                    } else if self.active_workspace == i + 1 {
                        self.active_workspace = i;
                    }
                }
                ContextMenuAction::MoveToBottom => {
                    let last = num_workspaces - 1;
                    let ws = self.workspaces.remove(i);
                    self.workspaces.push(ws);
                    if self.active_workspace == i {
                        self.active_workspace = last;
                    } else if self.active_workspace > i {
                        self.active_workspace -= 1;
                    }
                }
                ContextMenuAction::Delete => {
                    self.delete_workspace(i);
                }
            }
        } else if let Some(i) = delete_workspace {
            self.delete_workspace(i);
        } else if let Some(i) = clicked_workspace {
            self.switch_workspace(i);
        }

        if add_clicked {
            self.new_context();
        }
    }
}
