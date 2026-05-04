//! Spatial-grid minimap overlay.
//!
//! Renders a small top-right overlay showing the 2-D window grid for the
//! active workspace. Empty rows are collapsed so the grid is always compact.
//! The active window is highlighted; clicking a cell switches to it.

use crate::context::Window;
use crate::theme::Colors;

/// Runtime state for the minimap overlay.
pub struct MinimapState {
    /// Whether the minimap is shown at all. `Cmd+Shift+M` toggles this.
    pub visible: bool,
}

impl MinimapState {
    pub fn new() -> Self {
        Self { visible: false }
    }

    pub fn with_visible(visible: bool) -> Self {
        Self { visible }
    }

    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }
}

const CELL_W: f32 = 34.0;
const CELL_H: f32 = 26.0;
const CELL_GAP: f32 = 4.0;
const INSET_RIGHT: f32 = 18.0;
const INSET_TOP: f32 = 54.0;
const CORNER_RADIUS: f32 = 6.0;
const NAME_FONT_SIZE: f32 = 12.0;

pub fn render_minimap(
    ui: &mut egui::Ui,
    content_rect: egui::Rect,
    windows: &[Window],
    active_window: usize,
    last_visited: &std::collections::HashMap<u32, u32>,
    colors: &Colors,
    workspace_id: u64,
    workspace_name: &str,
) -> Option<usize> {
    // Only windows belonging to the current workspace.
    let visible: Vec<(usize, &Window)> = windows
        .iter()
        .enumerate()
        .filter(|(_, c)| c.context_id == workspace_id)
        .collect();

    if visible.is_empty() {
        return None;
    }

    ui.set_opacity(0.75);

    // ── Collapse empty rows so the grid is always compact ─────────────────
    let mut raw_ys: Vec<u32> = visible.iter().map(|(_, c)| c.grid_y).collect();
    raw_ys.sort_unstable();
    raw_ys.dedup();
    let row_remap: std::collections::HashMap<u32, u32> = raw_ys
        .iter()
        .enumerate()
        .map(|(new, &old)| (old, new as u32))
        .collect();

    let max_x = visible.iter().map(|(_, c)| c.grid_x).max().unwrap_or(0);
    let max_mapped_y = row_remap.values().copied().max().unwrap_or(0);
    let cols = max_x + 1;
    let rows = max_mapped_y + 1;

    let grid_pixel_w = cols as f32 * (CELL_W + CELL_GAP) - CELL_GAP;
    let grid_pixel_h = rows as f32 * (CELL_H + CELL_GAP) - CELL_GAP;
    let padding = 10.0;

    // ── Workspace name sizing ─────────────────────────────────────────────
    let name_galley = ui.painter().layout(
        workspace_name.to_string(),
        egui::FontId::proportional(NAME_FONT_SIZE),
        colors.text_primary,
        f32::INFINITY,
    );
    let name_w = name_galley.size().x;
    let name_h = name_galley.size().y;

    let panel_w = (grid_pixel_w + padding * 2.0).max(name_w + padding * 2.0 + 8.0);
    let panel_h = grid_pixel_h + padding * 2.0 + name_h + 4.0;

    let panel_min = egui::pos2(
        content_rect.right() - panel_w - INSET_RIGHT,
        content_rect.top() + INSET_TOP,
    );
    let panel_rect = egui::Rect::from_min_size(panel_min, egui::Vec2::new(panel_w, panel_h));

    ui.painter().rect_filled(
        panel_rect,
        egui::CornerRadius::same(CORNER_RADIUS as u8),
        colors.bg_sidebar,
    );
    ui.painter().rect_stroke(
        panel_rect,
        egui::CornerRadius::same(CORNER_RADIUS as u8),
        egui::Stroke::new(1.0, colors.border),
        egui::StrokeKind::Inside,
    );

    // Workspace name
    let name_x = panel_rect.center().x - name_w * 0.5;
    let name_y = panel_rect.min.y + padding * 0.5;
    ui.painter().galley(
        egui::pos2(name_x, name_y),
        name_galley,
        colors.text_primary,
    );

    let grid_origin = egui::pos2(
        panel_min.x + (panel_w - grid_pixel_w) * 0.5,
        panel_rect.min.y + padding + name_h + 2.0,
    );
    // Claim the full panel rect so clicks on the background, title, and cell
    // gaps don't fall through to whatever widget is rendered beneath the
    // overlay. In egui, painting does not claim input — only allocating with a
    // Sense does. Without this, the pane behind the minimap would still receive
    // and act on any click that didn't land exactly on a cell.
    ui.allocate_rect(panel_rect, egui::Sense::hover());

    let mut clicked: Option<usize> = None;

    for (original_idx, ctx) in &visible {
        let idx = *original_idx;
        let mapped_y = row_remap.get(&ctx.grid_y).copied().unwrap_or(0);
        let cell_x = grid_origin.x + ctx.grid_x as f32 * (CELL_W + CELL_GAP);
        let cell_y = grid_origin.y + mapped_y as f32 * (CELL_H + CELL_GAP);
        let cell_rect = egui::Rect::from_min_size(
            egui::pos2(cell_x, cell_y),
            egui::Vec2::new(CELL_W, CELL_H),
        );

        let is_active = idx == active_window;
        let is_trail = !is_active
            && last_visited.get(&ctx.grid_y).copied() == Some(ctx.grid_x);

        let fill = if is_active {
            colors.accent
        } else {
            colors.bg_active
        };

        let border_color = if is_trail { colors.accent } else { colors.border };

        ui.painter().rect(
            cell_rect,
            egui::CornerRadius::same(3),
            fill,
            egui::Stroke::new(1.0, border_color),
            egui::StrokeKind::Inside,
        );

        // Small page number inside each cell (0-based, left-bottom)
        let page_num = visible.iter().position(|(i, _)| *i == idx).unwrap_or(0);
        ui.painter().text(
            egui::pos2(cell_rect.left() + 3.0, cell_rect.bottom() - 2.0),
            egui::Align2::LEFT_BOTTOM,
            format!("{}", page_num),
            egui::FontId::proportional(9.0),
            if is_active { colors.bg_darkest } else { colors.text_dim },
        );

        let cell_response = ui.interact(
            cell_rect,
            egui::Id::new(("minimap_cell", idx)),
            egui::Sense::click(),
        );
        if cell_response.clicked() {
            clicked = Some(idx);
        }
        if cell_response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }

    clicked
}
