use egui::{Align2, Color32, CornerRadius, Pos2, Response, Stroke, StrokeKind, Vec2};

use crate::ui::style;
use crate::ui::theme::Colors;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ListRowPips {
    pub count: usize,
    pub focused_idx: Option<usize>,
    pub hidden_indices: Vec<usize>,
    /// Per-pip agent state (parallel to pip index). `None` = no agent.
    pub activities: Vec<Option<crate::app_protocol::AgentState>>,
}

pub struct ListRow<'a> {
    body: &'a str,
    secondary: Option<&'a str>,
    chip: Option<&'a str>,
    metadata_chips: &'a [&'a str],
    pane_pips: Option<ListRowPips>,
    trailing: Option<&'a str>,
    selected: bool,
    danger_trailing: bool,
    dense: bool,
}

impl<'a> ListRow<'a> {
    pub fn new(body: &'a str) -> Self {
        Self {
            body,
            secondary: None,
            chip: None,
            metadata_chips: &[],
            pane_pips: None,
            trailing: None,
            selected: false,
            danger_trailing: false,
            dense: false,
        }
    }

    /// Single-line row at `LIST_ROW_DENSE_H` for data-dense listings.
    /// Dense rows carry no secondary line — callers wanting two lines
    /// keep the default height.
    pub fn dense(mut self) -> Self {
        self.dense = true;
        self.secondary = None;
        self
    }

    pub fn secondary(mut self, secondary: &'a str) -> Self {
        self.secondary = (!secondary.is_empty()).then_some(secondary);
        self
    }

    pub fn chip(mut self, label: &'a str) -> Self {
        self.chip = Some(label);
        self
    }

    pub fn metadata_chips(mut self, labels: &'a [&'a str]) -> Self {
        self.metadata_chips = labels;
        self
    }

    pub fn pane_pips(mut self, pips: ListRowPips) -> Self {
        self.pane_pips = Some(pips);
        self
    }

    pub fn trailing_action(mut self, label: &'a str) -> Self {
        self.trailing = Some(label);
        self
    }

    pub fn danger_trailing(mut self, danger: bool) -> Self {
        self.danger_trailing = danger;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn show(self, ui: &mut egui::Ui, colors: &Colors) -> ListRowResponse {
        let row_h = if self.dense {
            style::LIST_ROW_DENSE_H
        } else {
            style::LIST_ROW_H
        };
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), row_h),
            egui::Sense::click(),
        );
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        let center_y = rect.center().y;

        // Trailing rect is computable before any painting — needed early so
        // the danger glow can be drawn behind the row content.
        let trailing_rect = self.trailing.map(|label| {
            let size = trailing_size(ui, label);
            egui::Rect::from_center_size(
                Pos2::new(rect.right() - TRAILING_PAD_H - size.x / 2.0, center_y),
                size,
            )
        });
        let trailing_hovered = trailing_rect.is_some_and(|r| ui.rect_contains_pointer(r));

        let inset = selection_inset(rect);
        if self.selected {
            paint_selection(ui.painter(), rect, colors);
        } else if self.danger_trailing && trailing_hovered {
            // Hovering a destructive action warns at row level: soft danger
            // tint + hairline danger outline before anything is clicked.
            ui.painter()
                .rect_filled(inset, style::RADIUS_SM, colors.danger.gamma_multiply(0.07));
            ui.painter().rect_stroke(
                inset,
                style::RADIUS_SM,
                Stroke::new(1.0, colors.danger.gamma_multiply(0.35)),
                StrokeKind::Inside,
            );
        } else if response.hovered() {
            ui.painter()
                .rect_filled(inset, style::RADIUS_SM, colors.bg_hover);
        }

        let x = rect.left() + style::LIST_ROW_PAD_H;

        let trailing_response = self
            .trailing
            .zip(trailing_rect)
            .map(|(label, trailing_rect)| {
                let id = response.id.with("_trailing_action");
                let trailing_response = ui.interact(trailing_rect, id, egui::Sense::click());
                // Destructive actions read as destructive at rest — soft danger,
                // escalating to full danger on hover.
                let color = if self.danger_trailing {
                    if trailing_response.hovered() {
                        colors.danger
                    } else {
                        colors.danger.gamma_multiply(0.75)
                    }
                } else {
                    colors.text_dim
                };
                ui.painter().text(
                    trailing_rect.center(),
                    Align2::CENTER_CENTER,
                    label,
                    egui::FontId::proportional(style::TEXT_CAPTION),
                    color,
                );
                trailing_response
            });

        let text_right = self.text_right(ui, rect);
        // The chip trails the title so every row's title starts at the same
        // x — variable-width leading chips made list starts ragged. Reserve
        // its width out of the primary line's budget only; secondary metadata
        // gets the full width.
        let chip_reserved = self
            .chip
            .map(|label| chip_size(ui, label).x + style::LIST_ROW_GAP)
            .unwrap_or(0.0);
        let primary_max = (text_right - x - chip_reserved).max(0.0);
        let secondary_max = (text_right - x).max(0.0);
        let text_metrics =
            draw_text_block(ui, &self, colors, x, center_y, primary_max, secondary_max);
        if let Some(label) = self.chip {
            draw_chip(
                ui,
                label,
                colors,
                text_metrics.primary_end_x + style::LIST_ROW_GAP,
                text_metrics.primary_center_y,
            );
        }
        self.draw_metadata(ui, colors, rect, trailing_rect, text_metrics);

        ListRowResponse {
            row: response,
            trailing: trailing_response,
        }
    }

    fn action_left(&self, ui: &egui::Ui, rect: egui::Rect) -> f32 {
        self.trailing
            .map(|label| rect.right() - TRAILING_PAD_H - trailing_size(ui, label).x)
            .unwrap_or_else(|| rect.right() - style::LIST_ROW_PAD_H)
    }

    fn text_right(&self, ui: &egui::Ui, rect: egui::Rect) -> f32 {
        let action_left = self.action_left(ui, rect);
        let has_trailing = self.trailing.is_some();
        let metadata_width = self.metadata_lane_width(ui);
        if metadata_width > 0.0 {
            action_left - style::LIST_ROW_GAP - metadata_width - style::LIST_ROW_GAP
        } else if has_trailing {
            action_left - style::LIST_ROW_GAP
        } else {
            action_left
        }
    }

    fn metadata_lane_width(&self, ui: &egui::Ui) -> f32 {
        let chip_width = metadata_chip_row_width(ui, self.metadata_chips);
        let pip_width = self
            .pane_pips
            .as_ref()
            .map(|pips| pane_pips_width(pips.count))
            .unwrap_or(0.0);
        chip_width.max(pip_width)
    }

    fn draw_metadata(
        &self,
        ui: &egui::Ui,
        colors: &Colors,
        rect: egui::Rect,
        trailing_rect: Option<egui::Rect>,
        text_metrics: TextBlockMetrics,
    ) {
        let lane_width = self.metadata_lane_width(ui);
        if lane_width <= 0.0 {
            return;
        }
        let action_left = trailing_rect
            .map(|r| r.left())
            .unwrap_or_else(|| rect.right() - style::LIST_ROW_PAD_H);
        let lane_right = action_left - style::LIST_ROW_GAP;
        let lane_left = lane_right - lane_width;

        let has_compact_pips =
            self.pane_pips.is_some() && text_metrics.secondary_center_y.is_none();
        let chip_center_y = if has_compact_pips {
            text_metrics.primary_center_y - COMPACT_METADATA_CHIP_OFFSET_Y
        } else {
            text_metrics.primary_center_y
        };

        if !self.metadata_chips.is_empty() {
            draw_chip_row(ui, self.metadata_chips, colors, lane_right, chip_center_y);
        }

        if let Some(pips) = &self.pane_pips {
            let center_y = text_metrics
                .secondary_center_y
                .unwrap_or(text_metrics.primary_center_y + COMPACT_METADATA_PIP_OFFSET_Y);
            let t = ui.input(|i| i.time);
            let has_working = pips.activities.iter().any(|s| {
                matches!(s, Some(crate::app_protocol::AgentState::Working))
            });
            if has_working {
                ui.ctx().request_repaint();
            }
            draw_pips(ui, pips, colors, lane_left, lane_right, center_y, t);
        }
    }
}

pub struct ListRowResponse {
    row: Response,
    trailing: Option<Response>,
}

impl ListRowResponse {
    pub fn row_clicked(&self) -> bool {
        self.row.clicked()
    }

    pub fn row_double_clicked(&self) -> bool {
        self.row.double_clicked()
    }

    pub fn row_hovered(&self) -> bool {
        self.row.hovered()
    }

    pub fn scroll_to_me(&self, align: Option<egui::Align>) {
        self.row.scroll_to_me(align);
    }

    pub fn trailing_clicked(&self) -> bool {
        self.trailing.as_ref().is_some_and(Response::clicked)
    }
}

/// Paints the primary title (and optional secondary line) and returns the
/// primary line's end x and vertical center, so the trailing chip can be
/// placed right after the title text.
fn draw_text_block(
    ui: &mut egui::Ui,
    row: &ListRow<'_>,
    colors: &Colors,
    x: f32,
    center_y: f32,
    primary_max: f32,
    secondary_max: f32,
) -> TextBlockMetrics {
    // Primary is always full-strength — selection is conveyed by the accent
    // rail and tint, not by dimming unselected titles into the metadata.
    let primary_color = colors.text_primary;
    // Primary reads one step above secondary metadata — both size and weight,
    // otherwise the two lines collapse into one undifferentiated block.
    let primary_font = crate::ui::theme::font_medium(style::TEXT_CAPTION);
    let primary = elided_galley(ui, row.body, primary_font, primary_color, primary_max);
    let primary_size = primary.size();

    if let Some(secondary) = row.secondary {
        let secondary_font = egui::FontId::proportional(style::TEXT_HINT);
        let secondary_galley = elided_galley(
            ui,
            secondary,
            secondary_font,
            colors.text_dim,
            secondary_max,
        );
        let secondary_h = secondary_galley.size().y;
        let total_h = primary_size.y + 2.0 + secondary_h;
        let primary_pos = Pos2::new(x, center_y - total_h / 2.0);
        let secondary_pos = Pos2::new(x, primary_pos.y + primary_size.y + 2.0);
        ui.painter().galley(primary_pos, primary, primary_color);
        ui.painter()
            .galley(secondary_pos, secondary_galley, colors.text_dim);
        TextBlockMetrics {
            primary_end_x: primary_pos.x + primary_size.x,
            primary_center_y: primary_pos.y + primary_size.y / 2.0,
            secondary_center_y: Some(secondary_pos.y + secondary_h / 2.0),
        }
    } else {
        let pos = Pos2::new(x, center_y - primary_size.y / 2.0);
        ui.painter().galley(pos, primary, primary_color);
        TextBlockMetrics {
            primary_end_x: pos.x + primary_size.x,
            primary_center_y: center_y,
            secondary_center_y: None,
        }
    }
}

#[derive(Clone, Copy)]
struct TextBlockMetrics {
    primary_end_x: f32,
    primary_center_y: f32,
    secondary_center_y: Option<f32>,
}

fn elided_galley(
    ui: &egui::Ui,
    text: &str,
    font_id: egui::FontId,
    color: Color32,
    max_width: f32,
) -> std::sync::Arc<egui::Galley> {
    let text = elide_to_width(ui, text, font_id.clone(), max_width);
    ui.fonts(|f| f.layout_no_wrap(text, font_id, color))
}

fn elide_to_width(ui: &egui::Ui, text: &str, font_id: egui::FontId, max_width: f32) -> String {
    if max_width <= 0.0 {
        return String::new();
    }

    let width = |s: &str| {
        ui.fonts(|f| {
            f.layout_no_wrap(s.to_string(), font_id.clone(), Color32::WHITE)
                .size()
                .x
        })
    };

    if width(text) <= max_width {
        return text.to_string();
    }

    const ELLIPSIS: &str = "...";
    if width(ELLIPSIS) > max_width {
        return String::new();
    }

    let mut out = String::new();
    for ch in text.chars() {
        let mut candidate = out.clone();
        candidate.push(ch);
        candidate.push_str(ELLIPSIS);
        if width(&candidate) > max_width {
            break;
        }
        out.push(ch);
    }
    out.push_str(ELLIPSIS);
    out
}

/// The selection highlight spans the row's full width — flush with the
/// text fields above it in palettes/pickers — and stops 1px short
/// vertically so adjacent selected rows don't fuse into one slab.
fn selection_inset(row_rect: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_max(
        Pos2::new(row_rect.min.x, row_rect.min.y + 1.0),
        Pos2::new(row_rect.max.x, row_rect.max.y - 1.0),
    )
}

/// The two shapes (tint fill + soft outline) of the shared selection
/// treatment, for widgets that paint into pre-reserved shape layers
/// (see `selectable_row`).
pub fn selection_shapes(row_rect: egui::Rect, colors: &Colors) -> [egui::Shape; 2] {
    let inset = selection_inset(row_rect);
    [
        egui::Shape::rect_filled(inset, style::RADIUS_SM, colors.accent.gamma_multiply(0.14)),
        egui::Shape::rect_stroke(
            inset,
            style::RADIUS_SM,
            Stroke::new(1.0, colors.accent.gamma_multiply(0.45)),
            StrokeKind::Inside,
        ),
    ]
}

/// Shared selection treatment for the host ListRow, `selectable_row`, and
/// the PGAP ListView renderer: accent-tinted fill with a soft accent
/// outline. One visual language for "this row is selected" everywhere.
pub fn paint_selection(painter: &egui::Painter, row_rect: egui::Rect, colors: &Colors) {
    for shape in selection_shapes(row_rect, colors) {
        painter.add(shape);
    }
}

/// Trailing actions sit further off the row edge than body content — they are
/// peripheral affordances, not content, and need air on the right.
const TRAILING_PAD_H: f32 = style::LIST_ROW_PAD_H + style::SPACE_SM;

fn trailing_size(ui: &egui::Ui, label: &str) -> Vec2 {
    let galley = ui.fonts(|f| {
        f.layout_no_wrap(
            label.to_string(),
            egui::FontId::proportional(style::TEXT_CAPTION),
            Color32::WHITE,
        )
    });
    Vec2::new(galley.size().x.max(24.0), style::LIST_ROW_H)
}

// Row chips share the key-chip visual language: borderless bg_active fill,
// dim monospace text, quiet annotations — never louder than the title.
const CHIP_PAD_H: f32 = 4.0;
const CHIP_PAD_V: f32 = 1.5;

fn chip_galley(ui: &egui::Ui, label: &str, colors: &Colors) -> std::sync::Arc<egui::Galley> {
    ui.fonts(|f| {
        f.layout_no_wrap(
            label.to_string(),
            egui::FontId::monospace(style::TEXT_HINT),
            colors.text_dim,
        )
    })
}

fn chip_size(ui: &egui::Ui, label: &str) -> Vec2 {
    let galley = ui.fonts(|f| {
        f.layout_no_wrap(
            label.to_string(),
            egui::FontId::monospace(style::TEXT_HINT),
            Color32::WHITE,
        )
    });
    let h = galley.size().y + CHIP_PAD_V * 2.0;
    Vec2::new((galley.size().x + CHIP_PAD_H * 2.0).max(h), h)
}

fn draw_chip(ui: &egui::Ui, label: &str, colors: &Colors, x: f32, center_y: f32) {
    let galley = chip_galley(ui, label, colors);
    let size = chip_size(ui, label);
    let rect = egui::Rect::from_min_size(Pos2::new(x, center_y - size.y / 2.0), size);
    ui.painter()
        .rect_filled(rect, CornerRadius::same(4), colors.bg_active);
    ui.painter().galley(
        Pos2::new(
            rect.center().x - galley.size().x / 2.0,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        colors.text_dim,
    );
}

fn metadata_chip_row_width(ui: &egui::Ui, labels: &[&str]) -> f32 {
    if labels.is_empty() {
        return 0.0;
    }
    labels
        .iter()
        .map(|label| chip_size(ui, label).x)
        .sum::<f32>()
        + style::SPACE_XS * labels.len().saturating_sub(1) as f32
}

fn draw_chip_row(ui: &egui::Ui, labels: &[&str], colors: &Colors, right: f32, center_y: f32) {
    let total_w = metadata_chip_row_width(ui, labels);
    let mut x = right - total_w;
    for label in labels {
        draw_chip(ui, label, colors, x, center_y);
        x += chip_size(ui, label).x + style::SPACE_XS;
    }
}

const PANE_PIP_RADIUS: f32 = 3.0;
const PANE_PIP_SPACING: f32 = 9.0;
const PANE_PIP_MAX: usize = 8;
const COMPACT_METADATA_CHIP_OFFSET_Y: f32 = 10.0;
const COMPACT_METADATA_PIP_OFFSET_Y: f32 = 10.0;

fn pane_pips_width(count: usize) -> f32 {
    if count == 0 {
        return 0.0;
    }
    let capped = count.min(PANE_PIP_MAX);
    let base = (capped.saturating_sub(1) as f32) * PANE_PIP_SPACING + PANE_PIP_RADIUS * 2.0;
    if count > PANE_PIP_MAX {
        base + style::SPACE_XS + 16.0
    } else {
        base
    }
}

fn draw_pips(
    ui: &egui::Ui,
    pips: &ListRowPips,
    colors: &Colors,
    lane_left: f32,
    lane_right: f32,
    center_y: f32,
    time: f64,
) {
    if pips.count == 0 {
        return;
    }
    let width = pane_pips_width(pips.count);
    let start_x = (lane_right - width).max(lane_left);
    let capped = pips.count.min(PANE_PIP_MAX);
    for dot_i in 0..capped {
        let center = Pos2::new(
            start_x + PANE_PIP_RADIUS + dot_i as f32 * PANE_PIP_SPACING,
            center_y,
        );
        let hidden = pips.hidden_indices.contains(&dot_i);
        let agent_state = pips.activities.get(dot_i).and_then(|s| s.as_ref());
        let color = crate::ui::activity::pip_color(
            agent_state,
            pips.focused_idx == Some(dot_i),
            colors,
            time,
        );
        if hidden {
            ui.painter()
                .circle_stroke(center, PANE_PIP_RADIUS, Stroke::new(1.0, color));
        } else {
            ui.painter().circle_filled(center, PANE_PIP_RADIUS, color);
        }
    }
    if pips.count > PANE_PIP_MAX {
        let overflow_x = start_x + PANE_PIP_RADIUS + capped as f32 * PANE_PIP_SPACING;
        let overflow_color = if pips.focused_idx.is_some_and(|idx| idx >= PANE_PIP_MAX) {
            colors.accent
        } else {
            colors.text_dim.gamma_multiply(0.55)
        };
        ui.painter().text(
            Pos2::new(overflow_x, center_y),
            Align2::LEFT_CENTER,
            format!("+{}", pips.count - PANE_PIP_MAX),
            egui::FontId::proportional(style::TEXT_HINT),
            overflow_color,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_row_builder_keeps_optional_secondary_empty_by_default() {
        let row = ListRow::new("note").secondary("").chip("app");
        assert_eq!(row.body, "note");
        assert!(row.secondary.is_none());
        assert_eq!(row.chip, Some("app"));
        assert!(row.metadata_chips.is_empty());
        assert!(!row.selected);
    }

    #[test]
    fn trailing_metadata_reserves_text_width() {
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput::default());
        egui::CentralPanel::default().show(&ctx, |ui| {
            let row_rect =
                egui::Rect::from_min_size(Pos2::ZERO, Vec2::new(240.0, style::LIST_ROW_H));
            let plain = ListRow::new("Assistant").secondary("Long app description");
            let metadata = ListRow::new("Assistant")
                .secondary("Long app description")
                .metadata_chips(&["app", "ws"]);

            assert!(
                metadata.text_right(ui, row_rect) < plain.text_right(ui, row_rect),
                "metadata lane must reduce the text area so secondary text truncates before chips"
            );
        });
        let _ = ctx.end_pass();
    }

    #[test]
    fn pane_pips_reserve_the_metadata_lane() {
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput::default());
        egui::CentralPanel::default().show(&ctx, |ui| {
            let row_rect =
                egui::Rect::from_min_size(Pos2::ZERO, Vec2::new(240.0, style::LIST_ROW_H));
            let row = ListRow::new("Workspace").pane_pips(ListRowPips {
                count: 3,
                focused_idx: Some(1),
                hidden_indices: vec![2],
                activities: vec![None, None, None],
            });

            assert!(row.metadata_lane_width(ui) > 0.0);
            assert!(row.text_right(ui, row_rect) < row_rect.right() - style::LIST_ROW_PAD_H);
        });
        let _ = ctx.end_pass();
    }

    #[test]
    fn compact_metadata_keeps_pips_padded_below_chip() {
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput::default());
        egui::CentralPanel::default().show(&ctx, |ui| {
            let primary_center_y = style::LIST_ROW_H / 2.0;
            let chip_bottom =
                primary_center_y - COMPACT_METADATA_CHIP_OFFSET_Y + chip_size(ui, "ctx").y / 2.0;
            let pip_top = primary_center_y + COMPACT_METADATA_PIP_OFFSET_Y - PANE_PIP_RADIUS;

            assert!(
                pip_top - chip_bottom >= style::SPACE_SM,
                "compact metadata pips need visible padding below the chip"
            );
        });
        let _ = ctx.end_pass();
    }

    #[test]
    fn elide_to_width_keeps_text_single_line_within_width() {
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput::default());
        egui::CentralPanel::default().show(&ctx, |ui| {
            let font = egui::FontId::proportional(style::TEXT_HINT);
            let elided = elide_to_width(ui, "a very long note filename.md", font.clone(), 40.0);
            let galley = ui.fonts(|f| f.layout_no_wrap(elided, font, Color32::WHITE));
            assert!(galley.size().x <= 40.0);
            assert_eq!(galley.rows.len(), 1);
        });
        let _ = ctx.end_pass();
    }
}
