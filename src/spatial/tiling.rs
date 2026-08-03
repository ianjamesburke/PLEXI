use crate::app_protocol::AgentState;
use crate::host::pane::{AppRuntime, Pane, TerminalPane};
use crate::render;
use crate::ui::style;
use crate::ui::theme::Colors;
use egui_term::{BackendCommand, TerminalTheme};
use egui_tiles::{
    Behavior, ResizeState, SimplificationOptions, TabState, TileId, Tiles, UiResponse,
};
use std::collections::HashMap;
use std::path::PathBuf;

pub type PaneId = u64;

pub(crate) const TAB_BAR_HEIGHT: f32 = 20.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TabBarAction {
    Switch(usize),
    Reorder { from_idx: usize, to_idx: usize },
}

#[derive(Clone, Copy)]
struct TabDragState {
    container_tile: TileId,
    from_idx: usize,
    start_pos: egui::Pos2,
}

#[derive(Clone)]
pub struct TabGroupInfo {
    pub active_idx: usize,
    /// Primary pane ID for each tab, in display order.
    pub members: Vec<PaneId>,
    /// The TileId of the `Container::Tabs` that owns this group.
    pub container_tile: TileId,
}

fn tab_drop_marker_x(bar_rect: egui::Rect, tab_width: f32, from_idx: usize, to_idx: usize) -> f32 {
    let target_left = bar_rect.left() + to_idx as f32 * tab_width;
    if to_idx > from_idx {
        target_left + tab_width
    } else {
        target_left
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_drop_marker_tracks_insert_edge() {
        let bar = egui::Rect::from_min_size(egui::pos2(10.0, 0.0), egui::vec2(300.0, 20.0));
        let tab_width = 100.0;

        assert_eq!(tab_drop_marker_x(bar, tab_width, 2, 0), 10.0);
        assert_eq!(tab_drop_marker_x(bar, tab_width, 0, 2), 310.0);
        assert_eq!(tab_drop_marker_x(bar, tab_width, 1, 1), 110.0);
    }
}

/// Render a full-width tab bar and return a switch/reorder action, if any.
/// The caller must pre-allocate `bar_rect` (typically `TAB_BAR_HEIGHT` px tall)
/// and advance the cursor past it.
// Arg-struct refactor is a design change tracked in stint 0661.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_tab_bar(
    ctx: &egui::Context,
    painter: &egui::Painter,
    bar_rect: egui::Rect,
    group: &TabGroupInfo,
    tab_labels: &HashMap<PaneId, String>,
    tab_activities: &HashMap<PaneId, AgentState>,
    colors: &Colors,
    font_size: f32,
    overtake_hint: bool,
) -> Option<TabBarAction> {
    painter.rect_filled(bar_rect, 0.0, colors.pane_header_bg());

    let tab_count = group.members.len();
    if tab_count == 0 {
        return None;
    }

    let tab_width = bar_rect.width() / tab_count as f32;
    let accent_bar_height = 2.0;
    let font = egui::FontId::proportional(font_size);

    let tab_idx_at = |pos: egui::Pos2| -> Option<usize> {
        if !bar_rect.contains(pos) {
            return None;
        }
        Some((((pos.x - bar_rect.left()) / tab_width).floor() as usize).min(tab_count - 1))
    };

    let drag_id = egui::Id::new(("plexi_tab_drag", group.container_tile));
    let (pressed, down, released, pos) = ctx.input(|i| {
        (
            i.pointer.primary_pressed(),
            i.pointer.primary_down(),
            i.pointer.any_released(),
            i.pointer.interact_pos(),
        )
    });
    let mut action = None;
    let mut drag_marker: Option<(usize, egui::Pos2)> = None;

    if pressed {
        if let Some(pos) = pos {
            if let Some(from_idx) = tab_idx_at(pos) {
                ctx.data_mut(|d| {
                    d.insert_temp(
                        drag_id,
                        TabDragState {
                            container_tile: group.container_tile,
                            from_idx,
                            start_pos: pos,
                        },
                    );
                });
            }
        }
    }

    if released {
        let state = ctx.data(|d| d.get_temp::<TabDragState>(drag_id));
        ctx.data_mut(|d| d.remove::<TabDragState>(drag_id));
        if let (Some(state), Some(pos)) = (state, pos) {
            if state.container_tile == group.container_tile {
                let moved = pos.distance_sq(state.start_pos) > 16.0;
                if moved {
                    if let Some(to_idx) = tab_idx_at(pos) {
                        if to_idx != state.from_idx {
                            action = Some(TabBarAction::Reorder {
                                from_idx: state.from_idx,
                                to_idx,
                            });
                        }
                    }
                } else if let Some(idx) = tab_idx_at(pos) {
                    if idx != group.active_idx {
                        action = Some(TabBarAction::Switch(idx));
                    }
                }
            }
        }
    } else if !down {
        ctx.data_mut(|d| d.remove::<TabDragState>(drag_id));
    } else if let Some(pos) = pos {
        let state = ctx.data(|d| d.get_temp::<TabDragState>(drag_id));
        if let Some(state) = state {
            if state.container_tile == group.container_tile
                && pos.distance_sq(state.start_pos) > 16.0
            {
                if let Some(to_idx) = tab_idx_at(pos) {
                    if to_idx != state.from_idx {
                        drag_marker = Some((state.from_idx, pos));
                    }
                }
            }
        }
    }

    for (i, &pane_id) in group.members.iter().enumerate() {
        let is_active = i == group.active_idx;
        let tab_rect = egui::Rect::from_min_size(
            egui::pos2(bar_rect.left() + i as f32 * tab_width, bar_rect.top()),
            egui::vec2(tab_width, TAB_BAR_HEIGHT),
        );

        if is_active {
            painter.rect_filled(tab_rect, 0.0, colors.bg_active);
        }

        // Vertical divider between tabs (skip before first)
        if i > 0 {
            let divider_x = tab_rect.left();
            let inset = 4.0;
            painter.line_segment(
                [
                    egui::pos2(divider_x, bar_rect.top() + inset),
                    egui::pos2(divider_x, bar_rect.bottom() - inset),
                ],
                egui::Stroke::new(1.5_f32, colors.border),
            );
        }

        const TAB_PIP_RADIUS: f32 = 4.0;
        const TAB_PIP_LEFT_PAD: f32 = 7.0;
        let has_pip = tab_activities.contains_key(&pane_id);
        let pip_space = if has_pip {
            TAB_PIP_LEFT_PAD + TAB_PIP_RADIUS * 2.0
        } else {
            0.0
        };

        if let Some(state) = tab_activities.get(&pane_id) {
            let t = ctx.input(|i| i.time);
            let color = crate::ui::activity::dot_color_from_time(state, colors, t, i);
            let cx = tab_rect.left() + TAB_PIP_LEFT_PAD + TAB_PIP_RADIUS;
            painter.circle_filled(egui::pos2(cx, tab_rect.center().y), TAB_PIP_RADIUS, color);
            if matches!(state, AgentState::Working) {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }

        let label = tab_labels
            .get(&pane_id)
            .cloned()
            .unwrap_or_else(|| "Tab".to_string());

        let text_color = if is_active {
            colors.text_primary
        } else {
            colors.text_dim
        };
        let text_pad = 8.0;
        let content_left = tab_rect.left() + pip_space;
        let max_text_width = (tab_rect.right() - content_left - text_pad).max(0.0);

        let mut layout_job = egui::text::LayoutJob::default();
        layout_job.append(
            &label,
            0.0,
            egui::TextFormat {
                font_id: font.clone(),
                color: text_color,
                ..Default::default()
            },
        );
        layout_job.wrap = egui::text::TextWrapping {
            max_width: max_text_width,
            max_rows: 1,
            break_anywhere: true,
            overflow_character: Some('…'),
        };
        let galley = painter.ctx().fonts_mut(|f| f.layout_job(layout_job));

        let text_pos = egui::pos2(
            content_left + (tab_rect.right() - content_left) / 2.0 - galley.size().x / 2.0,
            tab_rect.center().y - galley.size().y / 2.0,
        );
        crate::ui::snap::galley_snapped(painter, text_pos, galley, text_color);

        if is_active {
            let accent_rect = egui::Rect::from_min_size(
                egui::pos2(tab_rect.left(), tab_rect.bottom() - accent_bar_height),
                egui::vec2(tab_width, accent_bar_height),
            );
            painter.rect_filled(accent_rect, 0.0, colors.accent);
        }
    }

    if let Some((from_idx, pos)) = drag_marker {
        if let Some(to_idx) = tab_idx_at(pos) {
            let x = tab_drop_marker_x(bar_rect, tab_width, from_idx, to_idx);
            let marker_rect = egui::Rect::from_center_size(
                egui::pos2(x, bar_rect.center().y),
                egui::vec2(3.0, TAB_BAR_HEIGHT - 3.0),
            );
            painter.rect_filled(marker_rect, 1.5, colors.accent);
        }
    }

    if overtake_hint {
        let hint_font = egui::FontId::monospace(font_size - 1.0);
        let hint_text = "Esc";
        let hint_galley = painter.layout_no_wrap(hint_text.to_string(), hint_font, colors.text_dim);
        let chip_pad = 4.0;
        let chip_w = hint_galley.size().x + chip_pad * 2.0;
        let chip_h = (TAB_BAR_HEIGHT - 6.0).max(10.0);
        let chip_rect = egui::Rect::from_min_size(
            egui::pos2(
                bar_rect.right() - chip_w - 4.0,
                bar_rect.center().y - chip_h / 2.0,
            ),
            egui::vec2(chip_w, chip_h),
        );
        painter.rect_filled(chip_rect, egui::CornerRadius::same(3), colors.bg_active);
        painter.rect_stroke(
            chip_rect,
            egui::CornerRadius::same(3),
            egui::Stroke::new(1.0_f32, colors.border),
            egui::StrokeKind::Inside,
        );
        crate::ui::snap::text_snapped(
            painter,
            chip_rect.center(),
            egui::Align2::CENTER_CENTER,
            hint_text,
            egui::FontId::monospace(font_size - 1.0),
            colors.text_dim,
        );
    }

    action
}

/// What kind of pane occupies a minimap slot.
#[derive(Clone, Copy, PartialEq)]
pub enum PaneKind {
    Terminal,
    App,
    /// Text/editor app pane (manifest `text-editor`). Renders a document glyph
    /// in the minimap instead of the generic app mark.
    TextEditor,
    Portal,
}

/// A single pane slot within one window's minimap.
#[derive(Clone)]
pub struct MiniPane {
    /// Normalized [0,1]×[0,1] rect within its window body.
    pub norm_rect: egui::Rect,
    pub kind: PaneKind,
    pub focused: bool,
    /// True when the pane has meaningful content (always true for running panes).
    pub has_content: bool,
    /// OSC title or app name, if available.
    pub title: Option<String>,
    /// False for terminal panes that have exited; dims the pane in the minimap.
    pub active: bool,
    /// Agent/terminal activity state; renders a dot in the pane's top-left corner.
    pub activity: Option<crate::app_protocol::AgentState>,
}

/// One window in the child context, with its spatial grid position.
#[derive(Clone)]
pub struct MiniWindow {
    pub grid_x: u32,
    pub grid_y: u32,
    pub panes: Vec<MiniPane>,
}

/// Preview data for a Portal tile.
#[derive(Clone)]
pub struct PortalPreview {
    pub context_name: String,
    pub context_description: String,
    pub pane_count: usize,
    pub notification_count: usize,
    pub windows: Vec<MiniWindow>,
    pub window_count: usize,
}

impl Default for PortalPreview {
    fn default() -> Self {
        PortalPreview {
            context_name: "(deleted)".to_string(),
            context_description: String::new(),
            pane_count: 0,
            notification_count: 0,
            windows: Vec::new(),
            window_count: 0,
        }
    }
}

pub struct PlexiBehavior<'a> {
    pub panes: &'a mut HashMap<PaneId, Pane>,
    pub theme: TerminalTheme,
    pub new_focused: Option<TileId>,
    pub close_exited: Option<TileId>,
    /// Set when a tab bar switches or reorders tabs.
    pub tab_action: Option<(TileId, TabBarAction)>,
    pub tab_info: HashMap<TileId, TabGroupInfo>,
    /// Pre-computed display label for each pane: user-set name, then app name, then type string.
    pub tab_labels: HashMap<PaneId, String>,
    pub zoomed_pane: Option<TileId>,
    pub colors: Colors,
    pub pane_names: HashMap<PaneId, String>,
    pub drag_cursor_pos: Option<egui::Pos2>,
    /// Cached once per frame — true if files are being dragged over the window.
    /// Avoids O(n) `ui.input()` calls inside `pane_ui` for each background pane.
    pub hovered_files: bool,
    /// The active workspace root (or `None` when running outside a workspace).
    /// Used by `terminal_pane::render` to flag terminal panes whose CWD has
    /// drifted outside the workspace tree. See issue #308 Phase 1.
    pub workspace_root: Option<PathBuf>,
    /// The rendered window's context root, pushed into app panes every frame
    /// so state-scope resolution follows `plexi context set-root` at call
    /// time (stint 0652).
    pub context_root: PathBuf,
    /// Opacity applied to unfocused panes when ghost mode is active.
    /// `None` = no dimming. Values below 1.0 dim all non-focused panes.
    pub unfocused_opacity: Option<f32>,
    /// Preview data for Portal tiles.
    pub portal_info: HashMap<PaneId, PortalPreview>,
    /// The pane that owns keyboard input this frame, derived once by
    /// `PlexiApp::owner_pane` (stint 0429). `None` while an overlay owns
    /// input, so every pane renders unfocused under a modal.
    pub owner_pane: Option<PaneId>,
    /// True when the Control modifier is held — triggers the pane ID ghost overlay.
    pub ctrl_held: bool,
    /// Set by double-click on a Portal pane — the target context_id to zoom into.
    pub portal_zoom_request: Option<u64>,
    /// Resolved inter-pane gap width (from config, default 4.0).
    pub pane_gap: f32,
    /// Resolved pane title bar font size (from config, default 11.0).
    pub pane_title_font_size: f32,
    /// Keyboard input the host took from `ctx` before the render pass, destined
    /// for the focused terminal pane (stint 0387). Consumed (via `take`) by the
    /// pane whose `is_focused` is true, so egui's render-time widget machinery
    /// can't swallow a key (Cmd+A) first. `None` when the focused pane is not a
    /// terminal (or an overlay owns input).
    pub focused_terminal_input: Option<crate::render::terminal_pane::TerminalInput>,
    /// This frame's modifier state, used to build the empty [`TerminalInput`]
    /// for unfocused terminals (which still need current modifiers for pointer
    /// link-hover and mouse reporting).
    pub frame_modifiers: egui::Modifiers,
    /// Pending `AppRequest::ClickPane` injections, keyed by target pane.
    /// Consumed (via `remove`) by whichever pane actually renders this frame —
    /// see `crate::host::pane::PendingPaneClick`.
    pub pending_pane_clicks: &'a mut HashMap<PaneId, crate::host::pane::PendingPaneClick>,
}

impl Behavior<PaneId> for PlexiBehavior<'_> {
    fn pane_ui(&mut self, ui: &mut egui::Ui, tile_id: TileId, pane_id: &mut PaneId) -> UiResponse {
        // While any pane is zoomed, paint background panes as dark placeholders
        // and skip all input detection — the zoom overlay owns focus and drop
        // handling. This avoids per-pane `ui.input()` calls during hover, which
        // were O(n) and ran even when the results could never be acted on.
        if self.zoomed_pane.is_some() {
            let pane_rect = ui.available_rect_before_wrap();
            ui.painter()
                .rect_filled(pane_rect, 0.0, self.colors.bg_darkest);
            return UiResponse::None;
        }

        // Detect clicks or file drags for focus.
        let is_click =
            ui.input(|i| i.pointer.any_pressed()) && ui.rect_contains_pointer(ui.max_rect());
        let is_drag_hovering = match self.drag_cursor_pos {
            Some(pos) => ui.max_rect().contains(pos),
            None => self.hovered_files && ui.rect_contains_pointer(ui.max_rect()),
        };
        if is_click || is_drag_hovering {
            self.new_focused = Some(tile_id);
        }

        let is_focused = self.owner_pane == Some(*pane_id);

        let is_hidden = self.panes.get(pane_id).is_some_and(|p| p.is_hidden());

        if is_hidden {
            ui.set_opacity(0.4);
        } else if !is_focused {
            if let Some(opacity) = self.unfocused_opacity {
                if opacity < 1.0 {
                    ui.set_opacity(opacity);
                }
            }
        }

        // Drop target: the zoomed overlay owns drops when a pane is zoomed,
        // so this path only runs when zoomed_pane.is_none() (guaranteed above).
        if is_drag_hovering {
            if let Some(t) = self.panes.get_mut(pane_id).and_then(Pane::as_terminal_mut) {
                write_dropped_paths_to_terminal(ui, t);
            } else if let Some(app) = self.panes.get_mut(pane_id).and_then(Pane::as_app_mut) {
                deliver_dropped_files_to_app(ui, *pane_id, app);
            }
        }

        let pane_rect = ui.available_rect_before_wrap();

        let tab_activities: HashMap<PaneId, AgentState> = self
            .tab_info
            .get(&tile_id)
            .map(|group| {
                group
                    .members
                    .iter()
                    .filter_map(|id| {
                        self.panes
                            .get(id)
                            .and_then(|p| p.effective_activity().cloned())
                            .map(|a| (*id, a))
                    })
                    .collect()
            })
            .unwrap_or_default();

        let Some(pane) = self.panes.get_mut(pane_id) else {
            return UiResponse::None;
        };

        if let Some(app_pane) = pane.as_app_mut() {
            ui.painter()
                .rect_filled(pane_rect, 0.0, self.colors.bg_darkest);
            let mut app_ui = ui.new_child(egui::UiBuilder::new().max_rect(pane_rect));

            let tab_group = self.tab_info.get(&tile_id);
            let has_tabs = tab_group.is_some();
            if let Some(group) = tab_group {
                let bar_rect = egui::Rect::from_min_size(
                    app_ui.cursor().min,
                    egui::vec2(app_ui.available_width(), TAB_BAR_HEIGHT),
                );
                app_ui.advance_cursor_after_rect(bar_rect);
                let is_overtaken = app_pane.overlay_replaced.is_some();
                if let Some(action) = paint_tab_bar(
                    app_ui.ctx(),
                    app_ui.painter(),
                    bar_rect,
                    group,
                    &self.tab_labels,
                    &tab_activities,
                    &self.colors,
                    self.pane_title_font_size,
                    is_overtaken,
                ) {
                    self.tab_action = Some((group.container_tile, action));
                }
            }

            let pending_click = self.pending_pane_clicks.remove(pane_id);
            let ui_profile_label = match &app_pane.runtime {
                AppRuntime::Python(_) => format!("python:{}", app_pane.manifest_id),
                AppRuntime::Wasm(_) => format!("wasm:{}", app_pane.manifest_id),
                AppRuntime::Builtin(_) => format!("builtin:{}", app_pane.manifest_id),
            };
            crate::platform::ui_profile::time(&ui_profile_label, || {
                render::app_pane::render(
                    &mut app_ui,
                    app_pane,
                    &self.colors,
                    has_tabs,
                    pending_click,
                    &self.context_root,
                );
            });
        } else if let Some(terminal) = pane.as_terminal_mut() {
            ui.painter()
                .rect_filled(pane_rect, 0.0, self.colors.terminal_bg);
            let mut terminal_ui = ui.new_child(egui::UiBuilder::new().max_rect(pane_rect));
            // The focused terminal consumes the host-supplied keyboard buffer;
            // every other terminal gets an empty one (pointer/wheel only).
            let terminal_input = if is_focused {
                self.focused_terminal_input.take().unwrap_or_else(|| {
                    render::terminal_pane::TerminalInput {
                        keyboard_events: Vec::new(),
                        modifiers: self.frame_modifiers,
                    }
                })
            } else {
                render::terminal_pane::TerminalInput {
                    keyboard_events: Vec::new(),
                    modifiers: self.frame_modifiers,
                }
            };
            let (close_exited, tab_action) = crate::platform::ui_profile::time("terminal", || {
                render::terminal_pane::render(
                    &mut terminal_ui,
                    terminal,
                    tile_id,
                    pane_id,
                    is_focused,
                    &self.theme,
                    &self.colors,
                    &self.pane_names,
                    &self.tab_info,
                    &self.tab_labels,
                    &tab_activities,
                    self.workspace_root.as_deref(),
                    self.pane_title_font_size,
                    terminal_input,
                )
            });
            if close_exited {
                self.close_exited = Some(tile_id);
            }
            if tab_action.is_some() {
                self.tab_action = tab_action;
            }
        } else if pane.as_portal().is_some() {
            crate::platform::ui_profile::time("portal", || {
                // Portal tile — direct egui rendering.
                ui.painter()
                    .rect_filled(pane_rect, 0.0, self.colors.bg_darkest);
                let preview = self.portal_info.get(pane_id).cloned().unwrap_or_default();
                let padding = style::SPACE_MD;
                let inner = pane_rect.shrink(padding);
                let mut portal_ui = ui.new_child(egui::UiBuilder::new().max_rect(inner));
                let colors_for_portal = self.colors;
                portal_ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                    ui.label(
                        egui::RichText::new(&preview.context_name)
                            .size(style::TEXT_BODY)
                            .strong()
                            .color(colors_for_portal.text_primary),
                    );
                    if !preview.context_description.is_empty() {
                        ui.scope(|ui| {
                            ui.set_max_width(inner.width());
                            crate::ui::labels::description_label(
                                ui,
                                &preview.context_description,
                                &colors_for_portal,
                            );
                        });
                    }
                    {
                        let count_label =
                            if preview.window_count > 1 && preview.notification_count > 0 {
                                format!(
                                    "{} panes \u{b7} {} windows \u{b7} {} notifs",
                                    preview.pane_count,
                                    preview.window_count,
                                    preview.notification_count
                                )
                            } else if preview.window_count > 1 {
                                format!(
                                    "{} panes \u{b7} {} windows",
                                    preview.pane_count, preview.window_count
                                )
                            } else if preview.notification_count > 0 {
                                format!(
                                    "{} panes \u{b7} {} notifs",
                                    preview.pane_count, preview.notification_count
                                )
                            } else {
                                format!("{} panes", preview.pane_count)
                            };
                        ui.label(
                            egui::RichText::new(count_label)
                                .size(style::TEXT_HINT)
                                .color(colors_for_portal.text_dim),
                        );
                    }
                    ui.add_space(style::SPACE_SM);
                    if !preview.windows.is_empty() {
                        let header_used = ui.cursor().min.y - inner.min.y;
                        let map_h = (inner.height() - header_used).max(24.0);
                        let (map_area, _) = ui.allocate_exact_size(
                            egui::vec2(inner.width(), map_h),
                            egui::Sense::hover(),
                        );
                        let t = ui.input(|i| i.time);
                        if preview.windows.iter().flat_map(|w| &w.panes).any(|p| {
                            matches!(p.activity, Some(crate::app_protocol::AgentState::Working))
                        }) {
                            // Pulse only needs ~10fps; an unconditional
                            // request_repaint pins the window at display refresh.
                            ui.ctx()
                                .request_repaint_after(std::time::Duration::from_millis(100));
                        }
                        let painter = ui.painter();
                        paint_portal_minimap(
                            painter,
                            map_area,
                            &preview.windows,
                            &colors_for_portal,
                            t,
                        );
                    }
                });
            });
            // Double-click on portal pane to zoom into the sub-context.
            if let Some(target_ctx_id) = self.panes.get(pane_id).and_then(|p| p.portal_target()) {
                if ui.rect_contains_pointer(pane_rect)
                    && ui.input(|i| {
                        i.pointer
                            .button_double_clicked(egui::PointerButton::Primary)
                    })
                {
                    self.portal_zoom_request = Some(target_ctx_id);
                }
            }
        }

        if self.ctrl_held {
            let c = self.colors.text_primary;
            crate::ui::snap::text_snapped(
                ui.painter(),
                pane_rect.center(),
                egui::Align2::CENTER_CENTER,
                pane_id.to_string(),
                egui::FontId::proportional(style::TEXT_PANE_ID_GHOST),
                egui::Color32::from_rgba_unmultiplied(
                    c.r(),
                    c.g(),
                    c.b(),
                    style::PANE_ID_GHOST_ALPHA,
                ),
            );
        }

        UiResponse::None
    }

    fn tab_title_for_pane(&mut self, pane: &PaneId) -> egui::WidgetText {
        let is_hidden = self.panes.get(pane).is_some_and(|p| p.is_hidden());
        let explicit_name = self.pane_names.get(pane).cloned();
        let label = match (is_hidden, explicit_name) {
            (true, Some(name)) => format!("{name} (hidden)"),
            (true, None) => "hidden".to_string(),
            (false, Some(name)) => name,
            (false, None) => format!("Terminal {}", pane + 1),
        };
        let color = if is_hidden {
            crate::ui::sidebar_row::with_alpha(self.colors.text_dim, 0.5)
        } else {
            self.colors.text_dim
        };
        egui::RichText::new(label).size(11.0).color(color).into()
    }

    fn simplification_options(&self) -> SimplificationOptions {
        SimplificationOptions {
            all_panes_must_have_tabs: true,
            ..SimplificationOptions::default()
        }
    }

    fn tab_ui(
        &mut self,
        _tiles: &mut Tiles<PaneId>,
        ui: &mut egui::Ui,
        id: egui::Id,
        _tile_id: TileId,
        _state: &TabState,
    ) -> egui::Response {
        // During zoom, suppress all tab label rendering so they don't bleed
        // through the semi-transparent scrim over background panes.
        let (_, rect) = ui.allocate_space(egui::Vec2::ZERO);
        ui.interact(rect, id, egui::Sense::hover())
    }

    fn tab_bar_height(&self, _style: &egui::Style) -> f32 {
        0.0
    }

    fn gap_width(&self, _style: &egui::Style) -> f32 {
        self.pane_gap
    }

    fn resize_stroke(&self, _style: &egui::Style, resize_state: ResizeState) -> egui::Stroke {
        if self.zoomed_pane.is_some() {
            return egui::Stroke::NONE;
        }
        match resize_state {
            ResizeState::Idle => egui::Stroke::NONE,
            ResizeState::Hovering | ResizeState::Dragging => {
                egui::Stroke::new(2.0_f32, self.colors.text_primary)
            }
        }
    }

    fn paint_on_top_of_tile(
        &self,
        painter: &egui::Painter,
        _style: &egui::Style,
        tile_id: TileId,
        rect: egui::Rect,
    ) {
        // Focus outline is painted after tree.ui() in app/mod.rs using the parent
        // painter (full window clip rect), so StrokeKind::Outside fills the inter-pane gap.
        let _ = (painter, tile_id, rect);
    }
}

/// Write any files the user just dropped into the terminal, quoting paths
/// that contain shell-significant characters.
pub(crate) fn write_dropped_paths_to_terminal(ui: &egui::Ui, t: &mut TerminalPane) {
    let dropped = ui.input(|i| i.raw.dropped_files.clone());
    for file in dropped {
        let Some(path) = &file.path else { continue };
        let path_str = path.display().to_string();
        log::info!("drop: writing path to terminal: {path_str}");
        let escaped =
            if path_str.contains(|c: char| c.is_whitespace() || "\"'\\()&|;$`!#".contains(c)) {
                format!("'{}'", path_str.replace('\'', "'\\''"))
            } else {
                path_str
            };
        t.backend
            .process_command(BackendCommand::Write(escaped.as_bytes().to_vec()));
        log::info!("drop: path written ok");
    }
}

pub(crate) fn deliver_dropped_files_to_app(
    ui: &egui::Ui,
    pane_id: PaneId,
    app: &mut crate::host::pane::AppPane,
) {
    for file in ui.input(|i| i.raw.dropped_files.clone()) {
        let Some(path) = file.path else { continue };
        let source = path.to_string_lossy();
        match dispatch_drop_to_app(pane_id, app, &source, true) {
            Ok(_) => log::info!("drop: delivery accepted pane_id={pane_id} source_kind=file"),
            Err(error) => log::info!("drop: delivery rejected pane_id={pane_id} reason={error}"),
        }
    }
}

pub(crate) fn dispatch_drop_to_app(
    pane_id: PaneId,
    app: &mut crate::host::pane::AppPane,
    source: &str,
    pane_hovered: bool,
) -> Result<serde_json::Value, String> {
    if !pane_hovered || app.hidden {
        return Err(format!(
            "pane {pane_id} is not an eligible hovered drop target"
        ));
    }
    app.runtime.drop_file(source)
}

// ── Portal mini-map ───────────────────────────────────────────────────────────

/// Compute normalized [0,1]×[0,1] rects for each leaf pane in a tile tree,
/// paired with each leaf's `TileId` so callers can look up pane type and focus.
pub(crate) fn compute_minimap_rects(
    tiles: &egui_tiles::Tiles<PaneId>,
    root: egui_tiles::TileId,
) -> Vec<(egui::Rect, egui_tiles::TileId)> {
    let full = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    let mut out = Vec::new();
    collect_tile_rects(tiles, root, full, &mut out);
    out
}

fn collect_tile_rects(
    tiles: &egui_tiles::Tiles<PaneId>,
    tile_id: egui_tiles::TileId,
    rect: egui::Rect,
    out: &mut Vec<(egui::Rect, egui_tiles::TileId)>,
) {
    match tiles.get(tile_id) {
        Some(egui_tiles::Tile::Pane(_)) => out.push((rect, tile_id)),
        Some(egui_tiles::Tile::Container(container)) => match container {
            egui_tiles::Container::Linear(linear) => {
                let is_h = linear.dir == egui_tiles::LinearDir::Horizontal;
                let total = if is_h { rect.width() } else { rect.height() };
                let sizes = linear.shares.split(&linear.children, total);
                let mut offset = if is_h { rect.min.x } else { rect.min.y };
                for (&child_id, &size) in linear.children.iter().zip(&sizes) {
                    let child_rect = if is_h {
                        egui::Rect::from_min_max(
                            egui::pos2(offset, rect.min.y),
                            egui::pos2(offset + size, rect.max.y),
                        )
                    } else {
                        egui::Rect::from_min_max(
                            egui::pos2(rect.min.x, offset),
                            egui::pos2(rect.max.x, offset + size),
                        )
                    };
                    collect_tile_rects(tiles, child_id, child_rect, out);
                    offset += size;
                }
            }
            egui_tiles::Container::Tabs(tabs) => {
                let child = tabs.active.or_else(|| tabs.children.first().copied());
                if let Some(child_id) = child {
                    collect_tile_rects(tiles, child_id, rect, out);
                }
            }
            egui_tiles::Container::Grid(grid) => {
                let children: Vec<egui_tiles::TileId> = grid.children().copied().collect();
                let n = children.len();
                if n > 0 {
                    let cols = ((n as f32).sqrt().ceil() as usize).max(1);
                    let rows = n.div_ceil(cols);
                    let w = rect.width() / cols as f32;
                    let h = rect.height() / rows as f32;
                    for (i, child_id) in children.iter().enumerate() {
                        let col = i % cols;
                        let row = i / cols;
                        let child_rect = egui::Rect::from_min_max(
                            egui::pos2(rect.min.x + col as f32 * w, rect.min.y + row as f32 * h),
                            egui::pos2(
                                rect.min.x + (col + 1) as f32 * w,
                                rect.min.y + (row + 1) as f32 * h,
                            ),
                        );
                        collect_tile_rects(tiles, *child_id, child_rect, out);
                    }
                }
            }
        },
        None => {}
    }
}

/// Collect leaf pane IDs by walking the tile tree in spatial order
/// (left-to-right for horizontal splits, top-to-bottom for vertical).
pub(crate) fn collect_pane_ids_spatial(
    tiles: &egui_tiles::Tiles<PaneId>,
    root: egui_tiles::TileId,
) -> Vec<PaneId> {
    let mut out = Vec::new();
    collect_pane_ids_recursive(tiles, root, &mut out);
    out
}

fn collect_pane_ids_recursive(
    tiles: &egui_tiles::Tiles<PaneId>,
    tile_id: egui_tiles::TileId,
    out: &mut Vec<PaneId>,
) {
    match tiles.get(tile_id) {
        Some(egui_tiles::Tile::Pane(pid)) => out.push(*pid),
        Some(egui_tiles::Tile::Container(container)) => match container {
            egui_tiles::Container::Linear(linear) => {
                for &child_id in &linear.children {
                    collect_pane_ids_recursive(tiles, child_id, out);
                }
            }
            egui_tiles::Container::Tabs(tabs) => {
                if let Some(active) = tabs.active {
                    collect_pane_ids_recursive(tiles, active, out);
                } else if let Some(&first) = tabs.children.first() {
                    collect_pane_ids_recursive(tiles, first, out);
                }
            }
            egui_tiles::Container::Grid(grid) => {
                for child_id in grid.children() {
                    collect_pane_ids_recursive(tiles, *child_id, out);
                }
            }
        },
        None => {}
    }
}

// ── Portal x-ray minimap ─────────────────────────────────────────────────────

/// Render the x-ray minimap for a portal tile.
///
/// Windows are laid out in a grid matching their grid_x/grid_y positions.
/// Each window gets a "monitor bezel" frame with a thin chrome bar, then
/// its pane layout fills the body below.
pub(crate) fn paint_portal_minimap(
    painter: &egui::Painter,
    area: egui::Rect,
    windows: &[MiniWindow],
    colors: &Colors,
    time: f64,
) {
    if windows.is_empty() {
        return;
    }

    // Determine grid extents so we know how many columns/rows to allocate.
    let max_gx = windows.iter().map(|w| w.grid_x).max().unwrap_or(0);
    let max_gy = windows.iter().map(|w| w.grid_y).max().unwrap_or(0);
    let cols = (max_gx + 1) as usize;
    let rows = (max_gy + 1) as usize;

    const WIN_GAP: f32 = 10.0;
    const WIN_RADIUS: f32 = 4.0;
    const PANE_GAP: f32 = 3.0;

    let cell_w = (area.width() - WIN_GAP * (cols as f32 - 1.0)) / cols as f32;
    let cell_h = (area.height() - WIN_GAP * (rows as f32 - 1.0)) / rows as f32;

    for win in windows {
        let col = win.grid_x as usize;
        let row = win.grid_y as usize;
        let win_x = area.min.x + col as f32 * (cell_w + WIN_GAP);
        let win_y = area.min.y + row as f32 * (cell_h + WIN_GAP);
        let win_rect =
            egui::Rect::from_min_size(egui::pos2(win_x, win_y), egui::vec2(cell_w, cell_h));

        // Window frame background
        let frame_bg = egui::Color32::from_rgba_unmultiplied(
            colors.terminal_bg.r(),
            colors.terminal_bg.g(),
            colors.terminal_bg.b(),
            153,
        );
        painter.rect_filled(win_rect, WIN_RADIUS, frame_bg);

        // Window frame border
        let frame_border = egui::Color32::from_rgba_unmultiplied(
            colors.border.r(),
            colors.border.g(),
            colors.border.b(),
            77,
        );
        painter.rect_stroke(
            win_rect,
            WIN_RADIUS,
            egui::Stroke::new(1.0_f32, frame_border),
            egui::StrokeKind::Outside,
        );

        let body_rect = win_rect;

        // Render panes inside the body
        for pane in win.panes.iter() {
            let pane_rect = egui::Rect::from_min_max(
                egui::pos2(
                    body_rect.min.x + pane.norm_rect.min.x * body_rect.width(),
                    body_rect.min.y + pane.norm_rect.min.y * body_rect.height(),
                ),
                egui::pos2(
                    body_rect.min.x + pane.norm_rect.max.x * body_rect.width(),
                    body_rect.min.y + pane.norm_rect.max.y * body_rect.height(),
                ),
            );
            let cell = pane_rect.shrink(PANE_GAP);
            if cell.width() < 2.0 || cell.height() < 2.0 {
                continue;
            }

            // Pane background — solid screen color
            painter.rect_filled(cell, 2.0, colors.bg_darkest);

            // Inactive (exited) panes get a dim overlay
            if !pane.active {
                let dim_overlay = egui::Color32::from_rgba_unmultiplied(0, 0, 0, 60);
                painter.rect_filled(cell, 2.0, dim_overlay);
            }

            // Focused pane: accent tint + glow edge
            if pane.focused {
                let tint = egui::Color32::from_rgba_unmultiplied(
                    colors.accent.r(),
                    colors.accent.g(),
                    colors.accent.b(),
                    20,
                );
                painter.rect_filled(cell, 2.0, tint);

                // Bottom-edge glow line
                let glow_y = cell.max.y - 1.0;
                let glow_color = egui::Color32::from_rgba_unmultiplied(
                    colors.accent.r(),
                    colors.accent.g(),
                    colors.accent.b(),
                    50,
                );
                painter.line_segment(
                    [
                        egui::pos2(cell.min.x, glow_y),
                        egui::pos2(cell.max.x, glow_y),
                    ],
                    egui::Stroke::new(1.5_f32, glow_color),
                );

                // Focused border
                let accent_border = egui::Color32::from_rgba_unmultiplied(
                    colors.accent.r(),
                    colors.accent.g(),
                    colors.accent.b(),
                    70,
                );
                painter.rect_stroke(
                    cell,
                    2.0,
                    egui::Stroke::new(1.0_f32, accent_border),
                    egui::StrokeKind::Middle,
                );
            } else {
                let dim_border = egui::Color32::from_rgba_unmultiplied(
                    colors.border.r(),
                    colors.border.g(),
                    colors.border.b(),
                    77,
                );
                painter.rect_stroke(
                    cell,
                    2.0,
                    egui::Stroke::new(1.0_f32, dim_border),
                    egui::StrokeKind::Middle,
                );
            }

            // Activity dot — top-left of the pane, mirroring the title-bar
            // dot on real panes. Vertically centered on the title line so it
            // reads as part of the title row, with matching left padding.
            const ACTIVITY_DOT_R: f32 = crate::ui::sidebar_row::PANE_DOT_RADIUS;
            const ACTIVITY_DOT_PAD: f32 = 5.0;
            let title_font_size: f32 = if cell.width() > 80.0 { 11.0 } else { 9.0 };
            let activity_dot = pane.activity.as_ref().filter(|_| cell.width() > 14.0);
            if let Some(state) = activity_dot {
                let color = crate::ui::activity::dot_color_from_time(state, colors, time, 0);
                painter.circle_filled(
                    egui::pos2(
                        cell.min.x + ACTIVITY_DOT_PAD + ACTIVITY_DOT_R,
                        cell.min.y + 2.0 + title_font_size * 0.5,
                    ),
                    ACTIVITY_DOT_R,
                    color,
                );
            }

            if !pane.has_content {
                continue;
            }
            let content_area = cell.shrink(2.0);
            if content_area.width() < 4.0 || content_area.height() < 4.0 {
                continue;
            }

            // Standardized icon alpha: 200 focused / 60 unfocused, dimmed to 30% for inactive panes.
            let icon_alpha: u8 = if pane.focused { 200 } else { 100 };
            let icon_alpha: u8 = if !pane.active {
                (icon_alpha as f32 * 0.3) as u8
            } else {
                icon_alpha
            };

            // Title label (OSC title or app name) at top of pane
            let mut content_top = content_area.min.y;
            if let Some(ref title) = pane.title {
                if cell.width() > 25.0 && cell.height() > 14.0 {
                    let title_alpha: u8 = if pane.focused { 130 } else { 80 };
                    let title_color = egui::Color32::from_rgba_unmultiplied(
                        colors.text_dim.r(),
                        colors.text_dim.g(),
                        colors.text_dim.b(),
                        title_alpha,
                    );
                    let font_size = title_font_size;
                    // Title clears the dot (pad + diameter + gap), measured
                    // from the cell edge like the dot itself.
                    let title_x_offset = if activity_dot.is_some() {
                        ACTIVITY_DOT_PAD + ACTIVITY_DOT_R * 2.0 + 4.0 - 2.0
                    } else {
                        1.0
                    };
                    crate::ui::snap::text_snapped(
                        painter,
                        egui::pos2(content_area.min.x + title_x_offset, content_area.min.y),
                        egui::Align2::LEFT_TOP,
                        title,
                        egui::FontId::proportional(font_size),
                        title_color,
                    );
                    content_top += font_size + 2.0;
                }
            }
            let draw_area = egui::Rect::from_min_max(
                egui::pos2(content_area.min.x, content_top),
                content_area.max,
            );
            if draw_area.height() < 4.0 {
                continue;
            }

            match pane.kind {
                PaneKind::App => {
                    let block_color = egui::Color32::from_rgba_unmultiplied(
                        colors.text_dim.r(),
                        colors.text_dim.g(),
                        colors.text_dim.b(),
                        icon_alpha / 2,
                    );
                    let accent_dim = egui::Color32::from_rgba_unmultiplied(
                        colors.accent.r(),
                        colors.accent.g(),
                        colors.accent.b(),
                        icon_alpha,
                    );

                    // Centered grid of small squares (widget-like feel)
                    let grid_size =
                        (draw_area.width().min(draw_area.height()) * 0.75).clamp(8.0, 48.0);
                    let cols = ((grid_size / 6.0) as usize).clamp(2, 4);
                    let rows = cols;
                    let sq = (grid_size / cols as f32).floor();
                    let gap = (sq * 0.25).clamp(1.0, 2.0);
                    let total_w = cols as f32 * sq + (cols - 1) as f32 * gap;
                    let total_h = rows as f32 * sq + (rows - 1) as f32 * gap;
                    let ox = draw_area.center().x - total_w * 0.5;
                    let oy = draw_area.center().y - total_h * 0.5;

                    for r in 0..rows {
                        for c in 0..cols {
                            let x = ox + c as f32 * (sq + gap);
                            let y = oy + r as f32 * (sq + gap);
                            let fill = if (r + c) % 3 == 0 {
                                accent_dim
                            } else {
                                block_color
                            };
                            painter.rect_filled(
                                egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(sq, sq)),
                                1.0,
                                fill,
                            );
                        }
                    }
                }
                PaneKind::TextEditor => {
                    // Portrait sheet with a folded top-right corner; the text
                    // rules carry the accent color so the glyph reads as a
                    // document at a glance.
                    let glyph = (draw_area.width().min(draw_area.height())).clamp(12.0, 60.0);
                    let pw = glyph * 0.58;
                    let ph = glyph * 0.74;
                    let center = draw_area.center();
                    let paper = egui::Rect::from_center_size(center, egui::vec2(pw, ph));
                    let fold = (pw * 0.30).clamp(2.5, 12.0);

                    let outline = egui::Color32::from_rgba_unmultiplied(
                        colors.text_dim.r(),
                        colors.text_dim.g(),
                        colors.text_dim.b(),
                        (icon_alpha as u32 * 3 / 2).min(255) as u8,
                    );
                    let fill = egui::Color32::from_rgba_unmultiplied(
                        colors.text_dim.r(),
                        colors.text_dim.g(),
                        colors.text_dim.b(),
                        (icon_alpha / 4).max(8),
                    );
                    // Accent text rules — the defining cue of the document glyph.
                    let rule = egui::Color32::from_rgba_unmultiplied(
                        colors.accent.r(),
                        colors.accent.g(),
                        colors.accent.b(),
                        icon_alpha.saturating_add(60),
                    );

                    // Sheet body (folded corner masked out via two polygons).
                    let body = vec![
                        paper.left_top(),
                        egui::pos2(paper.right() - fold, paper.top()),
                        egui::pos2(paper.right(), paper.top() + fold),
                        paper.right_bottom(),
                        paper.left_bottom(),
                    ];
                    painter.add(egui::Shape::convex_polygon(
                        body,
                        fill,
                        egui::Stroke::new(1.0_f32, outline),
                    ));
                    // Folded corner triangle, a touch lighter than the body.
                    painter.add(egui::Shape::convex_polygon(
                        vec![
                            egui::pos2(paper.right() - fold, paper.top()),
                            egui::pos2(paper.right() - fold, paper.top() + fold),
                            egui::pos2(paper.right(), paper.top() + fold),
                        ],
                        egui::Color32::from_rgba_unmultiplied(
                            colors.text_dim.r(),
                            colors.text_dim.g(),
                            colors.text_dim.b(),
                            icon_alpha / 2,
                        ),
                        egui::Stroke::new(1.0_f32, outline),
                    ));

                    // Text rules — drop the last when the glyph is tiny.
                    let rule_w = (pw * 0.13).clamp(1.0, 2.0);
                    let margin = pw * 0.20;
                    let rule_xs = (paper.left() + margin, paper.right() - margin);
                    let rows = if ph > 22.0 { 3 } else { 2 };
                    for i in 0..rows {
                        let ry = paper.top() + ph * (0.36 + i as f32 * 0.18);
                        // Last visible rule is shorter, like a paragraph end.
                        let right = if i == rows - 1 {
                            rule_xs.0 + (rule_xs.1 - rule_xs.0) * 0.6
                        } else {
                            rule_xs.1
                        };
                        painter.line_segment(
                            [egui::pos2(rule_xs.0, ry), egui::pos2(right, ry)],
                            egui::Stroke::new(rule_w, rule),
                        );
                    }
                }
                PaneKind::Portal => {
                    // 2x2 window grid, outlines only, bottom-right filled with accent (Plexi logo feel)
                    let grid_size =
                        (draw_area.width().min(draw_area.height()) * 0.7).clamp(10.0, 48.0);
                    let mini_gap = (grid_size * 0.1).clamp(1.5, 3.0);
                    let mini_w = (grid_size - mini_gap) / 2.0;
                    let mini_h = (grid_size - mini_gap) / 2.0;
                    let ox = draw_area.center().x - grid_size * 0.5;
                    let oy = draw_area.center().y - grid_size * 0.5;

                    let outline_color = egui::Color32::from_rgba_unmultiplied(
                        colors.text_dim.r(),
                        colors.text_dim.g(),
                        colors.text_dim.b(),
                        icon_alpha,
                    );
                    let accent_fill = egui::Color32::from_rgba_unmultiplied(
                        colors.accent.r(),
                        colors.accent.g(),
                        colors.accent.b(),
                        icon_alpha / 2,
                    );

                    for r in 0..2u32 {
                        for c in 0..2u32 {
                            let x = ox + c as f32 * (mini_w + mini_gap);
                            let y = oy + r as f32 * (mini_h + mini_gap);
                            let rect = egui::Rect::from_min_size(
                                egui::pos2(x, y),
                                egui::vec2(mini_w, mini_h),
                            );
                            if r == 1 && c == 1 {
                                painter.rect_filled(rect, 2.0, accent_fill);
                            } else {
                                painter.rect_stroke(
                                    rect,
                                    2.0,
                                    egui::Stroke::new(1.0_f32, outline_color),
                                    egui::StrokeKind::Inside,
                                );
                            }
                        }
                    }
                }
                PaneKind::Terminal => {
                    // Centered ">_" prompt symbol
                    let font_size =
                        (draw_area.width().min(draw_area.height()) * 0.70).clamp(10.0, 36.0);
                    let prompt_color = egui::Color32::from_rgba_unmultiplied(
                        colors.text_primary.r(),
                        colors.text_primary.g(),
                        colors.text_primary.b(),
                        icon_alpha,
                    );
                    crate::ui::snap::text_snapped(
                        painter,
                        draw_area.center(),
                        egui::Align2::CENTER_CENTER,
                        ">_",
                        egui::FontId::monospace(font_size),
                        prompt_color,
                    );
                }
            }
        }
    }
}
