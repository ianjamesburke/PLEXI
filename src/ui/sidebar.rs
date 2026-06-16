use crate::host::context::WindowMenuAction;
use crate::ui::button;
use crate::ui::list::ListDropdownHeader;
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

const NEST_DROP_X_OFFSET: f32 = 54.0;
const NEST_DROP_Y_FRACTION: f32 = 0.25;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SidebarDropMode {
    TopLevelSlot,
    Nest { target_idx: usize },
}

/// Where a dragged context will land when released: which section (active vs.
/// parked), the slot within that section's display list, and whether the drop
/// means top-level placement or nesting under a specific row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct SidebarDrop {
    parked: bool,
    slot: usize,
    mode: SidebarDropMode,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SidebarRowInfo {
    pub(crate) router_idx: usize,
    pub(crate) top_level_number: Option<usize>,
}

/// Resolve a dragged context against the current pointer position. The Parked
/// header acts as the boundary: anything at or below it targets the parked
/// section, anything above targets the active section. Within a section the
/// slot is the insertion index between rows. Returns `None` when the drop would
/// leave the context exactly where it already is (a no-op).
///
/// `src_parked`/`src_slot` describe the dragged context's current position so
/// same-section no-op drops (onto its own row edges) can be filtered out.
fn resolve_drag_drop(
    src_parked: bool,
    src_slot: usize,
    parked_header: Option<Rect>,
    pointer: Option<egui::Pos2>,
    active_rects: &[(Rect, usize)],
    parked_rects: &[(Rect, usize)],
) -> Option<SidebarDrop> {
    let pos = pointer?;
    let into_parked = parked_header.map_or(false, |h| pos.y >= h.top());
    let rects = if into_parked {
        parked_rects
    } else {
        active_rects
    };
    let row_rects: Vec<Rect> = rects.iter().map(|(rect, _)| *rect).collect();
    for (rect, target_idx) in rects {
        let middle_top = rect.top() + rect.height() * NEST_DROP_Y_FRACTION;
        let middle_bottom = rect.bottom() - rect.height() * NEST_DROP_Y_FRACTION;
        if pos.y >= middle_top
            && pos.y <= middle_bottom
            && pos.x >= rect.left() + NEST_DROP_X_OFFSET
        {
            return Some(SidebarDrop {
                parked: into_parked,
                slot: drop_slot_from_rects(&row_rects, pos.y),
                mode: SidebarDropMode::Nest {
                    target_idx: *target_idx,
                },
            });
        }
    }
    let slot = drop_slot_from_rects(&row_rects, pos.y);
    // Same-section drop onto the source's own edges leaves it in place.
    if into_parked == src_parked && (slot == src_slot || slot == src_slot + 1) {
        return None;
    }
    Some(SidebarDrop {
        parked: into_parked,
        slot,
        mode: SidebarDropMode::TopLevelSlot,
    })
}

/// Y coordinate of the reorder indicator line for a drop at `slot` within
/// `rects` (the section's row rectangles, in display order).
fn reorder_line_y(rects: &[Rect], slot: usize) -> f32 {
    if slot == 0 {
        rects.first().map_or(0.0, |r| r.min.y)
    } else if slot >= rects.len() {
        rects.last().map_or(0.0, |r| r.max.y)
    } else {
        rects[slot - 1].max.y
    }
}

impl PlexiApp {
    pub(crate) fn sidebar_display_order(&self) -> Vec<usize> {
        fn append_children(app: &PlexiApp, parent_id: Option<u64>, out: &mut Vec<usize>) {
            for idx in 0..app.router.len() {
                if app.router.get(idx).parent_id == parent_id {
                    out.push(idx);
                    let ctx_id = app.router.get(idx).context_id;
                    append_children(app, Some(ctx_id), out);
                }
            }
        }

        let mut order = Vec::with_capacity(self.router.len());
        append_children(self, None, &mut order);
        for idx in 0..self.router.len() {
            if !order.contains(&idx) {
                order.push(idx);
            }
        }
        order
    }

    pub(crate) fn sidebar_top_level_active_order(&self) -> Vec<usize> {
        self.sidebar_display_order()
            .into_iter()
            .filter(|&idx| self.router.get(idx).parent_id.is_none() && !self.router.get(idx).parked)
            .collect()
    }

    fn sidebar_section_order(&self, parked: bool) -> Vec<usize> {
        self.sidebar_display_order()
            .into_iter()
            .filter(|&idx| self.router.get(idx).parked == parked)
            .collect()
    }

    fn sidebar_has_children(&self, router_idx: usize) -> bool {
        let ctx_id = self.router.get(router_idx).context_id;
        self.router.iter().any(|ctx| ctx.parent_id == Some(ctx_id))
    }

    fn sidebar_has_collapsed_ancestor_in_section(&self, router_idx: usize, parked: bool) -> bool {
        let mut parent_id = self.router.get(router_idx).parent_id;
        while let Some(pid) = parent_id {
            let Some(parent_idx) = self.router.position(|ctx| ctx.context_id == pid) else {
                break;
            };
            let parent = self.router.get(parent_idx);
            if parent.parked == parked && self.collapsed_contexts.contains(&pid) {
                return true;
            }
            parent_id = parent.parent_id;
        }
        false
    }

    pub(crate) fn sidebar_visible_rows(&self, parked: bool) -> Vec<SidebarRowInfo> {
        let mut top_level_number = 0usize;
        let mut rows = Vec::new();
        for router_idx in self.sidebar_section_order(parked) {
            let ctx = self.router.get(router_idx);
            let number = if !parked && ctx.parent_id.is_none() {
                let n = top_level_number;
                top_level_number += 1;
                Some(n)
            } else {
                None
            };
            if self.sidebar_has_collapsed_ancestor_in_section(router_idx, parked) {
                continue;
            }
            rows.push(SidebarRowInfo {
                router_idx,
                top_level_number: number,
            });
        }
        rows
    }

    fn active_context_descends_from(&self, ancestor_context_id: u64) -> bool {
        let mut current = Some(self.router.active().context_id);
        while let Some(ctx_id) = current {
            if ctx_id == ancestor_context_id {
                return true;
            }
            current = self
                .router
                .iter()
                .find(|ctx| ctx.context_id == ctx_id)
                .and_then(|ctx| ctx.parent_id);
        }
        false
    }

    fn toggle_context_children_collapsed(&mut self, router_idx: usize) {
        if !self.sidebar_has_children(router_idx) {
            return;
        }
        let context_id = self.router.get(router_idx).context_id;
        if self.collapsed_contexts.remove(&context_id) {
            log::info!("sidebar: expanded child rows for context id={context_id}");
            return;
        }
        if self.router.active().context_id != context_id
            && self.active_context_descends_from(context_id)
        {
            self.switch_workspace(router_idx);
        }
        self.collapsed_contexts.insert(context_id);
        log::info!("sidebar: collapsed child rows for context id={context_id}");
    }

    fn sidebar_previous_top_level_context(&self, router_idx: usize) -> Option<usize> {
        let parked = self.router.get(router_idx).parked;
        let display_order = self.sidebar_display_order();
        let pos = display_order.iter().position(|&idx| idx == router_idx)?;
        display_order[..pos].iter().rev().copied().find(|&idx| {
            let ctx = self.router.get(idx);
            ctx.parent_id.is_none() && ctx.parked == parked
        })
    }

    fn sidebar_top_level_insert_index(
        &self,
        parked: bool,
        visible_slot: usize,
        moving_id: u64,
    ) -> usize {
        let visible_rows = self.sidebar_visible_rows(parked);
        let mut top_level_slot = 0usize;
        for row in visible_rows.iter().take(visible_slot) {
            let ctx = self.router.get(row.router_idx);
            if ctx.parent_id.is_none() && ctx.context_id != moving_id {
                top_level_slot += 1;
            }
        }

        let top_level_indices: Vec<usize> = self
            .sidebar_display_order()
            .into_iter()
            .filter(|&idx| {
                let ctx = self.router.get(idx);
                ctx.parked == parked && ctx.parent_id.is_none() && ctx.context_id != moving_id
            })
            .collect();

        if let Some(&target_idx) = top_level_indices.get(top_level_slot) {
            return target_idx;
        }
        top_level_indices
            .last()
            .map_or(self.router.len(), |&idx| self.router.subtree_end(idx))
    }

    fn switch_after_drag_park(&mut self, moved_idx: usize) {
        let len = self.router.len();
        let next = (1..len)
            .map(|offset| (moved_idx + offset) % len)
            .find(|&idx| !self.router.get(idx).parked);
        if let Some(idx) = next {
            self.switch_workspace(idx);
        }
    }

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
                if button::icon_button(ui, "+", "New context", &self.colors).clicked() {
                    add_clicked = true;
                }
            });
        });
        ui.add_space(4.0);

        let num_contexts = self.router.len();
        let mut clicked_workspace: Option<usize> = None;
        let mut delete_context: Option<usize> = None;
        let mut menu_action: Option<(usize, WindowMenuAction)> = None;
        let mut active_rects: Vec<(Rect, usize)> = Vec::with_capacity(num_contexts);
        let mut parked_rects: Vec<(Rect, usize)> = Vec::with_capacity(num_contexts);
        let mut drag_released = false;
        let mut unpark_context: Option<usize> = None;

        let focused_cwd = self
            .windows
            .get(self.active_window)
            .and_then(|w| w.focused_pane.map(|tile| (w, tile)))
            .and_then(|(w, tile)| w.get_focused_pane_cwd(tile));

        let active_rows = self.sidebar_visible_rows(false);
        let parked_rows = self.sidebar_visible_rows(true);

        // ── Active contexts ─────────────────────────────────────────────
        for row in active_rows.iter().copied() {
            let i = row.router_idx;
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

            // Build pane dots for this context row — track hidden + agent state.
            let pane_dots = if pane_count > 0 {
                let mut hidden_set = std::collections::HashSet::new();
                let mut activities = Vec::with_capacity(pane_count);
                for (dot_idx, &pid) in pane_ids.iter().enumerate() {
                    let pane_opt = self
                        .windows
                        .iter()
                        .filter(|w| w.context_id == ctx_id)
                        .find_map(|w| w.panes.get(&pid));
                    let is_hidden = pane_opt.map_or(false, |p| p.is_hidden());
                    if is_hidden {
                        hidden_set.insert(dot_idx);
                    }
                    activities.push(pane_opt.and_then(|p| p.effective_activity()).cloned());
                }
                Some(PaneDots {
                    count: pane_count,
                    focused_idx: focused_pane_idx,
                    hidden_set,
                    activities,
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
                let sidebar_w = sidebar_width;
                let scope = ui.scope(|ui| {
                    ui.set_width(ui.available_width());
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.add_space(20.0);
                        let te = ui
                            .scope(|ui| {
                                ui.set_max_width(sidebar_w - 56.0);
                                crate::ui::text_field::TextField::singleline(te_id, "").show(
                                    ui,
                                    &mut self.rename_buffer,
                                    &self.colors,
                                )
                            })
                            .inner;
                        if te.lost_focus() {
                            if ui.input(|inp| inp.key_pressed(egui::Key::Escape)) {
                                self.renaming_window = None;
                            } else {
                                let new_name = self.rename_buffer.trim().to_string();
                                if !new_name.is_empty() {
                                    self.router.get_mut(i).name = new_name.clone();
                                    crate::host::event_log::emit(
                                        crate::host::event_log::HostEvent::ContextRenamed {
                                            context_id: self.router.get(i).context_id,
                                            name: new_name,
                                            timestamp: crate::host::event_log::now_timestamp(),
                                        },
                                    );
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
                active_rects.push((row_rect, i));
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
                ctx_index: row.top_level_number,
                badge_count,
                subtitle,
                pane_dots,
                indent,
                child_toggle: self.sidebar_has_children(i).then_some(
                    !self
                        .collapsed_contexts
                        .contains(&self.router.get(i).context_id),
                ),
                draggable: true,
            }
            .draw(ui, egui::Id::new(("ctx", i)), &self.colors);

            active_rects.push((response.rect, i));

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
                let prev_top_level = self.sidebar_previous_top_level_context(i);
                let parent_id = self.router.get(i).parent_id;
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
                    if let Some(target_idx) = prev_top_level {
                        let target_id = self.router.get(target_idx).context_id;
                        if parent_id != Some(target_id) {
                            let label =
                                format!("Make subcontext of {}", self.router.get(target_idx).name);
                            if ui.button(label).clicked() {
                                menu_action =
                                    Some((i, WindowMenuAction::MoveIntoContext(target_id)));
                                ui.close_menu();
                            }
                        }
                    }
                    if parent_id.is_some() {
                        if ui.button("Promote out one level").clicked() {
                            menu_action = Some((i, WindowMenuAction::PromoteContext));
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
                        if ui.button("Park").clicked() {
                            menu_action = Some((i, WindowMenuAction::Park));
                            ui.close_menu();
                        }
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
                    SidebarAction::ToggleChildren => {
                        self.toggle_context_children_collapsed(i);
                        self.save_workspace();
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
        // The header doubles as a drop target: while a context is being
        // dragged, render it even when empty so the drag can release onto it.
        let parked_count = self.sidebar_section_order(true).len();
        let mut parked_header_rect: Option<Rect> = None;
        if parked_count > 0 || self.drag_context.is_some() {
            ui.add_space(8.0);

            let divider_id = egui::Id::new("parked_divider");
            let expanded = self.parked_section_expanded;
            let label = if parked_count > 0 {
                format!("Parked ({parked_count})")
            } else {
                "Parked".to_string()
            };

            let response = ListDropdownHeader::new(&label, expanded)
                .indent(12.0)
                .label_left(30.0)
                .show(ui, divider_id, &self.colors);
            parked_header_rect = Some(response.rect);
            if response.clicked() {
                self.parked_section_expanded = !self.parked_section_expanded;
            }
            ui.add_space(4.0);

            if self.parked_section_expanded {
                for row in parked_rows.iter().copied() {
                    let i = row.router_idx;
                    let ctx = self.router.get(i);
                    let ctx_name = ctx.name.clone();
                    let ctx_id = ctx.context_id;
                    let subtitle = ctx.root.as_ref().map(|p| p.display().to_string());
                    let indent = ctx.depth;

                    // Build pane dots for parked row (same logic as active rows).
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
                    let pane_dots = if pane_count > 0 {
                        let mut hidden_set = std::collections::HashSet::new();
                        let mut activities = Vec::with_capacity(pane_count);
                        for (dot_idx, &pid) in pane_ids.iter().enumerate() {
                            let pane_opt = self
                                .windows
                                .iter()
                                .filter(|w| w.context_id == ctx_id)
                                .find_map(|w| w.panes.get(&pid));
                            let is_hidden = pane_opt.map_or(false, |p| p.is_hidden());
                            if is_hidden {
                                hidden_set.insert(dot_idx);
                            }
                            activities.push(pane_opt.and_then(|p| p.effective_activity()).cloned());
                        }
                        Some(PaneDots {
                            count: pane_count,
                            focused_idx: None,
                            hidden_set,
                            activities,
                        })
                    } else {
                        None
                    };

                    let (action, response) = ContextItem {
                        is_active: false,
                        is_dragging: self.drag_context == Some(i),
                        any_dragging: self.drag_context.is_some(),
                        action_enabled: false,
                        ctx_name,
                        ctx_index: None,
                        badge_count: 0,
                        subtitle,
                        pane_dots,
                        indent,
                        child_toggle: self.sidebar_has_children(i).then_some(
                            !self
                                .collapsed_contexts
                                .contains(&self.router.get(i).context_id),
                        ),
                        draggable: true,
                    }
                    .draw(ui, egui::Id::new(("parked_ctx", i)), &self.colors);

                    parked_rects.push((response.rect, i));

                    response.context_menu(|ui| {
                        if ui.button("Unpark").clicked() {
                            unpark_context = Some(i);
                            ui.close_menu();
                        }
                    });

                    match action {
                        SidebarAction::DragStart => {
                            self.drag_context = Some(i);
                        }
                        SidebarAction::DragEnd => {
                            drag_released = true;
                        }
                        SidebarAction::ToggleChildren => {
                            self.toggle_context_children_collapsed(i);
                            self.save_workspace();
                        }
                        // A plain click must not unpark — parked contexts only
                        // come back via a drag into the active list or the
                        // right-click "Unpark" action. Clicking is inert.
                        _ => {}
                    }
                }
            }
        }

        // Resolve the drag target once and use it for both the drop affordance
        // and the release action. The Parked header is the section boundary:
        // dropping at/below it targets the parked list, above it the active
        // list — in either case at a chosen slot, so a drag can park, unpark,
        // and reorder within either list.
        let pointer_pos = ui.input(|i| i.pointer.hover_pos());
        let src_section = self.drag_context.map(|src| {
            let src_parked = self.router.get(src).parked;
            let src_slot = if src_parked {
                parked_rows
                    .iter()
                    .position(|row| row.router_idx == src)
                    .unwrap_or(0)
            } else {
                active_rows
                    .iter()
                    .position(|row| row.router_idx == src)
                    .unwrap_or(0)
            };
            (src_parked, src_slot)
        });
        let drop_target = src_section.and_then(|(src_parked, src_slot)| {
            resolve_drag_drop(
                src_parked,
                src_slot,
                parked_header_rect,
                pointer_pos,
                &active_rects,
                &parked_rects,
            )
        });

        // Drop affordance: a reorder line within the target section, or — when
        // dropping into an empty Parked list — an outline of the header itself.
        if let Some(drop) = drop_target {
            let target_rects = if drop.parked {
                &parked_rects
            } else {
                &active_rects
            };
            if drop.parked && parked_rects.is_empty() {
                // Border only — a fill would paint over the header label drawn
                // earlier this frame and hide the "Parked" text.
                if let Some(rect) = parked_header_rect {
                    ui.painter().rect_stroke(
                        rect.shrink(2.0),
                        crate::ui::style::RADIUS_SM,
                        Stroke::new(1.5, self.colors.accent),
                        egui::StrokeKind::Inside,
                    );
                }
            } else {
                match drop.mode {
                    SidebarDropMode::TopLevelSlot => {
                        let rects: Vec<Rect> = target_rects.iter().map(|(rect, _)| *rect).collect();
                        let line_y = reorder_line_y(&rects, drop.slot);
                        let x0 = rects.first().map_or(0.0, |r| r.min.x);
                        ui.painter().line_segment(
                            [
                                egui::pos2(x0, line_y),
                                egui::pos2(x0 + sidebar_width, line_y),
                            ],
                            Stroke::new(2.0, self.colors.accent),
                        );
                    }
                    SidebarDropMode::Nest { target_idx } => {
                        if let Some((rect, _)) =
                            target_rects.iter().find(|(_, idx)| *idx == target_idx)
                        {
                            ui.painter().rect_stroke(
                                rect.shrink(2.0),
                                crate::ui::style::RADIUS_SM,
                                Stroke::new(1.5, self.colors.accent),
                                egui::StrokeKind::Inside,
                            );
                        }
                    }
                }
            }
        }

        if drag_released {
            if let (Some(src), Some(drop), Some((src_parked, _src_slot))) =
                (self.drag_context, drop_target, src_section)
            {
                let src_id = self.router.get(src).context_id;
                let was_active = self.router.active_idx() == src;
                self.renaming_window = None;

                let moved = match drop.mode {
                    SidebarDropMode::Nest { target_idx } => {
                        let parent_id = self.router.get(target_idx).context_id;
                        if self.router.can_move_subtree_to_parent(src, Some(parent_id)) {
                            let target_parked = self.router.get(target_idx).parked;
                            self.router.set_subtree_parked(src, target_parked);
                            let moved = self.router.move_subtree_to_parent(src, Some(parent_id));
                            if moved {
                                self.collapsed_contexts.remove(&parent_id);
                                log::info!(
                                    "sidebar: drag-nested context id={src_id} under parent id={parent_id}"
                                );
                            }
                            moved
                        } else {
                            log::info!(
                                "sidebar: rejected invalid drag-nest context id={src_id} target_idx={target_idx}"
                            );
                            false
                        }
                    }
                    SidebarDropMode::TopLevelSlot => {
                        self.router.set_subtree_parked(src, drop.parked);
                        let src_after_park = self
                            .router
                            .position(|ctx| ctx.context_id == src_id)
                            .unwrap_or(src);
                        if self.router.get(src_after_park).parent_id.is_some() {
                            self.router.move_subtree_to_parent(src_after_park, None);
                        }
                        let src_after_promote = self
                            .router
                            .position(|ctx| ctx.context_id == src_id)
                            .unwrap_or(src_after_park);
                        let insert_pos =
                            self.sidebar_top_level_insert_index(drop.parked, drop.slot, src_id);
                        let moved = self
                            .router
                            .move_subtree_to_index(src_after_promote, insert_pos);
                        if moved {
                            log::info!(
                                "sidebar: drag-top-level context id={src_id} parked={} slot={}",
                                drop.parked,
                                drop.slot
                            );
                        }
                        moved
                    }
                };

                if moved {
                    let new_src_idx = self
                        .router
                        .position(|c| c.context_id == src_id)
                        .unwrap_or(self.router.active_idx());
                    let now_parked = self.router.get(new_src_idx).parked;
                    if now_parked && !src_parked && was_active {
                        self.switch_after_drag_park(new_src_idx);
                    } else if !now_parked && src_parked {
                        self.switch_workspace(new_src_idx);
                    }
                    self.save_workspace();
                }
            }
            self.drag_context = None;
        }

        if let Some(i) = unpark_context {
            self.unpark_context(i);
        } else if let Some((i, action)) = menu_action {
            match action {
                WindowMenuAction::Rename => {
                    self.open_context_rename(i);
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
                WindowMenuAction::MoveIntoContext(parent_id) => {
                    let moved_id = self.router.get(i).context_id;
                    if self.router.move_subtree_to_parent(i, Some(parent_id)) {
                        log::info!(
                            "sidebar: reparent context id={moved_id} under parent id={parent_id}"
                        );
                        self.save_workspace();
                    }
                }
                WindowMenuAction::PromoteContext => {
                    let moved_id = self.router.get(i).context_id;
                    if self.router.move_subtree_to_parent(i, None) {
                        log::info!("sidebar: promoted context id={moved_id} out one level");
                        self.save_workspace();
                    }
                }
                WindowMenuAction::SetRoot(path) => {
                    log::info!(
                        "sidebar: set context root ctx_idx={i} root={}",
                        path.display()
                    );
                    let ctx_id = self.router.get(i).context_id;
                    self.set_context_root(path, Some(ctx_id));
                    self.save_workspace();
                }
                WindowMenuAction::ClearRoot => {
                    log::info!("sidebar: clear context root ctx_idx={i}");
                    self.router.get_mut(i).root = None;
                    if i == self.router.active_idx() {
                        self.apply_context_transition_effects();
                    }
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
                WindowMenuAction::Park => {
                    self.park_context(i);
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

#[cfg(test)]
mod tests {
    use super::{reorder_line_y, resolve_drag_drop, SidebarDrop, SidebarDropMode};
    use egui::{pos2, vec2, Rect};

    // Two stacked active rows (y in [0,20] and [20,40]), the Parked header at
    // y in [50,70], and two parked rows below it (y in [70,90] and [90,110]).
    fn active_rects() -> Vec<Rect> {
        vec![
            Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 20.0)),
            Rect::from_min_size(pos2(0.0, 20.0), vec2(100.0, 20.0)),
        ]
    }
    fn parked_rects() -> Vec<Rect> {
        vec![
            Rect::from_min_size(pos2(0.0, 70.0), vec2(100.0, 20.0)),
            Rect::from_min_size(pos2(0.0, 90.0), vec2(100.0, 20.0)),
        ]
    }
    fn with_ids(rects: Vec<Rect>) -> Vec<(Rect, usize)> {
        rects
            .into_iter()
            .enumerate()
            .map(|(idx, rect)| (rect, idx))
            .collect()
    }
    fn header() -> Rect {
        Rect::from_min_size(pos2(0.0, 50.0), vec2(100.0, 20.0))
    }

    fn resolve(
        src_parked: bool,
        src_slot: usize,
        pointer_y: f32,
        parked_rects: &[Rect],
    ) -> Option<SidebarDrop> {
        let parked_rects = with_ids(parked_rects.to_vec());
        resolve_drag_drop(
            src_parked,
            src_slot,
            Some(header()),
            Some(pos2(10.0, pointer_y)),
            &with_ids(active_rects()),
            &parked_rects,
        )
    }

    #[test]
    fn active_dragged_into_empty_parked_parks() {
        // No parked rows yet; dropping below the header targets parked slot 0.
        let target = resolve(false, 0, 60.0, &[]);
        assert_eq!(
            target,
            Some(SidebarDrop {
                parked: true,
                slot: 0,
                mode: SidebarDropMode::TopLevelSlot,
            })
        );
    }

    #[test]
    fn active_dragged_between_parked_rows_picks_slot() {
        // Pointer over the gap between the two parked rows → parked slot 1.
        let target = resolve(false, 0, 88.0, &parked_rects());
        assert_eq!(
            target,
            Some(SidebarDrop {
                parked: true,
                slot: 1,
                mode: SidebarDropMode::TopLevelSlot,
            })
        );
    }

    #[test]
    fn parked_dragged_into_active_unparks_at_slot() {
        // Dragging parked row, pointer over the first active row → active slot 0.
        let target = resolve(true, 0, 5.0, &parked_rects());
        assert_eq!(
            target,
            Some(SidebarDrop {
                parked: false,
                slot: 0,
                mode: SidebarDropMode::TopLevelSlot,
            })
        );
    }

    #[test]
    fn reorder_within_active_list() {
        // Dragging active row 0 down past the last active row → active slot 2.
        let target = resolve(false, 0, 35.0, &parked_rects());
        assert_eq!(
            target,
            Some(SidebarDrop {
                parked: false,
                slot: 2,
                mode: SidebarDropMode::TopLevelSlot,
            })
        );
    }

    #[test]
    fn reorder_within_parked_list() {
        // Dragging parked row 0 (src_slot 0) past parked row 1 → parked slot 2.
        let target = resolve(true, 0, 105.0, &parked_rects());
        assert_eq!(
            target,
            Some(SidebarDrop {
                parked: true,
                slot: 2,
                mode: SidebarDropMode::TopLevelSlot,
            })
        );
    }

    #[test]
    fn dropping_on_row_content_nests_under_that_row() {
        let target = resolve_drag_drop(
            false,
            1,
            Some(header()),
            Some(pos2(80.0, 10.0)),
            &with_ids(active_rects()),
            &with_ids(parked_rects()),
        );
        assert_eq!(
            target,
            Some(SidebarDrop {
                parked: false,
                slot: 0,
                mode: SidebarDropMode::Nest { target_idx: 0 },
            })
        );
    }

    #[test]
    fn dropping_on_own_active_slot_is_a_noop() {
        // Active row 0, pointer still over row 0 (slot 0 or 1) → no move.
        assert_eq!(resolve(false, 0, 5.0, &parked_rects()), None);
        assert_eq!(resolve(false, 0, 25.0, &parked_rects()), None);
    }

    #[test]
    fn dropping_on_own_parked_slot_is_a_noop() {
        // Parked row 0 (src_slot 0), pointer over its own row edges → no move.
        assert_eq!(resolve(true, 0, 75.0, &parked_rects()), None);
        assert_eq!(resolve(true, 0, 95.0, &parked_rects()), None);
    }

    #[test]
    fn no_pointer_is_a_noop() {
        let target = resolve_drag_drop(
            false,
            0,
            Some(header()),
            None,
            &with_ids(active_rects()),
            &with_ids(parked_rects()),
        );
        assert_eq!(target, None);
    }

    #[test]
    fn reorder_line_y_clamps_to_section_bounds() {
        let rects = active_rects();
        assert_eq!(reorder_line_y(&rects, 0), 0.0); // top edge
        assert_eq!(reorder_line_y(&rects, 1), 20.0); // between rows
        assert_eq!(reorder_line_y(&rects, 2), 40.0); // bottom edge
        assert_eq!(reorder_line_y(&rects, 99), 40.0); // past end clamps
    }
}
