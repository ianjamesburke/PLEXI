use crate::shell;
use egui::{Align, Align2, Color32, CornerRadius, Layout, Rect, RichText, Stroke, Vec2};

use crate::app::PlexiApp;
use crate::overlays::MODAL_WIDTH;

impl PlexiApp {
    pub(crate) fn draw_command_palette(&mut self, ctx: &egui::Context) {
        // Build entries from all contexts
        let mut entries: Vec<(usize, String, egui_tiles::TileId, u64, String, String)> = Vec::new();
        for (ci, context) in self.contexts.iter().enumerate() {
            for (&pane_id, pane) in &context.panes {
                let Some(display_name) = pane.name.clone() else {
                    continue;
                };
                let cwd = shell::get_pid_cwd(pane.backend.child_pid())
                    .map(|p| {
                        let s = p.display().to_string();
                        if let Some(home) = dirs::home_dir() {
                            s.strip_prefix(&home.display().to_string())
                                .map(|rest| format!("~{rest}"))
                                .unwrap_or(s)
                        } else {
                            s
                        }
                    })
                    .unwrap_or_default();
                // Find the tile_id for this pane
                if let Some(tile_id) = context.tree.tiles.find_pane(&pane_id) {
                    entries.push((ci, context.name.clone(), tile_id, pane_id, display_name, cwd));
                }
            }
        }

        // Filter by query
        let query = self.palette_query.to_lowercase();
        let filtered: Vec<_> = entries
            .into_iter()
            .filter(|(_, ctx_name, _, _, name, cwd)| {
                if query.is_empty() {
                    return true;
                }
                name.to_lowercase().contains(&query)
                    || ctx_name.to_lowercase().contains(&query)
                    || cwd.to_lowercase().contains(&query)
            })
            .collect();

        // Clamp selection
        if self.palette_selected >= filtered.len() && !filtered.is_empty() {
            self.palette_selected = filtered.len() - 1;
        }

        // Handle keyboard nav before rendering
        let mut jump_to: Option<(usize, egui_tiles::TileId)> = None;
        ctx.input_mut(|input| {
            if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                self.show_command_palette = false;
            }
            if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)
                && !filtered.is_empty()
                && self.palette_selected < filtered.len() - 1
            {
                self.palette_selected += 1;
            }
            if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)
                && self.palette_selected > 0
            {
                self.palette_selected -= 1;
            }
            if input.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
                if let Some(entry) = filtered.get(self.palette_selected) {
                    jump_to = Some((entry.0, entry.2));
                }
            }
        });

        if let Some((ctx_idx, tile_id)) = jump_to {
            self.active_context = ctx_idx;
            self.contexts[ctx_idx].focused_pane = Some(tile_id);
            self.contexts[ctx_idx].zoomed_pane = None;
            self.contexts[ctx_idx].activate_tab_for(tile_id);
            self.show_command_palette = false;
            return;
        }

        if !self.show_command_palette {
            return;
        }

        // Render scrim
        let screen_rect = ctx.screen_rect();
        egui::Area::new(egui::Id::new("palette_scrim"))
            .fixed_pos(screen_rect.min)
            .show(ctx, |ui| {
                ui.painter().rect_filled(
                    screen_rect,
                    0.0,
                    Color32::from_black_alpha(120),
                );
                // Consume clicks on scrim to close
                let scrim_response = ui.allocate_rect(screen_rect, egui::Sense::click());
                if scrim_response.clicked() {
                    self.show_command_palette = false;
                }
            });

        // Render palette
        egui::Area::new(egui::Id::new("command_palette"))
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

                        // Search input
                        let te_id = egui::Id::new("palette_search");
                        let te = ui.add(
                            egui::TextEdit::singleline(&mut self.palette_query)
                                .id(te_id)
                                .desired_width(MODAL_WIDTH)
                                .hint_text("Jump to pane...")
                                .font(egui::TextStyle::Body),
                        );
                        if !te.has_focus() {
                            te.request_focus();
                        }

                        // Reset selection when query changes
                        if te.changed() {
                            self.palette_selected = 0;
                        }

                        ui.add_space(6.0);

                        // Results list
                        let current_ctx = self.active_context;
                        let current_focused = self.contexts[self.active_context].focused_pane;
                        for (i, (ci, ctx_name, tile_id, _pane_id, name, cwd)) in
                            filtered.iter().enumerate()
                        {
                            let is_selected = i == self.palette_selected;
                            let is_current = *ci == current_ctx
                                && current_focused == Some(*tile_id);

                            let fill = if is_selected {
                                self.colors.bg_active
                            } else {
                                Color32::TRANSPARENT
                            };

                            let row_rect = ui.cursor();
                            let row_rect = Rect::from_min_size(
                                row_rect.min,
                                Vec2::new(400.0, 36.0),
                            );
                            ui.painter()
                                .rect_filled(row_rect, CornerRadius::same(4), fill);

                            ui.allocate_ui_with_layout(
                                Vec2::new(400.0, 36.0),
                                Layout::left_to_right(Align::Center),
                                |ui| {
                                    ui.add_space(8.0);
                                    ui.vertical(|ui| {
                                        ui.add_space(2.0);
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new(ctx_name)
                                                    .size(10.0)
                                                    .color(self.colors.text_dim),
                                            );
                                            ui.label(
                                                RichText::new("\u{203A}")
                                                    .size(10.0)
                                                    .color(self.colors.text_dim),
                                            );
                                            let name_color = if is_current {
                                                self.colors.accent
                                            } else {
                                                self.colors.text_primary
                                            };
                                            ui.label(
                                                RichText::new(name)
                                                    .size(12.0)
                                                    .color(name_color),
                                            );
                                        });
                                        if !cwd.is_empty() {
                                            ui.label(
                                                RichText::new(cwd)
                                                    .size(9.0)
                                                    .color(self.colors.text_dim),
                                            );
                                        }
                                    });
                                },
                            );

                            // Click to jump
                            let click_response =
                                ui.interact(row_rect, egui::Id::new(("palette_row", i)), egui::Sense::click());
                            if click_response.clicked() {
                                self.active_context = *ci;
                                self.contexts[*ci].focused_pane = Some(*tile_id);
                                self.contexts[*ci].zoomed_pane = None;
                                self.contexts[*ci].activate_tab_for(*tile_id);
                                self.show_command_palette = false;
                            }
                            if click_response.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                        }

                        if filtered.is_empty() {
                            ui.label(
                                RichText::new("No matching panes")
                                    .size(11.0)
                                    .color(self.colors.text_dim),
                            );
                        }
                    });
            });
    }
}
