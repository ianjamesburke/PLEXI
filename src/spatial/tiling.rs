use crate::host::pane::{Pane, TerminalPane};
use crate::render;
use crate::ui::style;
use crate::ui::theme::Colors;
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

/// What kind of pane occupies a minimap slot.
#[derive(Clone, Copy, PartialEq)]
pub enum PaneKind {
    Terminal,
    App,
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
    /// Set by double-click on a Portal pane — the target context_id to zoom into.
    pub portal_zoom_request: Option<u64>,
    /// Resolved inter-pane gap width (from config, default 4.0).
    pub pane_gap: f32,
    /// Resolved pane title bar font size (from config, default 11.0).
    pub pane_title_font_size: f32,
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

        let is_hidden = self.panes.get(pane_id).map_or(false, |p| p.is_hidden());

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
                self.pane_title_font_size,
            );
            if close_exited {
                self.close_exited = Some(tile_id);
            }
        } else if pane.as_portal().is_some() {
            // Portal tile — direct egui rendering.
            ui.painter().rect_filled(pane_rect, 0.0, self.colors.bg_darkest);
            let preview = self.portal_info.get(pane_id).cloned().unwrap_or_default();
            let padding = style::SPACE_MD;
            let inner = pane_rect.shrink(padding);
            let mut portal_ui = ui.new_child(egui::UiBuilder::new().max_rect(inner));
            portal_ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                ui.label(
                    egui::RichText::new(&preview.context_name)
                        .size(style::TEXT_BODY)
                        .strong()
                        .color(self.colors.text_primary),
                );
                if !preview.context_description.is_empty() {
                    ui.scope(|ui| {
                        ui.set_max_width(inner.width());
                        crate::ui::widgets::description_label(ui, &preview.context_description, &self.colors);
                    });
                }
                {
                    let count_label = if preview.window_count > 1 && preview.notification_count > 0 {
                        format!("{} panes \u{b7} {} windows \u{b7} {} notifs", preview.pane_count, preview.window_count, preview.notification_count)
                    } else if preview.window_count > 1 {
                        format!("{} panes \u{b7} {} windows", preview.pane_count, preview.window_count)
                    } else if preview.notification_count > 0 {
                        format!("{} panes \u{b7} {} notifs", preview.pane_count, preview.notification_count)
                    } else {
                        format!("{} panes", preview.pane_count)
                    };
                    ui.label(
                        egui::RichText::new(count_label)
                            .size(style::TEXT_HINT)
                            .color(self.colors.text_dim),
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
                    let painter = ui.painter();
                    let colors = self.colors.clone();
                    paint_portal_minimap(painter, map_area, &preview.windows, &colors);
                }
            });
            // Double-click on portal pane to zoom into the sub-context.
            if let Some(target_ctx_id) = self.panes.get(pane_id).and_then(|p| p.portal_target()) {
                if ui.rect_contains_pointer(pane_rect)
                    && ui.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary))
                {
                    self.portal_zoom_request = Some(target_ctx_id);
                }
            }
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
        let is_hidden = self.panes.get(pane).map_or(false, |p| p.is_hidden());
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
        egui::RichText::new(label)
            .size(11.0)
            .color(color)
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
        self.pane_gap
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
        let win_rect = egui::Rect::from_min_size(egui::pos2(win_x, win_y), egui::vec2(cell_w, cell_h));

        // Window frame background
        let frame_bg = egui::Color32::from_rgba_unmultiplied(
            colors.terminal_bg.r(), colors.terminal_bg.g(), colors.terminal_bg.b(), 153,
        );
        painter.rect_filled(win_rect, WIN_RADIUS, frame_bg);

        // Window frame border
        let frame_border = egui::Color32::from_rgba_unmultiplied(
            colors.border.r(), colors.border.g(), colors.border.b(), 77,
        );
        painter.rect_stroke(win_rect, WIN_RADIUS, egui::Stroke::new(1.0, frame_border), egui::StrokeKind::Outside);

        let body_rect = win_rect;

        // Render panes inside the body
        for (_pane_idx, pane) in win.panes.iter().enumerate() {
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
                    colors.accent.r(), colors.accent.g(), colors.accent.b(), 20,
                );
                painter.rect_filled(cell, 2.0, tint);

                // Bottom-edge glow line
                let glow_y = cell.max.y - 1.0;
                let glow_color = egui::Color32::from_rgba_unmultiplied(
                    colors.accent.r(), colors.accent.g(), colors.accent.b(), 50,
                );
                painter.line_segment(
                    [egui::pos2(cell.min.x, glow_y), egui::pos2(cell.max.x, glow_y)],
                    egui::Stroke::new(1.5, glow_color),
                );

                // Focused border
                let accent_border = egui::Color32::from_rgba_unmultiplied(
                    colors.accent.r(), colors.accent.g(), colors.accent.b(), 70,
                );
                painter.rect_stroke(cell, 2.0, egui::Stroke::new(1.0, accent_border), egui::StrokeKind::Middle);
            } else {
                let dim_border = egui::Color32::from_rgba_unmultiplied(
                    colors.border.r(), colors.border.g(), colors.border.b(), 77,
                );
                painter.rect_stroke(cell, 2.0, egui::Stroke::new(1.0, dim_border), egui::StrokeKind::Middle);
            }

            if !pane.has_content {
                continue;
            }
            let content_area = cell.shrink(2.0);
            if content_area.width() < 4.0 || content_area.height() < 4.0 {
                continue;
            }

            let line_alpha: u8 = if pane.focused { 51 } else { 25 };
            let line_alpha: u8 = if !pane.active { (line_alpha as f32 * 0.3) as u8 } else { line_alpha };

            // Title label (OSC title or app name) at top of pane
            let mut content_top = content_area.min.y;
            if let Some(ref title) = pane.title {
                if cell.width() > 25.0 && cell.height() > 14.0 {
                    let title_alpha: u8 = if pane.focused { 102 } else { 51 };
                    let title_color = egui::Color32::from_rgba_unmultiplied(
                        colors.text_dim.r(), colors.text_dim.g(), colors.text_dim.b(), title_alpha,
                    );
                    let font_size = if cell.width() > 80.0 { 11.0 } else { 9.0 };
                    painter.text(
                        egui::pos2(content_area.min.x + 1.0, content_area.min.y),
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
                        colors.text_dim.r(), colors.text_dim.g(), colors.text_dim.b(), line_alpha / 2,
                    );
                    let accent_dim = egui::Color32::from_rgba_unmultiplied(
                        colors.accent.r(), colors.accent.g(), colors.accent.b(), line_alpha,
                    );

                    // Centered grid of small squares (widget-like feel)
                    let grid_size = (draw_area.width().min(draw_area.height()) * 0.75).clamp(8.0, 48.0);
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
                            let fill = if (r + c) % 3 == 0 { accent_dim } else { block_color };
                            painter.rect_filled(
                                egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(sq, sq)),
                                1.0,
                                fill,
                            );
                        }
                    }
                }
                PaneKind::Portal => {
                    // 2x2 window grid, outlines only, bottom-right filled with accent (Plexi logo feel)
                    let grid_size = (draw_area.width().min(draw_area.height()) * 0.7).clamp(10.0, 48.0);
                    let mini_gap = (grid_size * 0.1).clamp(1.5, 3.0);
                    let mini_w = (grid_size - mini_gap) / 2.0;
                    let mini_h = (grid_size - mini_gap) / 2.0;
                    let ox = draw_area.center().x - grid_size * 0.5;
                    let oy = draw_area.center().y - grid_size * 0.5;

                    let outline_color = egui::Color32::from_rgba_unmultiplied(
                        colors.text_dim.r(), colors.text_dim.g(), colors.text_dim.b(), line_alpha * 2,
                    );
                    let accent_fill = egui::Color32::from_rgba_unmultiplied(
                        colors.accent.r(), colors.accent.g(), colors.accent.b(),
                        if pane.focused { 80 } else { 50 },
                    );

                    for r in 0..2u32 {
                        for c in 0..2u32 {
                            let x = ox + c as f32 * (mini_w + mini_gap);
                            let y = oy + r as f32 * (mini_h + mini_gap);
                            let rect = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(mini_w, mini_h));
                            if r == 1 && c == 1 {
                                painter.rect_filled(rect, 2.0, accent_fill);
                            } else {
                                painter.rect_stroke(rect, 2.0, egui::Stroke::new(1.0, outline_color), egui::StrokeKind::Inside);
                            }
                        }
                    }
                }
                PaneKind::Terminal => {
                    // Centered ">_" prompt symbol
                    let font_size = (draw_area.width().min(draw_area.height()) * 0.45).clamp(8.0, 24.0);
                    let prompt_alpha = if pane.focused { line_alpha * 2 } else { line_alpha };
                    let prompt_color = egui::Color32::from_rgba_unmultiplied(
                        colors.text_dim.r(), colors.text_dim.g(), colors.text_dim.b(), prompt_alpha,
                    );
                    painter.text(
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

