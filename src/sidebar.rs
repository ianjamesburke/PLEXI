use crate::context::WindowMenuAction;
use crate::sidebar_row::{SidebarAction, ContextItem, PaneDots};
use egui::{Align, CornerRadius, Layout, Rect, RichText, Stroke, Vec2};
use egui_tiles::Tile;

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

        // Breadcrumb bar — only shown when navigated into a sub-context.
        if self.router.current_depth() > 0 {
            ui.horizontal(|ui| {
                ui.add_space(16.0);
                let mut path_ids: Vec<u64> = self.router.depth_stack.iter()
                    .map(|(ctx_id, _, _)| *ctx_id)
                    .collect();
                path_ids.push(self.router.active().context_id);

                let dim = path_ids.len() >= 3;
                let truncate = path_ids.len() >= 5;

                // Indices into path_ids to display. usize::MAX is the ellipsis placeholder.
                let display_indices: Vec<usize> = if truncate {
                    let last = path_ids.len() - 1;
                    vec![0, usize::MAX, last - 1, last]
                } else {
                    (0..path_ids.len()).collect()
                };

                for (pos, &idx) in display_indices.iter().enumerate() {
                    if idx == usize::MAX {
                        ui.label(RichText::new("…").color(self.colors.text_dim));
                    } else {
                        let name = self.router.iter()
                            .find(|c| c.context_id == path_ids[idx])
                            .map(|c| c.name.as_str())
                            .unwrap_or("?");
                        let text = if dim {
                            RichText::new(name).color(self.colors.text_dim)
                        } else {
                            RichText::new(name)
                        };
                        let _ = ui.small_button(text);
                    }
                    if pos < display_indices.len() - 1 {
                        ui.label(RichText::new("\u{203A}").color(self.colors.text_dim));
                    }
                }
            });
            ui.separator();
        }

        let num_contexts = self.router.len();

        // Pre-calculate pane rows once if any context is expanded — avoids O(n) calls inside loop.
        let pane_groups = if !self.sidebar_expanded_contexts.is_empty() {
            Some(self.collect_inspector_rows().0)
        } else {
            None
        };

        let mut delete_context: Option<usize> = None;
        let mut menu_action: Option<(usize, WindowMenuAction)> = None;
        let mut row_rects: Vec<Rect> = Vec::with_capacity(num_contexts);
        let mut drag_released = false;

        let focused_cwd = self.windows.get(self.active_window)
            .and_then(|w| w.focused_pane.map(|tile| (w, tile)))
            .and_then(|(w, tile)| w.get_focused_pane_cwd(tile));

        for i in 0..num_contexts {
            let is_active = i == self.router.active_idx();
            let is_renaming = self.renaming_window == Some(i);
            let is_dragging = self.drag_context == Some(i);
            let any_dragging = self.drag_context.is_some();


            // Pane count + focused-dot index for this context
            let ctx_id = self.router.get(i).context_id;
            let mut pane_ids: Vec<u64> = self.windows.iter()
                .filter(|w| w.context_id == ctx_id)
                .flat_map(|w| w.panes.keys().copied())
                .collect();
            pane_ids.sort_unstable();
            let pane_count = pane_ids.len();

            let focused_dot_idx: Option<usize> = if is_active {
                self.windows.get(self.active_window)
                    .and_then(|w| w.focused_pane)
                    .and_then(|tile_id| {
                        let w = &self.windows[self.active_window];
                        match w.tree.tiles.get(tile_id) {
                            Some(Tile::Pane(pid)) => pane_ids.iter().position(|&p| p == *pid),
                            _ => None,
                        }
                    })
            } else {
                None
            };

            // --- Renaming: special-cased before ContextItem path ---
            if is_renaming {
                let fill = if is_active { self.colors.bg_active } else { self.colors.bg_sidebar_hover };
                let bg_idx = ui.painter().add(egui::Shape::Noop);
                let te_id = egui::Id::new(("rename_ctx", i));
                let accent = self.colors.accent;
                let sidebar_w = sidebar_width;
                let scope = ui.scope(|ui| {
                    ui.set_width(ui.available_width());
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.add_space(20.0);
                        let te = ui.scope(|ui| {
                            ui.visuals_mut().text_cursor.stroke.width = 1.5;
                            ui.visuals_mut().text_cursor.stroke.color = accent;
                            ui.add(
                                egui::TextEdit::singleline(&mut self.rename_buffer)
                                    .id(te_id)
                                    .desired_width(sidebar_w - 56.0)
                                    .font(egui::TextStyle::Body),
                            )
                        }).inner;
                        if te.lost_focus() {
                            if ui.input(|inp| inp.key_pressed(egui::Key::Escape)) {
                                self.renaming_window = None;
                            } else {
                                let new_name = self.rename_buffer.trim().to_string();
                                if !new_name.is_empty() {
                                    self.router.get_mut(i).name = new_name;
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
                    });
                    ui.add_space(4.0);
                });
                let row_rect = scope.response.rect;
                row_rects.push(row_rect);
                ui.painter().set(bg_idx, egui::Shape::rect_filled(row_rect, CornerRadius::ZERO, fill));
                if is_active {
                    ui.painter().rect_filled(
                        Rect::from_min_size(row_rect.min, Vec2::new(3.0, row_rect.height())),
                        CornerRadius::ZERO,
                        self.colors.accent,
                    );
                }
                continue;
            }

            // --- Normal row via ContextItem ---
            let ctx_name = self.router.get(i).name.clone();
            let ctx_depth = {
                let ctx = self.router.get(i);
                let active_ctx = self.router.active();
                if ctx.parent_id == Some(active_ctx.context_id) {
                    1u32 // direct child of current zoom level
                } else {
                    0u32 // at current level or ancestor
                }
            };
            let badge_count = if is_active {
                self.visible_notification_count()
            } else {
                self.context_notification_count(i)
            };
            let subtitle = self.router.get(i).root.as_ref()
                .map(|p| p.display().to_string());

            let (action, response) = ContextItem {
                is_active,
                is_dragging,
                any_dragging,
                action_enabled: num_contexts > 1 && !any_dragging,
                ctx_depth,
                ctx_name,
                ctx_index: Some(i),
                badge_count,
                subtitle,
                pane_dots: Some(PaneDots { count: pane_count, focused_idx: focused_dot_idx }),
            }.draw(ui, egui::Id::new(("ctx", i)), &self.colors);

            row_rects.push(response.rect);

            // Inline pane list when expanded
            let is_expanded = self.sidebar_expanded_contexts.contains(&ctx_id);
            if is_expanded {
                if let Some(groups) = &pane_groups {
                if let Some((_, _, rows)) = groups.iter().find(|(_, cid, _)| *cid == ctx_id) {
                    for row in rows {
                        let row_id = row.id;
                        let row_name: &str = if row.name.is_empty() { row.kind } else { &row.name };
                        ui.horizontal(|ui| {
                            ui.add_space(32.0);
                            crate::widgets::pane_type_badge(ui, row.kind, &self.colors);
                            let click = ui.add(
                                egui::Label::new(
                                    egui::RichText::new(row_name)
                                        .size(11.0)
                                        .color(self.colors.text_dim)
                                )
                                .selectable(false)
                                .sense(egui::Sense::click()),
                            );
                            crate::widgets::status_chip(ui, row.status, &self.colors);
                            if click.clicked() {
                                self.pane_navigate(row_id);
                                log::info!("sidebar: pane row clicked — focusing pane {row_id}");
                            }
                        });
                    }
                } // if let Some groups
                }
            }

            match action {
                SidebarAction::DragStart => { self.drag_context = Some(i); }
                SidebarAction::DragEnd => { drag_released = true; }
                _ => {}
            }

            if delete_context.is_none() {
                let num_ctxs = num_contexts;
                let cwd_for_menu = focused_cwd.clone();
                let has_root = self.router.get(i).root.is_some();
                response.context_menu(|ui| {
                    if ui.button("Rename").clicked() {
                        menu_action = Some((i, WindowMenuAction::Rename));
                        ui.close_menu();
                    }
                    if ui.button("Edit Description").clicked() {
                        menu_action = Some((i, WindowMenuAction::EditDescription));
                        ui.close_menu();
                    }
                    if ui.button("New sub-context").clicked() {
                        menu_action = Some((i, WindowMenuAction::NewSubContext));
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
                    ui.separator();
                    if let Some(cwd) = &cwd_for_menu {
                        if ui.button("Set root to current path").clicked() {
                            menu_action = Some((i, WindowMenuAction::SetRoot(cwd.clone())));
                            ui.close_menu();
                        }
                    }
                    {
                        let label = if has_root { "Edit root\u{2026}" } else { "Set root\u{2026}" };
                        if ui.button(label).clicked() {
                            menu_action = Some((i, WindowMenuAction::OpenRootOverlay));
                            ui.close_menu();
                        }
                    }
                    if has_root {
                        if ui.button("Clear root").clicked() {
                            menu_action = Some((i, WindowMenuAction::ClearRoot));
                            ui.close_menu();
                        }
                    }
                    if num_ctxs > 1 {
                        ui.separator();
                        if ui.button("Delete").clicked() { menu_action = Some((i, WindowMenuAction::Delete)); ui.close_menu(); }
                    }
                });

                match action {
                    SidebarAction::Delete => { delete_context = Some(i); }
                    SidebarAction::Rename => { menu_action = Some((i, WindowMenuAction::Rename)); }
                    SidebarAction::Activate => {
                        log::debug!("sidebar: activate ctx={i} active={}", self.router.active_idx());
                        let ctx = self.router.get(i);
                        let active_ctx = self.router.active();
                        if ctx.parent_id == Some(active_ctx.context_id) {
                            // Direct child → zoom in
                            let focused_tile = self.windows[self.active_window].focused_pane;
                            let current_ctx_id = active_ctx.context_id;
                            let current_win_id = self.windows[self.active_window].window_id;
                            self.router.push_depth(current_ctx_id, current_win_id, focused_tile);
                            self.switch_workspace(i);
                        } else if active_ctx.parent_id.is_some() && ctx.parent_id == active_ctx.parent_id {
                            // Sibling at same level → just switch
                            self.switch_workspace(i);
                        } else if ctx.parent_id.is_none() && self.router.current_depth() > 0 {
                            // Top-level context clicked while zoomed in → pop all depth stack then switch
                            while self.router.current_depth() > 0 {
                                self.router.pop_depth();
                            }
                            self.switch_workspace(i);
                        } else {
                            // Regular switch
                            self.switch_workspace(i);
                        }
                        // Auto-expand the newly active context
                        let new_active_id = self.router.active().context_id;
                        self.sidebar_expanded_contexts.insert(new_active_id);
                    }
                    _ => {}
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
                    self.renaming_window = None;
                    self.router.reorder_tracking_active(src, effective_dst);
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
                    self.rename_buffer = self.router.get(i).name.clone();
                }
                WindowMenuAction::EditDescription => {
                    log::info!("sidebar: edit context description ctx_idx={i}");
                    self.editing_description = Some(i);
                    self.description_buffer = self.router.get(i).description.clone().unwrap_or_default();
                    self.description_focus_requested = false;
                    self.push_focus_layer(crate::app::FocusLayer::ContextDescription);
                }
                WindowMenuAction::MoveToTop => {
                    self.renaming_window = None;
                    self.router.move_to_front_tracking_active(i);
                }
                WindowMenuAction::MoveUp => {
                    self.renaming_window = None;
                    self.router.swap_tracking_active(i, i - 1);
                }
                WindowMenuAction::MoveDown => {
                    self.renaming_window = None;
                    self.router.swap_tracking_active(i, i + 1);
                }
                WindowMenuAction::MoveToBottom => {
                    self.renaming_window = None;
                    self.router.move_to_back_tracking_active(i);
                }
                WindowMenuAction::SetRoot(path) => {
                    log::info!("sidebar: set context root ctx_idx={i} root={}", path.display());
                    self.router.get_mut(i).root = Some(path);
                    self.save_workspace();
                }
                WindowMenuAction::ClearRoot => {
                    log::info!("sidebar: clear context root ctx_idx={i}");
                    self.router.get_mut(i).root = None;
                    self.save_workspace();
                }
                WindowMenuAction::OpenRootOverlay => {
                    let existing = self.router.get(i).root.as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default();
                    log::info!("TextInputOverlay: opened target=ContextRoot({i}) via sidebar");
                    self.text_overlay_browse_rx = None;
                    self.text_overlay = Some((
                        crate::app::TextInputOverlay {
                            label: "Set context root".to_string(),
                            hint: "/path/to/project or ~/...".to_string(),
                            buffer: existing,
                            focus_requested: false,
                        },
                        crate::app::OverlayTarget::ContextRoot(i),
                    ));
                }
                WindowMenuAction::Delete => {
                    self.delete_context(i);
                }
                WindowMenuAction::NewSubContext => {
                    let parent_name = self.router.get(i).name.clone();
                    let cwd = self.windows.get(self.active_window)
                        .and_then(|w| w.focused_pane.map(|tile| (w, tile)))
                        .and_then(|(w, tile)| w.get_focused_pane_cwd(tile))
                        .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/")));
                    if let Err(e) = self.new_child_context(&parent_name, cwd) {
                        log::error!("sidebar: NewSubContext failed: {e}");
                    } else {
                        log::info!("sidebar: new sub-context created under '{parent_name}'");
                        self.save_workspace();
                    }
                }
            }
        } else if let Some(i) = delete_context {
            self.delete_context(i);
        }

        if add_clicked {
            self.new_context();
        }
    }
}
