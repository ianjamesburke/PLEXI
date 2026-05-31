use crate::pane::{Pane, TerminalPane};
use crate::render;
use crate::style;
use crate::theme::Colors;
use egui::Color32;
use egui_term::{BackendCommand, TerminalTheme};
use egui_tiles::{Behavior, ResizeState, SimplificationOptions, TabState, TileId, Tiles, UiResponse};
use std::collections::HashMap;
use std::path::PathBuf;

pub type PaneId = u64;

pub(crate) const DOT_RADIUS: f32 = 4.0;
const DOT_SPACING: f32 = 12.0;
const DOT_LEFT_MARGIN: f32 = 6.0;
pub(crate) const TAB_DOT_RESERVED_HEIGHT: f32 = 14.0;

pub(crate) fn paint_tab_dots(
    painter: &egui::Painter,
    left_x: f32,
    center_y: f32,
    active_idx: usize,
    count: usize,
    active_color: Color32,
    inactive_color: Color32,
) {
    let start_x = left_x + DOT_LEFT_MARGIN;
    for i in 0..count {
        let cx = start_x + (i as f32) * DOT_SPACING + DOT_RADIUS;
        let color = if i == active_idx {
            active_color
        } else {
            inactive_color
        };
        painter.circle_filled(egui::pos2(cx, center_y), DOT_RADIUS, color);
    }
}

/// Preview data for a Portal tile.
#[derive(Clone)]
pub struct PortalPreview {
    pub context_name: String,
    pub context_description: String,
    pub pane_count: usize,
    pub notification_count: usize,
    /// Normalized [0,1]×[0,1] rects for each leaf pane in the child window layout.
    pub minimap_rects: Vec<egui::Rect>,
}

impl Default for PortalPreview {
    fn default() -> Self {
        PortalPreview {
            context_name: "(deleted)".to_string(),
            context_description: String::new(),
            pane_count: 0,
            notification_count: 0,
            minimap_rects: Vec::new(),
        }
    }
}

pub struct PlexiBehavior<'a> {
    pub panes: &'a mut HashMap<PaneId, Pane>,
    pub focused_tile: Option<TileId>,
    pub theme: TerminalTheme,
    pub new_focused: Option<TileId>,
    pub close_exited: Option<TileId>,
    pub tab_info: HashMap<TileId, (usize, usize)>, // tile_id -> (index, count)
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
    /// Opacity applied to unfocused panes when ghost mode is active.
    /// `None` = no dimming. Values below 1.0 dim all non-focused panes.
    pub unfocused_opacity: Option<f32>,
    /// Preview data for Portal tiles.
    pub portal_info: HashMap<PaneId, PortalPreview>,
    /// True when an overlay or modal has captured keyboard input this frame.
    /// Prevents terminal panes from calling `request_focus()` and stealing
    /// egui focus from the active overlay (egui resolves focus last-caller-wins).
    pub modal_open: bool,
    /// True when the Control modifier is held — triggers the pane ID ghost overlay.
    pub ctrl_held: bool,
}

impl Behavior<PaneId> for PlexiBehavior<'_> {
    fn pane_ui(&mut self, ui: &mut egui::Ui, tile_id: TileId, pane_id: &mut PaneId) -> UiResponse {
        // While any pane is zoomed, paint background panes as dark placeholders
        // and skip all input detection — the zoom overlay owns focus and drop
        // handling. This avoids per-pane `ui.input()` calls during hover, which
        // were O(n) and ran even when the results could never be acted on.
        if self.zoomed_pane.is_some() {
            let pane_rect = ui.available_rect_before_wrap();
            ui.painter().rect_filled(pane_rect, 0.0, self.colors.bg_darkest);
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

        let is_focused = self.focused_tile == Some(tile_id) && !self.modal_open;

        if !is_focused {
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
            }
        }

        let pane_rect = ui.available_rect_before_wrap();
        let Some(pane) = self.panes.get_mut(pane_id) else {
            return UiResponse::None;
        };

        if let Some(app_pane) = pane.as_app_mut() {
            ui.painter().rect_filled(pane_rect, 0.0, self.colors.bg_darkest);
            let mut app_ui = ui.new_child(egui::UiBuilder::new().max_rect(pane_rect));
            render::app_pane::render(&mut app_ui, app_pane, &self.colors, is_focused);
        } else if let Some(terminal) = pane.as_terminal_mut() {
            ui.painter().rect_filled(pane_rect, 0.0, self.colors.terminal_bg);
            let mut terminal_ui =
                ui.new_child(egui::UiBuilder::new().max_rect(pane_rect));
            let close_exited = render::terminal_pane::render(
                &mut terminal_ui,
                terminal,
                tile_id,
                pane_id,
                is_focused,
                &self.theme,
                &self.colors,
                &self.pane_names,
                &self.tab_info,
                self.workspace_root.as_deref(),
            );
            if close_exited {
                self.close_exited = Some(tile_id);
            }
        } else if pane.as_portal().is_some() {
            // Portal tile — direct egui rendering.
            ui.painter().rect_filled(pane_rect, 0.0, self.colors.bg_darkest);
            let preview = self.portal_info.get(pane_id).cloned().unwrap_or_default();
            let osc_status: &'static str = pane.as_portal()
                .and_then(|p| p.context_state.as_ref())
                .map(|s| match s.status {
                    crate::context_state::ContextStatus::Working => "busy",
                    crate::context_state::ContextStatus::Error => "error",
                    _ => "idle",
                })
                .unwrap_or("idle");
            log::info!(
                "portal render: ctx={:?} status={} panes={}",
                preview.context_name, osc_status, preview.pane_count,
            );

            let padding = style::SPACE_MD;
            let inner = pane_rect.shrink(padding);
            let mut portal_ui = ui.new_child(egui::UiBuilder::new().max_rect(inner));
            portal_ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                // Context name
                ui.label(
                    egui::RichText::new(&preview.context_name)
                        .size(style::TEXT_BODY)
                        .strong()
                        .color(self.colors.text_primary),
                );
                // Description (if set)
                if !preview.context_description.is_empty() {
                    ui.scope(|ui| {
                        ui.set_max_width(inner.width());
                        crate::widgets::description_label(ui, &preview.context_description, &self.colors);
                    });
                }
                // Status chip + pane count + notification count
                ui.horizontal(|ui| {
                    crate::widgets::status_chip(ui, osc_status, &self.colors);
                    let count_label = if preview.notification_count > 0 {
                        format!("{} panes · {} notifs", preview.pane_count, preview.notification_count)
                    } else {
                        format!("{} panes", preview.pane_count)
                    };
                    ui.label(
                        egui::RichText::new(count_label)
                            .size(style::TEXT_HINT)
                            .color(self.colors.text_dim),
                    );
                });
                ui.add_space(style::SPACE_SM);
                // Mini-map: proportional rects for each leaf pane
                if !preview.minimap_rects.is_empty() {
                    let map_h = (inner.height() * 0.35).clamp(24.0, 80.0);
                    let (map_area, _) = ui.allocate_exact_size(
                        egui::vec2(inner.width(), map_h),
                        egui::Sense::hover(),
                    );
                    let origin = map_area.min;
                    let map_size = map_area.size();
                    let stroke = egui::Stroke::new(1.0, self.colors.text_dim.linear_multiply(0.35));
                    for norm in &preview.minimap_rects {
                        let scaled = egui::Rect::from_min_max(
                            egui::pos2(origin.x + norm.min.x * map_size.x, origin.y + norm.min.y * map_size.y),
                            egui::pos2(origin.x + norm.max.x * map_size.x, origin.y + norm.max.y * map_size.y),
                        );
                        ui.painter().rect_stroke(scaled.shrink(1.0), 2.0, stroke, egui::StrokeKind::Middle);
                    }
                }
            });
            // Shortcut hint pinned to bottom-right
            ui.painter().text(
                egui::pos2(pane_rect.right() - padding, pane_rect.bottom() - padding),
                egui::Align2::RIGHT_BOTTOM,
                "\u{2318}\u{21e7}\u{21b5} zoom in",
                egui::FontId::proportional(style::TEXT_HINT),
                self.colors.text_dim.linear_multiply(0.5),
            );
        }

        if self.ctrl_held {
            let c = self.colors.text_primary;
            ui.painter().text(
                pane_rect.center(),
                egui::Align2::CENTER_CENTER,
                pane_id.to_string(),
                egui::FontId::proportional(style::TEXT_PANE_ID_GHOST),
                egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), style::PANE_ID_GHOST_ALPHA),
            );
        }

        UiResponse::None
    }

    fn tab_title_for_pane(&mut self, pane: &PaneId) -> egui::WidgetText {
        let label = if let Some(name) = self.pane_names.get(pane) {
            name.clone()
        } else {
            format!("Terminal {}", pane + 1)
        };
        egui::RichText::new(label)
            .size(11.0)
            .color(self.colors.text_dim)
            .into()
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
        4.0
    }

    fn resize_stroke(&self, _style: &egui::Style, resize_state: ResizeState) -> egui::Stroke {
        if self.zoomed_pane.is_some() {
            return egui::Stroke::NONE;
        }
        match resize_state {
            ResizeState::Idle => egui::Stroke::NONE,
            ResizeState::Hovering | ResizeState::Dragging => {
                egui::Stroke::new(2.0, self.colors.text_primary)
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
        let escaped = if path_str.contains(|c: char| {
            c.is_whitespace() || "\"'\\()&|;$`!#".contains(c)
        }) {
            format!("'{}'", path_str.replace('\'', "'\\''"))
        } else {
            path_str
        };
        t.backend
            .process_command(BackendCommand::Write(escaped.as_bytes().to_vec()));
        log::info!("drop: path written ok");
    }
}

// ── Portal mini-map ───────────────────────────────────────────────────────────

/// Compute normalized [0,1]×[0,1] rects for each leaf pane in a tile tree.
/// Used by the Portal tile renderer to draw a proportional mini-map preview.
pub(crate) fn compute_minimap_rects(
    tiles: &egui_tiles::Tiles<PaneId>,
    root: egui_tiles::TileId,
) -> Vec<egui::Rect> {
    let full = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    let mut out = Vec::new();
    collect_tile_rects(tiles, root, full, &mut out);
    out
}

fn collect_tile_rects(
    tiles: &egui_tiles::Tiles<PaneId>,
    tile_id: egui_tiles::TileId,
    rect: egui::Rect,
    out: &mut Vec<egui::Rect>,
) {
    match tiles.get(tile_id) {
        Some(egui_tiles::Tile::Pane(_)) => out.push(rect),
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
                    let rows = (n + cols - 1) / cols;
                    let w = rect.width() / cols as f32;
                    let h = rect.height() / rows as f32;
                    for (i, child_id) in children.iter().enumerate() {
                        let col = i % cols;
                        let row = i / cols;
                        let child_rect = egui::Rect::from_min_max(
                            egui::pos2(rect.min.x + col as f32 * w, rect.min.y + row as f32 * h),
                            egui::pos2(rect.min.x + (col + 1) as f32 * w, rect.min.y + (row + 1) as f32 * h),
                        );
                        collect_tile_rects(tiles, *child_id, child_rect, out);
                    }
                }
            }
        },
        None => {}
    }
}

