use crate::context::WindowMenuAction;
use crate::sidebar_row::{with_alpha, SidebarAction, SidebarRow, SUBTITLE_LINE_HEIGHT, ROW_HEIGHT};
use egui::{Align, CornerRadius, Layout, Rect, RichText, Stroke, Vec2};
use egui_tiles::Tile;

use crate::app::PlexiApp;

const PANE_DOT_RADIUS: f32 = 2.5;
const PANE_DOT_SPACING: f32 = 8.0;
const PANE_DOT_MAX: usize = 6;
const SUBTITLE_MAX_CHARS: usize = 24;

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

                for (i, &ctx_id) in path_ids.iter().enumerate() {
                    let name = self.router.iter()
                        .find(|c| c.context_id == ctx_id)
                        .map(|c| c.name.as_str())
                        .unwrap_or("?");
                    let _ = ui.small_button(name);
                    if i < path_ids.len() - 1 {
                        ui.label(egui::RichText::new("\u{203A}").color(self.colors.text_dim));
                    }
                }
            });
            ui.separator();
        }

        let num_contexts = self.router.len();
        let mut clicked_workspace: Option<usize> = None;
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

            // --- Renaming: special-cased before SidebarRow path ---
            if is_renaming {
                let origin = ui.cursor().min;
                let row_rect = Rect::from_min_size(origin, Vec2::new(sidebar_width, ROW_HEIGHT));
                row_rects.push(row_rect);
                ui.allocate_space(Vec2::new(sidebar_width, ROW_HEIGHT));

                let fill = if is_active { self.colors.bg_active } else { self.colors.bg_sidebar_hover };
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
                        let te = ui.scope(|ui| {
                            ui.visuals_mut().text_cursor.stroke.width = 1.5;
                            ui.visuals_mut().text_cursor.stroke.color = self.colors.accent;
                            ui.add(
                                egui::TextEdit::singleline(&mut self.rename_buffer)
                                    .id(te_id)
                                    .desired_width(sidebar_width - 56.0)
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
                    },
                );
                continue;
            }

            // --- Normal row via SidebarRow ---

            // Derive subtitle: pty_title or CWD basename for the focused pane of this context.
            let subtitle: Option<String> = {
                // Find the window belonging to this context that has a focused pane.
                let ctx_window = self.windows.iter()
                    .find(|w| w.context_id == ctx_id && w.focused_pane.is_some());
                ctx_window.and_then(|w| {
                    let tile_id = w.focused_pane?;
                    let pane_id = match w.tree.tiles.get(tile_id)? {
                        Tile::Pane(pid) => *pid,
                        _ => return None,
                    };
                    let pane = w.panes.get(&pane_id)?;
                    if let Some(t) = pane.as_terminal() {
                        // Prefer name (user-set or OSC), then pty_title, then CWD basename.
                        t.name.clone()
                            .or_else(|| t.pty_title.clone())
                            .or_else(|| {
                                crate::shell::get_pid_cwd(t.backend.child_pid())
                                    .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                            })
                    } else if let Some(app) = pane.as_app() {
                        Some(app.name.clone())
                    } else {
                        None
                    }
                }).map(|s| {
                    if s.chars().count() > SUBTITLE_MAX_CHARS {
                        let mut truncated: String = s.chars().take(SUBTITLE_MAX_CHARS - 1).collect();
                        truncated.push('\u{2026}');
                        truncated
                    } else {
                        s
                    }
                })
            };

            // Subtitle line shown when context has at least one pane.
            let has_subtitle = pane_count > 0 && subtitle.is_some();
            let subtitle_height = if has_subtitle { SUBTITLE_LINE_HEIGHT } else { 0.0 };

            let row = SidebarRow::new(ui, sidebar_width, num_contexts > 1 && !any_dragging, subtitle_height)
                .active(is_active)
                .dragging(is_dragging, any_dragging);

            // Snapshot the layout for drop-indicator tracking (must be before draw()).
            let row_full = row.layout.full;

            // Prepare content data (borrows resolved before the closure).
            let text_color = with_alpha(
                if is_active { self.colors.text_primary } else { self.colors.text_dim },
                if is_dragging { 0.4 } else { 1.0 },
            );
            let ctx_name = self.router.get(i).name.clone();
            let ctx_depth = self.router.get(i).depth;
            let badge_count = if is_active {
                self.visible_notification_count()
            } else {
                self.context_notification_count(i)
            };
            let dim_color = self.colors.text_dim;
            let accent_color = self.colors.accent;

            // Capture subtitle rendering data before the closures.
            let show_dots = pane_count > 1;
            let subtitle_text = subtitle.clone();
            let indent = 20.0 + ctx_depth as f32 * 12.0;

            let (action, response) = row.draw(
                ui,
                egui::Id::new(("ctx", i)),
                &self.colors,
                // Line 1: context name + badge
                |row_ui, _hovered| {
                    row_ui.add_space(20.0 + ctx_depth as f32 * 12.0);
                    if i < 9 {
                        row_ui.label(
                            RichText::new(format!("{}", i + 1)).size(11.0).color(dim_color),
                        );
                    }
                    row_ui.add(
                        egui::Label::new(RichText::new(&ctx_name).size(12.0).color(text_color))
                            .selectable(false),
                    );
                    if badge_count > 0 {
                        row_ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.add_space(8.0);
                            let badge_text = if badge_count > 9 { "9+".to_string() } else { badge_count.to_string() };
                            ui.label(RichText::new(badge_text).size(10.0).color(accent_color));
                        });
                    }
                },
                // Line 2: pane dots (if 2+ panes) + subtitle text
                |painter, subtitle_rect, row_alpha| {
                    let cy = subtitle_rect.center().y;
                    let mut text_x = subtitle_rect.min.x + indent;

                    // Pane dots (only when 2+ panes)
                    if show_dots {
                        let capped = pane_count.min(PANE_DOT_MAX);
                        for dot_i in 0..capped {
                            let cx = subtitle_rect.min.x + indent + (dot_i as f32) * PANE_DOT_SPACING + PANE_DOT_RADIUS;
                            let color = if focused_dot_idx == Some(dot_i) {
                                with_alpha(accent_color, row_alpha)
                            } else {
                                with_alpha(dim_color, 0.35 * row_alpha)
                            };
                            painter.circle_filled(egui::pos2(cx, cy), PANE_DOT_RADIUS, color);
                        }
                        if pane_count > PANE_DOT_MAX {
                            let overflow_x = subtitle_rect.min.x + indent + (capped as f32) * PANE_DOT_SPACING + PANE_DOT_RADIUS * 0.5;
                            painter.text(
                                egui::pos2(overflow_x, cy),
                                egui::Align2::LEFT_CENTER,
                                format!("+{}", pane_count - PANE_DOT_MAX),
                                egui::FontId::proportional(8.0),
                                with_alpha(dim_color, 0.5 * row_alpha),
                            );
                        }
                        // Advance text_x past the dots
                        let dots_width = (capped as f32) * PANE_DOT_SPACING + PANE_DOT_RADIUS;
                        if pane_count > PANE_DOT_MAX {
                            // Extra space for overflow text
                            text_x += dots_width + 16.0;
                        } else {
                            text_x += dots_width + 6.0;
                        }
                    }

                    // Subtitle text
                    if let Some(ref text) = subtitle_text {
                        painter.text(
                            egui::pos2(text_x, cy),
                            egui::Align2::LEFT_CENTER,
                            text,
                            egui::FontId::proportional(9.0),
                            with_alpha(dim_color, 0.6 * row_alpha),
                        );
                    }
                },
            );

            row_rects.push(row_full);

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
                        clicked_workspace = Some(i);
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
