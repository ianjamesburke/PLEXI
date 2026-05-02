use crate::context::WindowMenuAction;
use crate::sidebar_row::{with_alpha, SidebarRow, ROW_HEIGHT};
use egui::{Align, CornerRadius, Layout, Rect, RichText, Stroke, Vec2};

use crate::app::PlexiApp;

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

        // Header
        ui.add_space(8.0);
        let mut add_clicked = false;
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.label(RichText::new("Contexts").size(10.0).color(self.colors.text_section));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(12.0);
                if ui
                    .add(egui::Button::new(RichText::new("+").size(12.0).color(self.colors.text_dim)).frame(false))
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text("New context")
                    .clicked()
                {
                    add_clicked = true;
                }
            });
        });
        ui.add_space(4.0);

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
            let any_dragging = self.drag_context.is_some();

            // --- Renaming: special-cased before SidebarRow path ---
            if is_renaming {
                let origin = ui.cursor().min;
                let row_rect = Rect::from_min_size(origin, Vec2::new(sidebar_width, ROW_HEIGHT));
                row_rects.push(row_rect);
                ui.allocate_space(Vec2::new(sidebar_width, ROW_HEIGHT));

                let fill = if is_active { self.colors.bg_active } else { self.colors.bg_hover };
                ui.painter().rect_filled(row_rect, CornerRadius::ZERO, fill);
                if is_active {
                    ui.painter().rect_filled(
                        Rect::from_min_size(row_rect.min, Vec2::new(3.0, ROW_HEIGHT)),
                        CornerRadius::ZERO,
                        self.colors.accent,
                    );
                }

                let te_id = egui::Id::new(("rename_ctx", i));
                let text_rect = row_rect.shrink2(egui::vec2(20.0, 2.0));
                ui.allocate_new_ui(
                    egui::UiBuilder::new()
                        .max_rect(text_rect)
                        .layout(Layout::left_to_right(Align::Center)),
                    |ui| {
                        let te = ui.add(
                            egui::TextEdit::singleline(&mut self.rename_buffer)
                                .id(te_id)
                                .desired_width(sidebar_width - 56.0)
                                .font(egui::TextStyle::Body),
                        );
                        if te.lost_focus() {
                            if ui.input(|inp| inp.key_pressed(egui::Key::Escape)) {
                                self.renaming_window = None;
                            } else {
                                let new_name = self.rename_buffer.trim().to_string();
                                if !new_name.is_empty() {
                                    self.contexts[i].name = new_name;
                                }
                                self.renaming_window = None;
                            }
                            ui.input_mut(|inp| {
                                inp.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
                                inp.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
                            });
                        }
                        if te.gained_focus() || !te.has_focus() {
                            te.request_focus();
                            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), te_id) {
                                state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                                    egui::text::CCursor::new(0),
                                    egui::text::CCursor::new(self.rename_buffer.len()),
                                )));
                                state.store(ui.ctx(), te_id);
                            }
                        }
                    },
                );
                continue;
            }

            // --- Normal row via SidebarRow ---
            // Zones are declared here, before any rendering.
            let row = SidebarRow::new(ui, sidebar_width, num_contexts > 1 && !any_dragging)
                .active(is_active)
                .dragging(is_dragging, any_dragging);

            // Snapshot the layout for drop-indicator tracking (must be before draw()).
            let row_full = row.layout.full;

            // Prepare content data (borrows resolved before the closure).
            let text_color = with_alpha(
                if is_active { self.colors.text_primary } else { self.colors.text_dim },
                if is_dragging { 0.4 } else { 1.0 },
            );
            let ctx_name = self.contexts[i].name.clone();
            let badge_count = if is_active {
                self.visible_notification_count()
            } else {
                self.context_notification_count(i)
            };
            let dim_color = self.colors.text_dim;
            let accent_color = self.colors.accent;

            let result = row.draw(
                ui,
                egui::Id::new(("ctx", i)),
                &self.colors,
                |row_ui, hovered| {
                    row_ui.add_space(20.0);
                    if i < 9 {
                        row_ui.label(
                            RichText::new(format!("{}", i + 1)).size(11.0).color(dim_color),
                        );
                    }
                    row_ui.add(
                        egui::Label::new(RichText::new(&ctx_name).size(12.0).color(text_color))
                            .selectable(false),
                    );
                    // Badge — only when not hovering (X button takes that space on hover)
                    if badge_count > 0 && !hovered {
                        row_ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.add_space(8.0);
                            let badge_text = if badge_count > 9 { "9+".to_string() } else { badge_count.to_string() };
                            ui.label(RichText::new(badge_text).size(10.0).color(accent_color));
                        });
                    }
                },
            );

            row_rects.push(row_full);

            if result.drag_started { self.drag_context = Some(i); }
            if result.drag_stopped { drag_released = true; }

            if delete_context.is_none() {
                let num_ctxs = num_contexts;
                result.drag_response.context_menu(|ui| {
                    if ui.button("Rename").clicked() {
                        menu_action = Some((i, WindowMenuAction::Rename));
                        ui.close_menu();
                    }
                    ui.separator();
                    if i > 0 {
                        if ui.button("Move to Top").clicked() { menu_action = Some((i, WindowMenuAction::MoveToTop)); ui.close_menu(); }
                        if ui.button("Move Up").clicked() { menu_action = Some((i, WindowMenuAction::MoveUp)); ui.close_menu(); }
                    }
                    if i < num_ctxs - 1 {
                        if ui.button("Move Down").clicked() { menu_action = Some((i, WindowMenuAction::MoveDown)); ui.close_menu(); }
                        if ui.button("Move to Bottom").clicked() { menu_action = Some((i, WindowMenuAction::MoveToBottom)); ui.close_menu(); }
                    }
                    if num_ctxs > 1 {
                        if i > 0 || i < num_ctxs - 1 { ui.separator(); }
                        if ui.button("Delete").clicked() { menu_action = Some((i, WindowMenuAction::Delete)); ui.close_menu(); }
                    }
                });

                if result.action_clicked {
                    delete_context = Some(i);
                } else if result.primary_double_clicked && !any_dragging {
                    menu_action = Some((i, WindowMenuAction::Rename));
                } else if result.primary_clicked && !any_dragging {
                    clicked_workspace = Some(i);
                }
            }
        }

        // Drop slot + drag reorder
        let drop_index: Option<usize> = if self.drag_context.is_some() {
            ui.input(|i| i.pointer.hover_pos())
                .map(|pos| drop_slot_from_rects(&row_rects, pos.y))
        } else {
            None
        };

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

        if let Some((i, action)) = menu_action {
            match action {
                WindowMenuAction::Rename => {
                    self.renaming_window = Some(i);
                    self.rename_buffer = self.contexts[i].name.clone();
                }
                WindowMenuAction::MoveToTop => {
                    let ctx = self.contexts.remove(i);
                    self.contexts.insert(0, ctx);
                    if self.active_context == i { self.active_context = 0; }
                    else if self.active_context < i { self.active_context += 1; }
                }
                WindowMenuAction::MoveUp => {
                    self.contexts.swap(i, i - 1);
                    if self.active_context == i { self.active_context = i - 1; }
                    else if self.active_context == i - 1 { self.active_context = i; }
                }
                WindowMenuAction::MoveDown => {
                    self.contexts.swap(i, i + 1);
                    if self.active_context == i { self.active_context = i + 1; }
                    else if self.active_context == i + 1 { self.active_context = i; }
                }
                WindowMenuAction::MoveToBottom => {
                    let last = num_contexts - 1;
                    let ctx = self.contexts.remove(i);
                    self.contexts.push(ctx);
                    if self.active_context == i { self.active_context = last; }
                    else if self.active_context > i { self.active_context -= 1; }
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
