use crate::host::context::WindowMenuAction;
use crate::ui::sidebar_row::{ContextItem, PaneDots, SidebarAction};
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

        let num_contexts = self.router.len();
        let mut clicked_workspace: Option<usize> = None;
        let mut delete_context: Option<usize> = None;
        let mut menu_action: Option<(usize, WindowMenuAction)> = None;
        let mut row_rects: Vec<Rect> = Vec::with_capacity(num_contexts);
        let mut drag_released = false;
        let mut unpark_context: Option<usize> = None;

        let focused_cwd = self
            .windows
            .get(self.active_window)
            .and_then(|w| w.focused_pane.map(|tile| (w, tile)))
            .and_then(|(w, tile)| w.get_focused_pane_cwd(tile));

        // Build display order: top-level contexts first, each followed by children.
        // Separate into active (unparked) and parked lists.
        let mut display_order: Vec<usize> = Vec::with_capacity(num_contexts);
        for i in 0..num_contexts {
            if self.router.get(i).parent_id.is_none() {
                display_order.push(i);
                let ctx_id = self.router.get(i).context_id;
                for j in 0..num_contexts {
                    if self.router.get(j).parent_id == Some(ctx_id) {
                        display_order.push(j);
                    }
                }
            }
        }
        // Catch orphans whose parent was deleted.
        for i in 0..num_contexts {
            if !display_order.contains(&i) {
                display_order.push(i);
            }
        }

        // Partition: active contexts render in the main list, parked ones below.
        let active_order: Vec<usize> = display_order
            .iter()
            .copied()
            .filter(|&i| !self.router.get(i).parked)
            .collect();
        let parked_order: Vec<usize> = display_order
            .iter()
            .copied()
            .filter(|&i| self.router.get(i).parked)
            .collect();

        // ── Active contexts ─────────────────────────────────────────────
        for (display_idx, &i) in active_order.iter().enumerate() {
            let is_active = i == self.router.active_idx();
            let is_renaming = self.renaming_window == Some(i);
            let is_dragging = self.drag_context == Some(i);
            let any_dragging = self.drag_context.is_some();

            // Pane count + focused-pane index for this context
            let ctx_id = self.router.get(i).context_id;
            let mut ctx_windows: Vec<usize> = self
                .windows
                .iter()
                .enumerate()
                .filter(|(_, w)| w.context_id == ctx_id)
                .map(|(idx, _)| idx)
                .collect();
            ctx_windows.sort_by_key(|&idx| {
                let w = &self.windows[idx];
                (w.grid_y, w.grid_x)
            });
            let mut pane_ids: Vec<u64> = Vec::new();
            for &win_idx in &ctx_windows {
                let w = &self.windows[win_idx];
                if let Some(root) = w.tree.root() {
                    pane_ids.extend(crate::spatial::tiling::collect_pane_ids_spatial(
                        &w.tree.tiles,
                        root,
                    ));
                }
            }
            let pane_count = pane_ids.len();

            let focused_pane_idx: Option<usize> = if is_active {
                self.windows
                    .get(self.active_window)
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

            // Build pane dots for this context row — track which are hidden.
            let pane_dots = if pane_count > 0 {
                let mut hidden_set = std::collections::HashSet::new();
                for (dot_idx, &pid) in pane_ids.iter().enumerate() {
                    let is_hidden = self
                        .windows
                        .iter()
                        .filter(|w| w.context_id == ctx_id)
                        .find_map(|w| w.panes.get(&pid))
                        .map_or(false, |p| p.is_hidden());
                    if is_hidden {
                        hidden_set.insert(dot_idx);
                    }
                }
                Some(PaneDots {
                    count: pane_count,
                    focused_idx: focused_pane_idx,
                    hidden_set,
                })
            } else {
                None
            };

            // --- Renaming: special-cased before ContextItem path ---
            if is_renaming {
                let fill = if is_active {
                    self.colors.bg_active
                } else {
                    self.colors.bg_sidebar_hover
                };
                let bg_idx = ui.painter().add(egui::Shape::Noop);
                let te_id = egui::Id::new(("rename_ctx", i));
                let accent = self.colors.accent;
                let sidebar_w = sidebar_width;
                let scope = ui.scope(|ui| {
                    ui.set_width(ui.available_width());
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.add_space(20.0);
                        let te = ui
                            .scope(|ui| {
                                ui.visuals_mut().text_cursor.stroke.width = 1.5;
                                ui.visuals_mut().text_cursor.stroke.color = accent;
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.rename_buffer)
                                        .id(te_id)
                                        .desired_width(sidebar_w - 56.0)
                                        .font(egui::TextStyle::Body),
                                )
                            })
                            .inner;
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
                    ui.add_space(4.0);
                });
                let row_rect = scope.response.rect;
                row_rects.push(row_rect);
                ui.painter().set(
                    bg_idx,
                    egui::Shape::rect_filled(row_rect, CornerRadius::ZERO, fill),
                );
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
            let badge_count = if is_active {
                self.visible_notification_count()
            } else {
                self.context_notification_count(i)
            };
            let subtitle = self
                .router
                .get(i)
                .root
                .as_ref()
                .map(|p| p.display().to_string());

            let indent = self.router.get(i).depth;
            let (action, response) = ContextItem {
                is_active,
                is_dragging,
                any_dragging,
                action_enabled: num_contexts > 1 && !any_dragging,
                ctx_name,
                ctx_index: Some(display_idx),
                badge_count,
                subtitle,
                pane_dots,
                indent,
            }
            .draw(ui, egui::Id::new(("ctx", i)), &self.colors);

            row_rects.push(response.rect);

            match action {
                SidebarAction::DragStart => {
                    self.drag_context = Some(i);
                }
                SidebarAction::DragEnd => {
                    drag_released = true;
                }
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
                    if i < num_ctxs - 1 {
                        if ui.button("Move Down").clicked() {
                            menu_action = Some((i, WindowMenuAction::MoveDown));
                            ui.close_menu();
                        }
                        if ui.button("Move to Bottom").clicked() {
                            menu_action = Some((i, WindowMenuAction::MoveToBottom));
                            ui.close_menu();
                        }
                    }
                    ui.separator();
                    if let Some(cwd) = &cwd_for_menu {
                        if ui.button("Set root to current path").clicked() {
                            menu_action = Some((i, WindowMenuAction::SetRoot(cwd.clone())));
                            ui.close_menu();
                        }
                    }
                    {
                        let label = if has_root {
                            "Edit root\u{2026}"
                        } else {
                            "Set root\u{2026}"
                        };
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
                        if ui.button("Delete").clicked() {
                            menu_action = Some((i, WindowMenuAction::Delete));
                            ui.close_menu();
                        }
                    }
                });

                match action {
                    SidebarAction::Delete => {
                        delete_context = Some(i);
                    }
                    SidebarAction::Rename => {
                        menu_action = Some((i, WindowMenuAction::Rename));
                    }
                    SidebarAction::Activate => {
                        log::debug!(
                            "sidebar: activate ctx={i} active={}",
                            self.router.active_idx()
                        );
                        clicked_workspace = Some(i);
                    }
                    _ => {}
                }
            }
        }

        // ── Parked section ──────────────────────────────────────────────
        let parked_count = parked_order.len();
        if parked_count > 0 {
            ui.add_space(8.0);

            // Divider row: "Parked (N)" -- clickable to expand/collapse
            let divider_id = egui::Id::new("parked_divider");
            let expanded = self.parked_section_expanded;
            let chevron = if expanded { "\u{25BE}" } else { "\u{25B8}" }; // ▾ or ▸
            let divider_text = format!("{chevron} Parked ({parked_count})");

            let divider_response = ui
                .horizontal(|ui| {
                    ui.add_space(16.0);
                    ui.add(
                        egui::Label::new(
                            RichText::new(divider_text)
                                .size(10.0)
                                .color(self.colors.text_dim),
                        )
                        .selectable(false)
                        .sense(egui::Sense::click()),
                    )
                })
                .inner;

            let response = ui.interact(divider_response.rect, divider_id, egui::Sense::click());
            if response.clicked() || divider_response.clicked() {
                self.parked_section_expanded = !self.parked_section_expanded;
            }
            if response.hovered() || divider_response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }

            // Expanded: show parked context names
            if self.parked_section_expanded {
                for &i in &parked_order {
                    let ctx_name = self.router.get(i).name.clone();
                    let text_color = self.colors.text_dim;

                    let row_response = ui
                        .horizontal(|ui| {
                            ui.add_space(24.0);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&ctx_name).size(11.0).color(text_color),
                                )
                                .selectable(false)
                                .sense(egui::Sense::click()),
                            )
                        })
                        .inner;

                    let row_id = egui::Id::new(("parked_ctx", i));
                    let interact = ui.interact(row_response.rect, row_id, egui::Sense::click());
                    if interact.clicked() || row_response.clicked() {
                        unpark_context = Some(i);
                    }
                    if interact.hovered() || row_response.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
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
                    row_rects.first().map_or(0.0, |r| r.min.y)
                } else if dst >= row_rects.len() {
                    row_rects.last().map_or(0.0, |r| r.max.y)
                } else {
                    row_rects[dst - 1].max.y
                };
                let x0 = row_rects.first().map_or(0.0, |r| r.min.x);
                ui.painter().line_segment(
                    [
                        egui::pos2(x0, line_y),
                        egui::pos2(x0 + sidebar_width, line_y),
                    ],
                    Stroke::new(2.0, self.colors.accent),
                );
            }
        }

        if let Some(i) = unpark_context {
            self.unpark_context(i);
        } else if let Some((i, action)) = menu_action {
            match action {
                WindowMenuAction::Rename => {
                    self.renaming_window = Some(i);
                    self.rename_buffer = self.router.get(i).name.clone();
                }
                WindowMenuAction::EditDescription => {
                    log::info!("sidebar: edit context description ctx_idx={i}");
                    self.editing_description = Some(i);
                    self.description_buffer =
                        self.router.get(i).description.clone().unwrap_or_default();
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
                    log::info!(
                        "sidebar: set context root ctx_idx={i} root={}",
                        path.display()
                    );
                    self.router.get_mut(i).root = Some(path);
                    self.save_workspace();
                }
                WindowMenuAction::ClearRoot => {
                    log::info!("sidebar: clear context root ctx_idx={i}");
                    self.router.get_mut(i).root = None;
                    self.save_workspace();
                }
                WindowMenuAction::OpenRootOverlay => {
                    let existing = self
                        .router
                        .get(i)
                        .root
                        .as_ref()
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
            }
        } else if let Some(i) = delete_context {
            self.delete_context(i);
        } else if let Some(i) = clicked_workspace {
            let target_parent = self.router.get(i).parent_id;
            let current_ctx_id = self.router.active().context_id;
            if target_parent == Some(current_ctx_id) {
                let current_win_id = self.windows[self.active_window].window_id;
                let focused_tile = self.windows[self.active_window].focused_pane;
                self.router
                    .push_depth(current_ctx_id, current_win_id, focused_tile);
            }
            self.switch_workspace(i);
        }

        if add_clicked {
            self.new_context();
        }
    }
}
