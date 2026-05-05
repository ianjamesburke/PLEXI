use crate::pane::{Pane, TerminalPane};
use crate::render;
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

        let is_focused = self.focused_tile == Some(tile_id);

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
        ui.painter().rect_filled(pane_rect, 0.0, self.colors.terminal_bg);
        let mut inner_ui = ui.new_child(egui::UiBuilder::new().max_rect(pane_rect.shrink(8.0)));
        let ui = &mut inner_ui;

        if let Some(app_pane) = pane.as_app_mut() {
            render::app_pane::render(ui, app_pane, &self.colors, is_focused);
        } else if let Some(terminal) = pane.as_terminal_mut() {
            let close_exited = render::terminal_pane::render(
                ui,
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
