use crate::host::context::WindowMenuAction;
use crate::ui::button;
use crate::ui::list::ListDropdownHeader;
use crate::ui::sidebar_row::{PaneDots, SidebarAction, SidebarRow};
use crate::workspace::router::ContextMove;
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

/// Where a dragged context will land when released: which section (active vs.
/// parked) and the slot within that section's display list. A drop fully
/// determines both the context's `parked` flag and its order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct SidebarDrop {
    parked: bool,
    slot: usize,
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
    active_rects: &[Rect],
    parked_rects: &[Rect],
) -> Option<SidebarDrop> {
    let pos = pointer?;
    let into_parked = parked_header.is_some_and(|h| pos.y >= h.top());
    let rects = if into_parked {
        parked_rects
    } else {
        active_rects
    };
    let slot = drop_slot_from_rects(rects, pos.y);
    // Same-section drop onto the source's own edges leaves it in place.
    if into_parked == src_parked && (slot == src_slot || slot == src_slot + 1) {
        return None;
    }
    Some(SidebarDrop {
        parked: into_parked,
        slot,
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
    /// Sidebar pane dots retain the window and pane that context activation
    /// will restore, even while another context is active.
    fn sidebar_pane_dots(&self, ctx_id: u64, is_active_context: bool) -> Option<PaneDots> {
        let return_window_id = if is_active_context {
            self.windows
                .get(self.active_window)
                .map(|window| window.window_id)
        } else {
            self.context_active_window
                .get(&ctx_id)
                .copied()
                .or_else(|| {
                    self.windows
                        .iter()
                        .find(|window| window.context_id == ctx_id)
                        .map(|window| window.window_id)
                })
        };
        let mut ctx_windows: Vec<usize> = self
            .windows
            .iter()
            .enumerate()
            .filter(|(_, window)| window.context_id == ctx_id)
            .map(|(idx, _)| idx)
            .collect();
        ctx_windows.sort_by_key(|&idx| {
            let window = &self.windows[idx];
            (window.grid_y, window.grid_x)
        });

        let mut pane_ids = Vec::new();
        let mut windows = Vec::new();
        for &win_idx in &ctx_windows {
            let window = &self.windows[win_idx];
            let start = pane_ids.len();
            if let Some(root) = window.tree.root() {
                pane_ids.extend(crate::spatial::tiling::collect_pane_ids_spatial(
                    &window.tree.tiles,
                    root,
                ));
            }
            let count = pane_ids.len() - start;
            if count > 0 {
                windows.push(crate::ui::sidebar_row::PaneDotWindow {
                    start,
                    count,
                    is_return_target: return_window_id == Some(window.window_id),
                    is_active: is_active_context && self.active_window == win_idx,
                });
            }
        }

        if pane_ids.is_empty() {
            return None;
        }
        let focused_idx = return_window_id
            .and_then(|window_id| {
                self.windows
                    .iter()
                    .find(|window| window.window_id == window_id && window.context_id == ctx_id)
            })
            .and_then(|window| window.focused_pane.map(|tile| (window, tile)))
            .and_then(|(window, tile)| match window.tree.tiles.get(tile) {
                Some(Tile::Pane(pane_id)) => pane_ids.iter().position(|&id| id == *pane_id),
                _ => None,
            });
        let mut hidden_set = std::collections::HashSet::new();
        let mut activities = Vec::with_capacity(pane_ids.len());
        for (dot_idx, &pane_id) in pane_ids.iter().enumerate() {
            let pane = self
                .windows
                .iter()
                .filter(|window| window.context_id == ctx_id)
                .find_map(|window| window.panes.get(&pane_id));
            if pane.is_some_and(|pane| pane.is_hidden()) {
                hidden_set.insert(dot_idx);
            }
            let activity = pane.and_then(|pane| {
                if let Some(act) = pane.effective_activity() {
                    return Some(act.clone());
                }
                if let Some(t) = pane.as_terminal() {
                    if t.standby_session.is_some() {
                        return Some(crate::protocol::AgentState::NeedsYou);
                    }
                }
                if let Some(portal) = pane.as_portal() {
                    let status = crate::host::context_state::ContextState::compute(
                        portal.target_context_id,
                        self.router.as_slice(),
                        &self.windows,
                    )
                    .status;
                    let effective_status =
                        if matches!(status, crate::host::context_state::ContextStatus::Idle) {
                            portal
                                .context_state
                                .as_ref()
                                .map(|s| s.status.clone())
                                .unwrap_or(status)
                        } else {
                            status
                        };
                    return match effective_status {
                        crate::host::context_state::ContextStatus::NeedsYou => {
                            Some(crate::protocol::AgentState::NeedsYou)
                        }
                        crate::host::context_state::ContextStatus::Blocked => {
                            Some(crate::protocol::AgentState::Blocked)
                        }
                        crate::host::context_state::ContextStatus::Working => {
                            Some(crate::protocol::AgentState::Working)
                        }
                        crate::host::context_state::ContextStatus::Done => {
                            Some(crate::protocol::AgentState::Idle)
                        }
                        crate::host::context_state::ContextStatus::Error
                        | crate::host::context_state::ContextStatus::Idle => None,
                    };
                }
                None
            });
            activities.push(activity);
        }
        Some(PaneDots {
            count: pane_ids.len(),
            focused_idx,
            hidden_set,
            activities,
            windows,
        })
    }

    /// Rolled-up semantic indicator color for a sidebar context row when the
    /// context (or one of its sub-contexts) is in `NeedsYou` (coral),
    /// `Blocked` (amber), or `Error` (danger).
    pub(crate) fn sidebar_context_indicator_color(&self, ctx_id: u64) -> Option<egui::Color32> {
        let state = crate::host::context_state::ContextState::compute(
            ctx_id,
            self.router.as_slice(),
            &self.windows,
        );
        match state.status {
            crate::host::context_state::ContextStatus::NeedsYou => {
                Some(crate::ui::activity::needs_you_color(&self.colors))
            }
            crate::host::context_state::ContextStatus::Blocked => {
                Some(crate::ui::activity::blocked_color(&self.colors))
            }
            crate::host::context_state::ContextStatus::Error => Some(self.colors.danger),
            crate::host::context_state::ContextStatus::Working
            | crate::host::context_state::ContextStatus::Done
            | crate::host::context_state::ContextStatus::Idle => None,
        }
    }
}

const SIDEBAR_PANEL_ID: &str = "sidebar";
const SIDEBAR_DESIRED_WIDTH_ID: &str = "sidebar_desired_width";
const SIDEBAR_WAS_CLAMPED_ID: &str = "sidebar_was_clamped";
static LAST_LOGGED_SIDEBAR_MAX_BITS: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(0);

const _: () = assert!(
    crate::ui::style::SIDEBAR_MIN_W + crate::ui::style::SIDEBAR_CONTENT_MIN_W <= 400.0,
    "sidebar minimum width plus content-area floor must fit inside the 400pt minimum window width"
);

/// Compute the responsive `(default_width, min_width..=max_width)` for the
/// sidebar at `window_width`.
///
/// Expressed as a floor on the central content area
/// (`style::SIDEBAR_CONTENT_MIN_W`, 260 pt) rather than an arbitrary sidebar
/// fraction: `window_width - SIDEBAR_CONTENT_MIN_W` caps the sidebar's upper
/// bound (clamped to `SIDEBAR_MIN_W..=SIDEBAR_MAX_W`), and the default width
/// clamps into that range so even a 400-point window preserves 260 points for
/// panes.
pub(crate) fn sidebar_width_bounds(window_width: f32) -> (f32, std::ops::RangeInclusive<f32>) {
    let min_w = crate::ui::style::SIDEBAR_MIN_W;
    let max_for_content = (window_width - crate::ui::style::SIDEBAR_CONTENT_MIN_W).max(min_w);
    let max_w = max_for_content.min(crate::ui::style::SIDEBAR_MAX_W);
    let default_w = crate::ui::style::SIDEBAR_DEFAULT_W.clamp(min_w, max_w);
    (default_w, min_w..=max_w)
}

impl PlexiApp {
    /// Render the left sidebar panel with a responsive width clamp that
    /// preserves at least [`crate::ui::style::SIDEBAR_CONTENT_MIN_W`] for the
    /// central content area, restoring the unclamped width when the window
    /// widens again.
    pub(crate) fn show_sidebar_panel(&mut self, ui: &mut egui::Ui) {
        let window_width = ui.available_width();
        let (default_w, size_range) = sidebar_width_bounds(window_width);
        let min_w = *size_range.start();
        let max_w = *size_range.end();
        let max_bits = max_w.to_bits();
        if LAST_LOGGED_SIDEBAR_MAX_BITS.swap(max_bits, std::sync::atomic::Ordering::Relaxed)
            != max_bits
        {
            log::info!(
                "sidebar: width_bounds default={default_w:.1} range={min_w:.1}..={max_w:.1} window_width={window_width:.1} content_min={:.1}",
                crate::ui::style::SIDEBAR_CONTENT_MIN_W,
            );
        }

        let panel_id = egui::Id::new(SIDEBAR_PANEL_ID);
        let desired_id = egui::Id::new(SIDEBAR_DESIRED_WIDTH_ID);
        let clamped_id = egui::Id::new(SIDEBAR_WAS_CLAMPED_ID);
        let ctx = ui.ctx().clone();
        let desired_w = ctx
            .data(|d| d.get_temp::<f32>(desired_id))
            .unwrap_or(crate::ui::style::SIDEBAR_DEFAULT_W)
            .clamp(
                crate::ui::style::SIDEBAR_MIN_W,
                crate::ui::style::SIDEBAR_MAX_W,
            );
        let was_clamped = ctx
            .data(|d| d.get_temp::<bool>(clamped_id))
            .unwrap_or(false);

        if let Some(mut state) = egui::containers::panel::PanelState::load(&ctx, panel_id) {
            let current_w = state.rect.width();
            let target_w = if was_clamped {
                desired_w.clamp(min_w, max_w)
            } else {
                current_w.clamp(min_w, max_w)
            };
            if (current_w - target_w).abs() > f32::EPSILON {
                state.rect.max.x = state.rect.min.x + target_w;
                ctx.data_mut(|d| d.insert_persisted(panel_id, state));
            }
        }

        egui::Panel::left(SIDEBAR_PANEL_ID)
            .default_size(default_w)
            .size_range(size_range)
            .resizable(true)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(self.colors.bg_sidebar)
                    .inner_margin(egui::Margin::same(0)),
            )
            .show_inside(ui, |ui| {
                ui.set_max_width(ui.available_width().min(max_w));
                crate::platform::ui_profile::time("sidebar", || self.draw_sidebar(ui));
            });

        if let Some(state) = egui::containers::panel::PanelState::load(&ctx, panel_id) {
            let actual_w = state.rect.width();
            let is_clamped = max_w < desired_w - f32::EPSILON && actual_w >= max_w - f32::EPSILON;
            ctx.data_mut(|d| {
                d.insert_temp(clamped_id, is_clamped);
                if !is_clamped {
                    d.insert_temp(desired_id, actual_w);
                }
            });
        }
    }

    pub(crate) fn draw_sidebar(&mut self, ui: &mut egui::Ui) {
        let (_, clamp_range) = sidebar_width_bounds(ui.ctx().content_rect().width());
        let sidebar_width = ui.available_width().min(*clamp_range.end());
        ui.set_max_width(sidebar_width);
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
        // One fixed-height entry, however long a diagnostic is. The full text
        // lives in the attention review, never in the navigation header.
        let waiting = crate::overlays::attention::open_items();
        if !waiting.is_empty() {
            let label = crate::overlays::attention::sidebar_label(waiting.len());
            let mut open_attention = false;
            let attention_col = crate::ui::activity::needs_you_color(&self.colors);
            ui.horizontal(|ui| {
                ui.add_space(16.0);
                let width = (ui.available_width() - 12.0).max(0.0);
                let button = egui::Button::new(
                    RichText::new(label)
                        .size(crate::ui::style::TEXT_CAPTION)
                        .color(attention_col),
                )
                .fill(egui::Color32::TRANSPARENT)
                .stroke(Stroke::new(1.0_f32, attention_col))
                .truncate()
                .min_size(Vec2::new(width, crate::ui::style::SIDEBAR_ATTENTION_H));
                if ui
                    .add_sized(
                        Vec2::new(width, crate::ui::style::SIDEBAR_ATTENTION_H),
                        button,
                    )
                    .on_hover_text("Review items that need your attention")
                    .clicked()
                {
                    open_attention = true;
                }
            });
            if open_attention {
                self.open_attention_overlay("sidebar");
            }
            if let Some(item) = crate::overlays::attention::oldest_actionable(&waiting).cloned() {
                self.draw_sidebar_needs_you_actions(ui, &item);
            }
        }
        ui.add_space(4.0);

        let num_contexts = self.router.len();
        let mut clicked_workspace: Option<usize> = None;
        let mut delete_context: Option<usize> = None;
        let mut menu_action: Option<(usize, WindowMenuAction)> = None;
        let mut active_rects: Vec<Rect> = Vec::with_capacity(num_contexts);
        let mut parked_rects: Vec<Rect> = Vec::with_capacity(num_contexts);
        let mut drag_released = false;
        let mut unpark_context: Option<usize> = None;
        let mut parked_header_rect: Option<Rect> = None;

        let focused_cwd = self
            .windows
            .get(self.active_window)
            .and_then(|w| w.focused_pane.map(|tile| (w, tile)))
            .and_then(|(w, tile)| w.get_focused_pane_cwd(tile));

        // Build display order: top-level contexts only. Subcontexts never appear
        // in the sidebar — they are reached from inside their parent via its
        // Portal tile — so they are never enumerated, numbered, or rendered.
        let display_order: Vec<usize> = self.router.top_level_order();

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

        egui::ScrollArea::vertical()
            .id_salt("sidebar_contexts")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // ── Active contexts ─────────────────────────────────────────────
                let active_count = active_order.len();
                for (display_idx, &i) in active_order.iter().enumerate() {
                    let is_active = i == self.router.active_idx();
                    let is_renaming = self.renaming_window == Some(i);
                    let is_dragging = self.drag_context == Some(i);
                    let any_dragging = self.drag_context.is_some();

                    let ctx_id = self.router.get(i).context_id;
                    let pane_dots = self.sidebar_pane_dots(ctx_id, is_active);

                    // --- Renaming: special-cased before the SidebarRow path ---
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
                                        crate::ui::text_field::TextField::singleline(te_id, "")
                                    .surface(crate::ui::focus::SurfaceKey::Overlay(
                                        crate::app::input_owner::OverlaySurface::SidebarRename,
                                    ))
                                    .select_all_on_focus(true)
                                    .log_name("sidebar_rename")
                                    .show(ui, &mut self.rename_buffer, &self.colors)
                                    })
                                    .inner;
                                if te.lost_focus() {
                                    if ui.input(|inp| inp.key_pressed(egui::Key::Escape)) {
                                        self.renaming_window = None;
                                    } else {
                                        self.rename_context(i, &self.rename_buffer.clone());
                                        self.renaming_window = None;
                                    }
                                    ui.input_mut(|inp| {
                                        inp.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
                                        inp.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
                                    });
                                }
                            });
                            ui.add_space(4.0);
                        });
                        let row_rect = scope.response.rect;
                        active_rects.push(row_rect);
                        ui.painter().set(
                            bg_idx,
                            egui::Shape::rect_filled(row_rect, CornerRadius::ZERO, fill),
                        );
                        if is_active {
                            ui.painter().rect_filled(
                                Rect::from_min_size(
                                    row_rect.min,
                                    Vec2::new(3.0, row_rect.height()),
                                ),
                                CornerRadius::ZERO,
                                self.colors.accent,
                            );
                        }
                        continue;
                    }

                    // --- Normal row via SidebarRow ---
                    let ctx_name = self.router.get(i).name.to_string();
                    let badge_count = if is_active {
                        self.visible_notification_count()
                    } else {
                        self.context_notification_count(i)
                    };
                    let subtitle = Some(self.router.get(i).root.display().to_string());
                    let indicator_color = self.sidebar_context_indicator_color(ctx_id);

                    let (action, response) = SidebarRow {
                        is_active,
                        is_dragging,
                        any_dragging,
                        action_enabled: num_contexts > 1 && !any_dragging,
                        ctx_name,
                        ctx_index: Some(display_idx),
                        badge_count,
                        subtitle,
                        pane_dots,
                        draggable: true,
                    }
                    .show(ui, egui::Id::new(("ctx", i)), &self.colors);

                    if let Some(ind_col) = indicator_color {
                        ui.painter().rect_filled(
                            Rect::from_min_size(
                                response.rect.min,
                                Vec2::new(3.0, response.rect.height()),
                            ),
                            CornerRadius::ZERO,
                            ind_col,
                        );
                    }

                    active_rects.push(response.rect);

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
                        response.context_menu(|ui| {
                            if ui.button("Rename").clicked() {
                                menu_action = Some((i, WindowMenuAction::Rename));
                                ui.close();
                            }
                            if ui.button("Edit Description").clicked() {
                                menu_action = Some((i, WindowMenuAction::EditDescription));
                                ui.close();
                            }
                            ui.separator();
                            if display_idx > 0 {
                                if ui.button("Move to Top").clicked() {
                                    menu_action = Some((i, WindowMenuAction::MoveToTop));
                                    ui.close();
                                }
                                if ui.button("Move Up").clicked() {
                                    menu_action = Some((i, WindowMenuAction::MoveUp));
                                    ui.close();
                                }
                            }
                            if display_idx + 1 < active_count {
                                if ui.button("Move Down").clicked() {
                                    menu_action = Some((i, WindowMenuAction::MoveDown));
                                    ui.close();
                                }
                                if ui.button("Move to Bottom").clicked() {
                                    menu_action = Some((i, WindowMenuAction::MoveToBottom));
                                    ui.close();
                                }
                            }
                            ui.separator();
                            if let Some(cwd) = &cwd_for_menu {
                                if ui.button("Set root to current path").clicked() {
                                    menu_action = Some((i, WindowMenuAction::SetRoot(cwd.clone())));
                                    ui.close();
                                }
                            }
                            if ui.button("Edit root\u{2026}").clicked() {
                                menu_action = Some((i, WindowMenuAction::OpenRootOverlay));
                                ui.close();
                            }
                            if num_ctxs > 1 {
                                ui.separator();
                                if ui.button("Park").clicked() {
                                    menu_action = Some((i, WindowMenuAction::Park));
                                    ui.close();
                                }
                                if ui.button("Delete").clicked() {
                                    menu_action = Some((i, WindowMenuAction::Delete));
                                    ui.close();
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
                // The header doubles as a drop target: while a context is being
                // dragged, render it even when empty so the drag can release onto it.
                let parked_count = parked_order.len();
                if parked_count > 0 || self.drag_context.is_some() {
                    ui.add_space(8.0);

                    let divider_id = egui::Id::new("parked_divider");
                    let expanded = self.parked_section_expanded;
                    let label = if parked_count > 0 {
                        format!("Parked ({parked_count})")
                    } else {
                        "Parked".to_string()
                    };

                    let response = ListDropdownHeader::new(&label, expanded).indent(12.0).show(
                        ui,
                        divider_id,
                        &self.colors,
                    );
                    parked_header_rect = Some(response.rect);
                    if response.clicked() {
                        self.parked_section_expanded = !self.parked_section_expanded;
                    }
                    ui.add_space(4.0);

                    if self.parked_section_expanded {
                        for &i in &parked_order {
                            let ctx = self.router.get(i);
                            let ctx_name = ctx.name.to_string();
                            let ctx_id = ctx.context_id;
                            let subtitle = Some(ctx.root.display().to_string());

                            let pane_dots = self.sidebar_pane_dots(ctx_id, false);
                            let indicator_color = self.sidebar_context_indicator_color(ctx_id);

                            let (action, response) = SidebarRow {
                                is_active: false,
                                is_dragging: self.drag_context == Some(i),
                                any_dragging: self.drag_context.is_some(),
                                action_enabled: false,
                                ctx_name,
                                ctx_index: None,
                                badge_count: 0,
                                subtitle,
                                pane_dots,
                                draggable: true,
                            }
                            .show(
                                ui,
                                egui::Id::new(("parked_ctx", i)),
                                &self.colors,
                            );

                            if let Some(ind_col) = indicator_color {
                                ui.painter().rect_filled(
                                    Rect::from_min_size(
                                        response.rect.min,
                                        Vec2::new(3.0, response.rect.height()),
                                    ),
                                    CornerRadius::ZERO,
                                    ind_col,
                                );
                            }

                            parked_rects.push(response.rect);

                            response.context_menu(|ui| {
                                if ui.button("Unpark").clicked() {
                                    unpark_context = Some(i);
                                    ui.close();
                                }
                            });

                            match action {
                                SidebarAction::DragStart => {
                                    self.drag_context = Some(i);
                                }
                                SidebarAction::DragEnd => {
                                    drag_released = true;
                                }
                                SidebarAction::Activate => {
                                    unpark_context = Some(i);
                                }
                                _ => {}
                            }
                        }
                    }
                }
            });

        // Resolve the drag target once and use it for both the drop affordance
        // and the release action. The Parked header is the section boundary:
        // dropping at/below it targets the parked list, above it the active
        // list — in either case at a chosen slot, so a drag can park, unpark,
        // and reorder within either list.
        let pointer_pos = ui.input(|i| i.pointer.hover_pos());
        let src_section = self.drag_context.map(|src| {
            let src_parked = self.router.get(src).parked;
            let src_slot = if src_parked {
                parked_order.iter().position(|&i| i == src).unwrap_or(0)
            } else {
                active_order.iter().position(|&i| i == src).unwrap_or(0)
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
                        Stroke::new(1.5_f32, self.colors.accent),
                        egui::StrokeKind::Inside,
                    );
                }
            } else {
                let line_y = reorder_line_y(target_rects, drop.slot);
                let x0 = target_rects.first().map_or(0.0, |r| r.min.x);
                ui.painter().line_segment(
                    [
                        egui::pos2(x0, line_y),
                        egui::pos2(x0 + sidebar_width, line_y),
                    ],
                    Stroke::new(2.0_f32, self.colors.accent),
                );
            }
        }

        if drag_released {
            if let (Some(src), Some(drop), Some((src_parked, src_slot))) =
                (self.drag_context, drop_target, src_section)
            {
                // Rebuild the active/parked section lists with `src` moved to its
                // new home, then commit the full ordering + parked flag at once.
                let mut new_active = active_order.clone();
                let mut new_parked = parked_order.clone();
                if src_parked {
                    new_parked.retain(|&i| i != src);
                } else {
                    new_active.retain(|&i| i != src);
                }
                let dest = if drop.parked {
                    &mut new_parked
                } else {
                    &mut new_active
                };
                // Same-section moves past the source's old slot shift down by one
                // once it is removed.
                let slot = if drop.parked == src_parked && drop.slot > src_slot {
                    drop.slot - 1
                } else {
                    drop.slot
                };
                dest.insert(slot.min(dest.len()), src);

                let src_id = self.router.get(src).context_id;
                let was_active = self.router.active_idx() == src;
                self.router.get_mut(src).parked = drop.parked;
                let mut new_order = new_active;
                new_order.extend(new_parked);
                // These lists hold top-level rows only; re-inflate each row's
                // hidden subtree so `apply_order` receives a full permutation
                // and the subcontexts travel with their parent.
                let new_order = self.router.expand_subtrees(&new_order);
                self.router.apply_order(&new_order);
                self.renaming_window = None;

                let new_src_idx = self
                    .router
                    .position(|c| c.context_id == src_id)
                    .unwrap_or(self.router.active_idx());
                if drop.parked && !src_parked {
                    log::info!("sidebar: drag-park context id={src_id} slot={}", drop.slot);
                    if was_active {
                        // Hand focus to the nearest unparked neighbor.
                        let len = self.router.len();
                        let next = (1..len)
                            .map(|o| (new_src_idx + o) % len)
                            .find(|&i| !self.router.get(i).parked);
                        if let Some(n) = next {
                            self.switch_workspace(n);
                        }
                    }
                } else if !drop.parked && src_parked {
                    log::info!(
                        "sidebar: drag-unpark context id={src_id} slot={}",
                        drop.slot
                    );
                    self.switch_workspace(new_src_idx);
                } else {
                    log::info!(
                        "sidebar: drag-reorder context id={src_id} parked={} slot={}",
                        drop.parked,
                        drop.slot
                    );
                }
                self.mark_workspace_dirty();
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
                    self.push_focus_layer(crate::app::FocusKind::ContextDescription);
                }
                WindowMenuAction::MoveToTop => {
                    self.renaming_window = None;
                    self.router.reorder_top_level(i, ContextMove::Top);
                }
                WindowMenuAction::MoveUp => {
                    self.renaming_window = None;
                    self.router.reorder_top_level(i, ContextMove::Up);
                }
                WindowMenuAction::MoveDown => {
                    self.renaming_window = None;
                    self.router.reorder_top_level(i, ContextMove::Down);
                }
                WindowMenuAction::MoveToBottom => {
                    self.renaming_window = None;
                    self.router.reorder_top_level(i, ContextMove::Bottom);
                }
                WindowMenuAction::SetRoot(path) => {
                    log::info!(
                        "sidebar: set context root ctx_idx={i} root={}",
                        path.display()
                    );
                    let ctx_id = self.router.get(i).context_id;
                    self.set_context_root(path, Some(ctx_id));
                    self.mark_workspace_dirty();
                }
                WindowMenuAction::OpenRootOverlay => {
                    let existing = self.router.get(i).root.display().to_string();
                    log::info!("TextInputOverlay: opened target=ContextRoot({i}) via sidebar");
                    self.text_overlay_browse_rx = None;
                    self.text_overlay = Some((
                        crate::app::TextInputOverlay {
                            label: "Set context root".to_string(),
                            hint: "/path/to/project or ~/...".to_string(),
                            buffer: existing,
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
            // Only top-level rows are rendered, so a click here is always a
            // lateral move between master contexts — never a descent into a
            // child, which is what the Portal tile is for. No depth push.
            self.switch_workspace(i);
        }

        if add_clicked {
            self.new_context();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn header() -> Rect {
        Rect::from_min_size(pos2(0.0, 50.0), vec2(100.0, 20.0))
    }

    #[test]
    fn sidebar_pane_dots_marks_the_saved_window_and_pane_for_inactive_contexts() {
        let ctx = egui::Context::default();
        let tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let (mut app, _) = PlexiApp::new_for_test(ctx, tick);
        let _ = app.add_test_pane();

        let context_id = 41;
        app.router.push(crate::host::context::Context {
            name: "return target".to_string().into(),
            root: std::path::PathBuf::from("/tmp"),
            description: None,
            context_id,
            parent_id: None,
            depth: 0,
            parked: false,
        });
        for (window_id, grid_x, pane_id) in [(42, 0, 420), (43, 1, 430)] {
            let mut tree = egui_tiles::Tree::empty(format!("sidebar-{window_id}"));
            let tile = tree.tiles.insert_pane(pane_id);
            tree.root = Some(tile);
            app.windows.push(crate::host::context::Window {
                name: String::new(),
                path: std::path::PathBuf::from("/tmp"),
                tree,
                panes: std::collections::HashMap::new(),
                focused_pane: (window_id == 43).then_some(tile),
                zoomed_pane: None,
                grid_x,
                grid_y: 0,
                window_id,
                context_id,
            });
        }
        app.context_active_window.insert(context_id, 43);

        let dots = app
            .sidebar_pane_dots(context_id, false)
            .expect("inactive context panes should render");
        assert_eq!(dots.focused_idx, Some(1));
        assert_eq!(dots.windows.len(), 2);
        assert!(!dots.windows[0].is_return_target);
        assert!(dots.windows[1].is_return_target);
        assert!(!dots.windows[1].is_active);
    }

    fn resolve(
        src_parked: bool,
        src_slot: usize,
        pointer_y: f32,
        parked_rects: &[Rect],
    ) -> Option<SidebarDrop> {
        resolve_drag_drop(
            src_parked,
            src_slot,
            Some(header()),
            Some(pos2(10.0, pointer_y)),
            &active_rects(),
            parked_rects,
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
                slot: 0
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
                slot: 1
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
                slot: 0
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
                slot: 2
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
                slot: 2
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
            &active_rects(),
            &parked_rects(),
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

    #[test]
    fn sidebar_width_bounds_preserves_content_floor_at_400pt_and_standard_widths() {
        use crate::ui::style::{
            SIDEBAR_CONTENT_MIN_W, SIDEBAR_DEFAULT_W, SIDEBAR_MAX_W, SIDEBAR_MIN_W,
        };

        // At the host's 400-point minimum window width, the sidebar clamps to
        // SIDEBAR_MIN_W (140 pt) so at least SIDEBAR_CONTENT_MIN_W (260 pt)
        // remains for the central content area.
        let (default_400, range_400) = sidebar_width_bounds(400.0);
        assert_eq!(default_400, SIDEBAR_MIN_W);
        assert_eq!(*range_400.start(), SIDEBAR_MIN_W);
        assert_eq!(*range_400.end(), 400.0 - SIDEBAR_CONTENT_MIN_W);
        assert!(400.0 - *range_400.end() >= SIDEBAR_CONTENT_MIN_W);

        // At an intermediate narrow width (460 pt), the sidebar cap is 200 pt
        // and the default clamps to 200 pt, keeping the 260 pt content floor.
        let (default_460, range_460) = sidebar_width_bounds(460.0);
        assert_eq!(default_460, 200.0);
        assert_eq!(*range_460.end(), 200.0);
        assert!(460.0 - *range_460.end() >= SIDEBAR_CONTENT_MIN_W);

        // At 520 pt (responsive narrow breakpoint), the default 220 pt fits
        // while the user resize cap is 260 pt (520 - 260).
        let (default_520, range_520) = sidebar_width_bounds(520.0);
        assert_eq!(default_520, SIDEBAR_DEFAULT_W);
        assert_eq!(*range_520.end(), 520.0 - SIDEBAR_CONTENT_MIN_W);

        // At standard widths (800 pt, 1280 pt), the full SIDEBAR_MIN_W..=SIDEBAR_MAX_W
        // range and SIDEBAR_DEFAULT_W are available.
        for &w in &[800.0_f32, 1280.0, 1400.0] {
            let (default_w, range) = sidebar_width_bounds(w);
            assert_eq!(default_w, SIDEBAR_DEFAULT_W);
            assert_eq!(*range.start(), SIDEBAR_MIN_W);
            assert_eq!(*range.end(), SIDEBAR_MAX_W);
            assert!(w - *range.end() >= SIDEBAR_CONTENT_MIN_W);
        }
    }
}
