//! Spatial-grid minimap overlay.
//!
//! Renders a small top-right overlay showing the 2-D page grid. Each cell
//! represents one context (page). The active page is highlighted with the
//! accent color. Clicking a cell switches to that page. The overlay fades to
//! near-transparent after 3 s of inactivity and can be pinned visible via
//! `Cmd+Shift+M`.

use crate::context::Context;
use crate::theme::Colors;
use std::time::Instant;

/// Runtime state for the minimap overlay.
pub struct MinimapState {
    /// Whether the minimap is shown at all. `Cmd+Shift+M` toggles this.
    pub visible: bool,
    last_activity: Instant,
}

impl MinimapState {
    pub fn new() -> Self {
        Self {
            visible: false,
            last_activity: Instant::now(),
        }
    }

    /// Call whenever the user presses a key or changes the active page so the
    /// minimap stays at full opacity for another 3 s.
    pub fn on_activity(&mut self) {
        self.last_activity = Instant::now();
    }

    /// Returns `true` when the minimap should render at reduced opacity
    /// (>3 s since last activity).
    pub fn is_faded(&self) -> bool {
        self.last_activity.elapsed().as_secs_f32() > 3.0
    }

    /// Toggle show/hide (`Cmd+Shift+M`).
    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }
}

/// Cell dimensions and gap used when rendering the minimap grid.
const CELL_W: f32 = 16.0;
const CELL_H: f32 = 12.0;
const CELL_GAP: f32 = 2.0;
const INSET_RIGHT: f32 = 16.0;
/// Top inset clears the toolbar (~34px) plus the `?` shortcuts button (~44px).
const INSET_TOP: f32 = 52.0;
const CORNER_RADIUS: f32 = 4.0;

/// Render the minimap overlay into `ui`. Returns `Some(index)` if the user
/// clicked a cell (caller should switch `active_context` to that index).
///
/// The minimap is positioned in the top-right corner of `content_rect` with
/// `INSET` px of margin.
///
/// `last_visited` maps each `grid_y` row to the `grid_x` of the last page
/// visited there. Cells matching this map (but not the active cell) receive a
/// subtle trail highlight so the user can see where they came from.
pub fn render_minimap(
    ui: &mut egui::Ui,
    content_rect: egui::Rect,
    contexts: &[Context],
    active_context: usize,
    state: &MinimapState,
    last_visited: &std::collections::HashMap<u32, u32>,
    colors: &Colors,
) -> Option<usize> {
    if contexts.is_empty() {
        return None;
    }

    let alpha_mult = if state.is_faded() { 0.15_f32 } else { 1.0_f32 };

    // Compute bounding box of the grid in grid-coordinate space so we can
    // determine the pixel dimensions of the minimap panel.
    let max_x = contexts.iter().map(|c| c.grid_x).max().unwrap_or(0);
    let max_y = contexts.iter().map(|c| c.grid_y).max().unwrap_or(0);
    let cols = max_x + 1;
    let rows = max_y + 1;

    let grid_pixel_w = cols as f32 * (CELL_W + CELL_GAP) - CELL_GAP;
    let grid_pixel_h = rows as f32 * (CELL_H + CELL_GAP) - CELL_GAP;
    let padding = 6.0;
    let panel_w = grid_pixel_w + padding * 2.0;
    let panel_h = grid_pixel_h + padding * 2.0;

    // Top-right inset position.
    let panel_min = egui::pos2(
        content_rect.right() - panel_w - INSET_RIGHT,
        content_rect.top() + INSET_TOP,
    );
    let panel_rect = egui::Rect::from_min_size(panel_min, egui::Vec2::new(panel_w, panel_h));

    // Draw the panel background.
    let bg_color = apply_alpha(colors.bg_sidebar, alpha_mult * 0.4);
    ui.painter().rect_filled(
        panel_rect,
        egui::CornerRadius::same(CORNER_RADIUS as u8),
        bg_color,
    );

    // Draw each context cell.
    let grid_origin = egui::pos2(panel_min.x + padding, panel_min.y + padding);
    let mut clicked: Option<usize> = None;

    for (idx, ctx) in contexts.iter().enumerate() {
        let cell_x = grid_origin.x + ctx.grid_x as f32 * (CELL_W + CELL_GAP);
        let cell_y = grid_origin.y + ctx.grid_y as f32 * (CELL_H + CELL_GAP);
        let cell_rect = egui::Rect::from_min_size(
            egui::pos2(cell_x, cell_y),
            egui::Vec2::new(CELL_W, CELL_H),
        );

        let is_active = idx == active_context;
        let is_trail = !is_active
            && last_visited.get(&ctx.grid_y).copied() == Some(ctx.grid_x);

        // Trail cells get a subtle accent-colored background drawn first.
        if is_trail {
            let trail_bg = apply_alpha(colors.accent, alpha_mult * 0.12);
            ui.painter().rect_filled(
                cell_rect,
                egui::CornerRadius::same(2),
                trail_bg,
            );
        }

        // Fill: active page = accent @ 80%, inactive = bg_active @ 60%.
        let fill = if is_active {
            apply_alpha(colors.accent, alpha_mult * 0.80)
        } else {
            apply_alpha(colors.bg_active, alpha_mult * 0.60)
        };

        // Border: trail cells get a slightly brighter border.
        let border_alpha = if is_trail { 0.60 } else { 0.40 };
        let border_color = apply_alpha(colors.border, alpha_mult * border_alpha);

        ui.painter().rect(
            cell_rect,
            egui::CornerRadius::same(2),
            fill,
            egui::Stroke::new(1.0, border_color),
            egui::StrokeKind::Inside,
        );

        // Invisible interact area over each cell so clicks register.
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

/// Apply an alpha multiplier to a `Color32`, scaling only the alpha channel.
fn apply_alpha(color: egui::Color32, mult: f32) -> egui::Color32 {
    let [r, g, b, a] = color.to_array();
    let new_a = ((a as f32 * mult).round() as u32).min(255) as u8;
    egui::Color32::from_rgba_premultiplied(r, g, b, new_a)
}
