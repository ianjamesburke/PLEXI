use egui::{Align2, Color32, Pos2, Response, Stroke, StrokeKind, Vec2};

use crate::ui::style;
use crate::ui::theme::Colors;

pub struct ListDropdownHeader<'a> {
    label: &'a str,
    expanded: bool,
    indent: f32,
}

impl<'a> ListDropdownHeader<'a> {
    pub fn new(label: &'a str, expanded: bool) -> Self {
        Self {
            label,
            expanded,
            indent: style::LIST_ROW_PAD_H,
        }
    }

    pub fn indent(mut self, indent: f32) -> Self {
        self.indent = indent;
        self
    }

    pub fn show(self, ui: &mut egui::Ui, id: egui::Id, colors: &Colors) -> egui::Response {
        let (rect, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), style::LIST_DROPDOWN_HEADER_H),
            egui::Sense::hover(),
        );
        let response = ui.interact(rect, id, egui::Sense::click());
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        let inset = selection_inset(rect);
        if response.hovered() {
            ui.painter()
                .rect_filled(inset, style::RADIUS_SM, colors.bg_hover);
        }

        let chevron_rect = egui::Rect::from_min_size(
            Pos2::new(rect.left() + self.indent, rect.top()),
            Vec2::new(style::LIST_DROPDOWN_CHEVRON_W, rect.height()),
        );
        let center = chevron_rect.center();
        let chevron_color = if response.hovered() {
            colors.text_primary
        } else {
            colors.text_section
        };
        let r = 4.5;
        let fraction = style::micro_motion(
            ui.ctx(),
            id.with("chevron"),
            self.expanded,
            style::DURATION_MICRO_MS,
        );
        let rotation = egui::emath::Rot2::from_angle(fraction * std::f32::consts::FRAC_PI_2);
        let points = [
            egui::vec2(-r * 0.45, -r),
            egui::vec2(-r * 0.45, r),
            egui::vec2(r * 0.65, 0.0),
        ]
        .into_iter()
        .map(|point| center + rotation * point)
        .collect();
        ui.painter().add(egui::Shape::convex_polygon(
            points,
            chevron_color,
            Stroke::NONE,
        ));

        paint_text_left_center(
            ui,
            self.label,
            crate::ui::theme::font_medium(style::TEXT_CAPTION),
            colors.text_section,
            chevron_rect.right() + style::SPACE_XS,
            rect.center().y,
        );

        response
    }
}

pub(crate) fn paint_text_left_center(
    ui: &egui::Ui,
    text: impl Into<String>,
    font_id: egui::FontId,
    color: Color32,
    left: f32,
    center_y: f32,
) {
    let galley = ui.fonts_mut(|f| f.layout_no_wrap(text.into(), font_id, color));
    crate::ui::snap::galley_snapped(
        ui.painter(),
        Pos2::new(left, center_y - galley.size().y / 2.0),
        galley,
        color,
    );
}

pub(crate) fn paint_text_centered(
    ui: &egui::Ui,
    text: impl Into<String>,
    font_id: egui::FontId,
    color: Color32,
    center: Pos2,
) {
    let galley = ui.fonts_mut(|f| f.layout_no_wrap(text.into(), font_id, color));
    crate::ui::snap::galley_snapped(
        ui.painter(),
        Pos2::new(
            center.x - galley.size().x / 2.0,
            center.y - galley.size().y / 2.0,
        ),
        galley,
        color,
    );
}

pub(crate) fn paint_table_header_background(ui: &egui::Ui, rect: egui::Rect, colors: &Colors) {
    ui.painter()
        .rect_filled(rect, style::RADIUS_MD, colors.bg_sidebar);
    ui.painter().rect_stroke(
        rect,
        style::RADIUS_MD,
        Stroke::new(1.0_f32, colors.border),
        StrokeKind::Inside,
    );
}

pub(crate) fn paint_table_header_label(
    ui: &egui::Ui,
    cell: egui::Rect,
    label: impl Into<String>,
    font_id: egui::FontId,
    colors: &Colors,
) {
    let label = label.into();
    let max_width = (cell.width() - style::SPACE_SM * 2.0).max(0.0);
    let elided = crate::ui::text::elide(
        ui,
        &label,
        font_id.clone(),
        max_width,
        crate::ui::text::Side::Trailing,
    );
    paint_text_left_center(
        ui,
        elided,
        font_id,
        colors.text_dim,
        cell.left() + style::SPACE_SM,
        cell.center().y,
    );
}

pub(crate) fn paint_table_cell_text(
    ui: &egui::Ui,
    cell: egui::Rect,
    text: impl Into<String>,
    font_id: egui::FontId,
    color: Color32,
) {
    paint_table_cell_text_elided(
        ui,
        cell,
        text,
        font_id,
        color,
        crate::ui::text::Side::Trailing,
    );
}

pub(crate) fn paint_table_cell_text_elided(
    ui: &egui::Ui,
    cell: egui::Rect,
    text: impl Into<String>,
    font_id: egui::FontId,
    color: Color32,
    side: crate::ui::text::Side,
) {
    let text = text.into();
    let max_width = (cell.width() - style::SPACE_SM * 2.0).max(0.0);
    let elided = crate::ui::text::elide(ui, &text, font_id.clone(), max_width, side);
    paint_text_left_center(
        ui,
        elided,
        font_id,
        color,
        cell.left() + style::SPACE_SM,
        cell.center().y,
    );
}

pub(crate) fn paint_table_row_background(
    ui: &egui::Ui,
    rect: egui::Rect,
    selected: bool,
    hovered: bool,
    colors: &Colors,
) {
    let fill = if selected {
        colors.bg_active
    } else if hovered {
        colors.bg_hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, style::RADIUS_MD, fill);
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ListRowPips {
    pub count: usize,
    pub focused_idx: Option<usize>,
    /// Per-pip agent state (parallel to pip index). `None` = no agent.
    pub activities: Vec<Option<crate::protocol::AgentState>>,
    pub static_health_pips: Vec<usize>,
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
    body_color: Option<Color32>,
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
            body_color: None,
        }
    }

    /// Preferred colour for the primary line (a state colour). It is still
    /// contrast-corrected against the painted row surface.
    pub fn body_color(mut self, color: Color32) -> Self {
        self.body_color = Some(color);
        self
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
        // Below this minimum the parent must provide scrolling or a wider
        // container; a mandatory action never becomes a blank sliver.
        ui.set_min_width(style::BUTTON_H_MD + style::LIST_ROW_PAD_H + TRAILING_PAD_H);
        let base_h = if self.dense {
            style::LIST_ROW_DENSE_H
        } else {
            style::LIST_ROW_H
        }
        .max(if self.trailing.is_some() {
            style::BUTTON_H_MD
        } else {
            0.0
        });
        let content_width =
            (ui.available_width() - style::LIST_ROW_PAD_H - TRAILING_PAD_H).max(style::BUTTON_H_MD);
        let action = self.trailing.map(|label| {
            ui.fonts_mut(|f| {
                f.layout(
                    label.into(),
                    egui::FontId::proportional(style::TEXT_CAPTION),
                    Color32::PLACEHOLDER,
                    content_width,
                )
            })
        });
        let action_size = action
            .as_ref()
            .map(|g| g.size().max(Vec2::splat(style::BUTTON_H_MD)));
        let stacked = action_size.is_some_and(|size| {
            size.x + style::LIST_ROW_GAP + style::PANE_HEADER_TITLE_MIN > content_width
        });
        let row_h = if stacked {
            base_h + style::SPACE_SM + action_size.map_or(0.0, |s| s.y)
        } else {
            base_h
        };
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), row_h), egui::Sense::click());
        let primary_rect = egui::Rect::from_min_size(rect.min, Vec2::new(rect.width(), base_h));
        let mut capture = super::layout_report::Capture::begin(ui.painter(), "list");
        capture.scrolling();
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Button,
                ui.is_enabled(),
                self.selected,
                self.body,
            )
        });
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let trailing_rect = action_size.map(|size| {
            if stacked {
                egui::Rect::from_min_size(
                    egui::pos2(
                        rect.left() + style::LIST_ROW_PAD_H,
                        rect.top() + base_h + style::SPACE_XS,
                    ),
                    size,
                )
            } else {
                egui::Rect::from_center_size(
                    egui::pos2(
                        rect.right() - TRAILING_PAD_H - size.x / 2.0,
                        primary_rect.center().y,
                    ),
                    Vec2::new(size.x, base_h),
                )
            }
        });
        let trailing_hovered = trailing_rect.is_some_and(|r| ui.rect_contains_pointer(r));
        let background = if self.selected {
            paint_selection(ui.painter(), rect, colors);
            colors.bg_sidebar.blend(colors.accent.gamma_multiply(0.14))
        } else if self.danger_trailing && trailing_hovered {
            ui.painter().rect_filled(
                selection_inset(rect),
                style::RADIUS_SM,
                colors.danger.gamma_multiply(0.07),
            );
            ui.painter().rect_stroke(
                selection_inset(rect),
                style::RADIUS_SM,
                Stroke::new(1.0_f32, colors.danger.gamma_multiply(0.35)),
                StrokeKind::Inside,
            );
            colors.bg_sidebar.blend(colors.danger.gamma_multiply(0.07))
        } else if response.hovered() {
            ui.painter()
                .rect_filled(selection_inset(rect), style::RADIUS_SM, colors.bg_hover);
            colors.bg_hover
        } else {
            colors.bg_sidebar
        };
        let surface = super::foreground::Surface::over(colors.bg_sidebar, background);
        let mut readable = *colors;
        readable.bg_sidebar = background;
        readable.text_primary =
            surface.foreground(colors.text_primary, super::foreground::Role::SmallText);
        let mut body_colors = readable;
        if let Some(color) = self.body_color {
            body_colors.text_primary =
                surface.foreground(color, super::foreground::Role::SmallText);
        }
        readable.text_dim = surface.foreground(colors.text_dim, super::foreground::Role::SmallText);
        let mut actions = Vec::new();
        let mut trailing_clicked = false;
        if let Some(((label, galley), hit)) = self.trailing.zip(action).zip(trailing_rect) {
            let action_response = ui.interact(
                hit,
                response.id.with("_trailing_action"),
                egui::Sense::click(),
            );
            action_response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
            });
            let preferred = if self.danger_trailing {
                colors.danger
            } else {
                colors.text_dim
            };
            let color = surface.foreground(preferred, super::foreground::Role::SmallText);
            capture.text_slot("action", &galley, hit);
            ui.painter().galley_with_override_text_color(
                hit.center() - galley.size() / 2.0,
                galley,
                color,
            );
            trailing_clicked = action_response.clicked();
            actions.push(super::layout_report::Action {
                id: "action".into(),
                name: label.into(),
                painted_label: label.into(),
                hit,
                minimum: Vec2::splat(style::BUTTON_H_MD),
                inactive: false,
            });
        }
        if stacked {
            static REFLOW: std::sync::Once = std::sync::Once::new();
            REFLOW.call_once(|| log::info!("ui: reflowing list action below its title"));
        }
        let lanes = self.lanes(ui, primary_rect, !stacked);
        let chip_width = self.chip.map(|label| chip_size(ui, label).x);
        let chip_slots: Vec<_> = chip_width
            .map(|width| super::bounded::Slot {
                id: "chip",
                width,
                compact: None,
                required: false,
            })
            .into_iter()
            .collect();
        if let Ok(title) = super::bounded::allocate(
            lanes.text,
            style::PANE_HEADER_TITLE_MIN,
            style::LIST_ROW_GAP,
            &chip_slots,
        ) {
            let metrics = draw_text_block(
                ui,
                &self,
                &body_colors,
                lanes.text.left(),
                primary_rect.center().y,
                title.text.width(),
                lanes.text.width(),
            );
            if let Some(label) = self.chip.filter(|_| {
                title
                    .slots
                    .iter()
                    .any(|s| s.presentation != super::bounded::Presentation::Hidden)
            }) {
                draw_chip(
                    ui,
                    label,
                    &readable,
                    metrics.primary_end_x + style::LIST_ROW_GAP,
                    metrics.primary_center_y,
                );
            }
            if let Some(metadata) = lanes.slots.iter().find(|s| s.id == "metadata") {
                self.draw_metadata(ui, &readable, primary_rect, metadata.rect, metrics);
            }
        }
        capture.finish(ui.painter(), rect, colors.bg_sidebar, actions, vec![]);
        ListRowResponse {
            row: response,
            trailing_clicked,
        }
    }

    fn lanes(
        &self,
        ui: &egui::Ui,
        rect: egui::Rect,
        reserve_action: bool,
    ) -> super::bounded::RowLayout<'static> {
        let bounds = egui::Rect::from_min_max(
            egui::pos2(rect.left() + style::LIST_ROW_PAD_H, rect.top()),
            egui::pos2(
                (rect.right() - TRAILING_PAD_H).max(rect.left() + style::LIST_ROW_PAD_H),
                rect.bottom(),
            ),
        );
        let mut slots = Vec::new();
        if let Some(label) = self.trailing.filter(|_| reserve_action) {
            slots.push(super::bounded::Slot {
                id: "action",
                width: trailing_size(ui, label).x,
                compact: None,
                required: true,
            });
        }
        let metadata = self.metadata_lane_width(ui);
        if metadata > 0.0 {
            slots.push(super::bounded::Slot {
                id: "metadata",
                width: metadata,
                compact: Some(metadata.min(bounds.width() * 0.45)),
                required: false,
            });
        }
        super::bounded::allocate(
            bounds,
            style::PANE_HEADER_TITLE_MIN,
            style::LIST_ROW_GAP,
            &slots,
        )
        .unwrap_or_else(|error| {
            log::info!(
                "ui: insufficient list row space; required={}",
                error.minimum_width
            );
            super::bounded::RowLayout {
                text: bounds,
                slots: Vec::new(),
            }
        })
    }

    fn metadata_lane_width(&self, ui: &egui::Ui) -> f32 {
        let chip_width = metadata_chip_row_width(ui, self.metadata_chips);
        let pip_width = self
            .pane_pips
            .as_ref()
            .map(|pips| pane_pips_width(ui, pips.count))
            .unwrap_or(0.0);
        chip_width.max(pip_width)
    }

    fn draw_metadata(
        &self,
        ui: &egui::Ui,
        colors: &Colors,
        rect: egui::Rect,
        lane: egui::Rect,
        text_metrics: TextBlockMetrics,
    ) {
        if lane.width() <= 0.0 {
            return;
        }
        let lane_left = lane.left();
        let lane_right = lane.right();

        let has_compact_pips =
            self.pane_pips.is_some() && text_metrics.secondary_center_y.is_none();
        let chip_center_y = if has_compact_pips {
            text_metrics.primary_center_y - COMPACT_METADATA_CHIP_OFFSET_Y
        } else if text_metrics.secondary_center_y.is_some() && self.pane_pips.is_some() {
            // Two-line row with pips: align chip with the title so pips rendered
            // at secondary_center_y have clear vertical separation.
            text_metrics.primary_center_y
        } else if text_metrics.secondary_center_y.is_some() {
            // Two-line row (no pips): center on the row for visual balance.
            rect.center().y
        } else {
            text_metrics.primary_center_y
        };

        if !self.metadata_chips.is_empty() {
            draw_chip_row(
                ui,
                self.metadata_chips,
                colors,
                lane_left,
                lane_right,
                chip_center_y,
            );
        }

        if let Some(pips) = &self.pane_pips {
            let center_y = text_metrics
                .secondary_center_y
                .unwrap_or(text_metrics.primary_center_y + COMPACT_METADATA_PIP_OFFSET_Y);
            let t = ui.input(|i| i.time);
            draw_pips(ui, pips, colors, lane_left, lane_right, center_y, t);
        }
    }
}

pub struct ListRowResponse {
    row: Response,
    trailing_clicked: bool,
}

impl ListRowResponse {
    pub(crate) fn response(&self) -> egui::Response {
        self.row.clone()
    }

    #[cfg(test)]
    pub(crate) fn rect(&self) -> egui::Rect {
        self.row.rect
    }

    pub fn row_clicked(&self) -> bool {
        self.row.clicked() || self.trailing_clicked
    }

    pub fn row_hovered(&self) -> bool {
        self.row.hovered()
    }

    /// See [`scroll_row_into_view`].
    pub fn scroll_into_view(&self, ui: &egui::Ui, selection_changed: bool) {
        scroll_row_into_view(ui, &self.row, selection_changed);
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
    let primary = crate::ui::text::elided_galley(
        ui,
        row.body,
        primary_font,
        primary_max,
        crate::ui::text::Side::Trailing,
    );
    let primary_size = primary.size();

    if let Some(secondary) = row.secondary {
        let secondary_font = egui::FontId::proportional(style::TEXT_HINT);
        let secondary_galley = crate::ui::text::elided_galley(
            ui,
            secondary,
            secondary_font,
            secondary_max,
            crate::ui::text::Side::Trailing,
        );
        let secondary_h = secondary_galley.size().y;
        let total_h = primary_size.y + 2.0 + secondary_h;
        let primary_pos = Pos2::new(x, center_y - total_h / 2.0);
        let secondary_pos = Pos2::new(x, primary_pos.y + primary_size.y + 2.0);
        crate::ui::snap::galley_snapped(ui.painter(), primary_pos, primary, primary_color);
        crate::ui::snap::galley_snapped(
            ui.painter(),
            secondary_pos,
            secondary_galley,
            colors.text_dim,
        );
        TextBlockMetrics {
            primary_end_x: primary_pos.x + primary_size.x,
            primary_center_y: primary_pos.y + primary_size.y / 2.0,
            secondary_center_y: Some(secondary_pos.y + secondary_h / 2.0),
        }
    } else {
        let pos = Pos2::new(x, center_y - primary_size.y / 2.0);
        crate::ui::snap::galley_snapped(ui.painter(), pos, primary, primary_color);
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

/// The selection highlight spans the row's full width — flush with the
/// text fields above it in palettes/pickers — and stops 1px short
/// vertically so adjacent selected rows don't fuse into one slab.
pub(crate) fn selection_inset(row_rect: egui::Rect) -> egui::Rect {
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
            Stroke::new(1.0_f32, colors.accent.gamma_multiply(0.45)),
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

/// Keeps the selected row of a keyboard-navigated list fully visible.
///
/// Call on the selected row's response inside the `ScrollArea`, every frame,
/// passing whether the selection changed this frame. egui already measured the
/// row's rect, so geometry (section headers, item spacing, dense rows) is
/// always exact — never compute row offsets by hand.
///
/// The enclosing `ScrollArea` MUST be built with `.animated(false)`. With the
/// default `animated(true)`, `scroll_to_me` routes through the ScrollArea's
/// `offset_target` animation state and the new offset is never applied to the
/// pass being presented — the scroll permanently lags the selection under
/// key-repeat, which is the original #2217 jitter. With `animated(false)` the
/// offset is written synchronously in the ScrollArea's `end()`, so the
/// discarded re-pass below renders at the corrected offset.
///
/// `request_discard` makes selection and scroll move atomically: the stale
/// pass is thrown away and only the corrected one is presented. Event-driven
/// input is safe across the re-run — `Context::run` hands `RawInput::take()`
/// to each pass, so key events fire exactly once.
pub(crate) fn scroll_row_into_view(ui: &egui::Ui, row: &egui::Response, selection_changed: bool) {
    if !selection_changed {
        return;
    }
    if ui.clip_rect().contains_rect(row.rect) {
        return; // already fully visible — no scroll, no extra pass
    }
    row.scroll_to_me(None);
    ui.ctx().request_discard("keyboard list scroll");
}

/// Trailing actions sit further off the row edge than body content — they are
/// peripheral affordances, not content, and need air on the right.
const TRAILING_PAD_H: f32 = style::LIST_ROW_PAD_H + style::SPACE_SM;

fn trailing_size(ui: &egui::Ui, label: &str) -> Vec2 {
    let galley = ui.fonts_mut(|f| {
        f.layout_no_wrap(
            label.to_string(),
            egui::FontId::proportional(style::TEXT_CAPTION),
            Color32::PLACEHOLDER,
        )
    });
    Vec2::new(galley.size().x.max(24.0), style::LIST_ROW_H)
}

// Row chips share the key-chip visual language: borderless bg_active fill,
// dim monospace text, quiet annotations — never louder than the title.
const CHIP_PAD_H: f32 = 4.0;
const CHIP_PAD_V: f32 = 1.5;

fn chip_galley(ui: &egui::Ui, label: &str, color: egui::Color32) -> std::sync::Arc<egui::Galley> {
    ui.fonts_mut(|f| {
        f.layout_no_wrap(
            label.to_string(),
            egui::FontId::monospace(style::TEXT_HINT),
            color,
        )
    })
}

fn chip_size(ui: &egui::Ui, label: &str) -> Vec2 {
    let galley = ui.fonts_mut(|f| {
        f.layout_no_wrap(
            label.to_string(),
            egui::FontId::monospace(style::TEXT_HINT),
            Color32::PLACEHOLDER,
        )
    });
    let h = galley.size().y + CHIP_PAD_V * 2.0;
    Vec2::new((galley.size().x + CHIP_PAD_H * 2.0).max(h), h)
}

fn draw_chip(ui: &egui::Ui, label: &str, colors: &Colors, x: f32, center_y: f32) {
    let text_color = super::foreground::Surface::over(colors.bg_sidebar, colors.bg_active)
        .foreground(
            colors.text_on(colors.bg_active),
            super::foreground::Role::SmallText,
        );
    let galley = chip_galley(ui, label, text_color);
    let size = chip_size(ui, label);
    let rect = egui::Rect::from_min_size(Pos2::new(x, center_y - size.y / 2.0), size);
    ui.painter()
        .rect_filled(rect, crate::ui::style::RADIUS_SMALL, colors.bg_active);
    crate::ui::snap::galley_snapped(
        ui.painter(),
        Pos2::new(
            rect.center().x - galley.size().x / 2.0,
            rect.center().y - galley.size().y / 2.0 + style::LIST_CHIP_TEXT_OFFSET_Y,
        ),
        galley,
        text_color,
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

fn draw_chip_row(
    ui: &egui::Ui,
    labels: &[&str],
    colors: &Colors,
    left: f32,
    right: f32,
    center_y: f32,
) {
    let budget = (right - left).max(0.0);
    let mut visible = labels.len();
    loop {
        let overflow = format!("+{}", labels.len() - visible);
        let mut shown = labels[..visible].to_vec();
        if visible < labels.len() {
            shown.push(&overflow);
        }
        let width = metadata_chip_row_width(ui, &shown);
        if width <= budget {
            let mut x = right - width;
            for label in shown {
                draw_chip(ui, label, colors, x, center_y);
                x += chip_size(ui, label).x + style::SPACE_XS;
            }
            break;
        }
        if visible == 0 {
            break;
        }
        visible -= 1;
    }
}

const PANE_PIP_RADIUS: f32 = 3.0;
const PANE_PIP_SPACING: f32 = 14.0;
const PANE_PIP_MAX: usize = 8;
const COMPACT_METADATA_CHIP_OFFSET_Y: f32 = 10.0;
const COMPACT_METADATA_PIP_OFFSET_Y: f32 = 10.0;

fn pane_pips_width(ui: &egui::Ui, count: usize) -> f32 {
    if count == 0 {
        return 0.0;
    }
    let capped = count.min(PANE_PIP_MAX);
    let base = (capped.saturating_sub(1) as f32) * PANE_PIP_SPACING + PANE_PIP_RADIUS * 2.0;
    if count > PANE_PIP_MAX {
        base + style::SPACE_XS
            + ui.fonts_mut(|f| {
                f.layout_no_wrap(
                    format!("+{}", count - capped),
                    egui::FontId::proportional(style::TEXT_HINT),
                    Color32::PLACEHOLDER,
                )
            })
            .size()
            .x
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
    let budget = (lane_right - lane_left).max(0.0);
    let mut capped = pips.count.min(PANE_PIP_MAX);
    let overflow_width = |count: usize| {
        if count == pips.count {
            return 0.0;
        }
        ui.fonts_mut(|f| {
            f.layout_no_wrap(
                format!("+{}", pips.count - count),
                egui::FontId::proportional(style::TEXT_HINT),
                Color32::PLACEHOLDER,
            )
        })
        .size()
        .x + style::SPACE_XS
    };
    let dots_width = |count: usize| {
        if count == 0 {
            0.0
        } else {
            (count - 1) as f32 * PANE_PIP_SPACING + PANE_PIP_RADIUS * 2.0
        }
    };
    while capped > 0 && dots_width(capped) + overflow_width(capped) > budget {
        capped -= 1;
    }
    let width = dots_width(capped) + overflow_width(capped);
    if width > budget {
        return;
    }
    let start_x = lane_right - width;
    for dot_i in 0..capped {
        let center = Pos2::new(
            start_x + PANE_PIP_RADIUS + dot_i as f32 * PANE_PIP_SPACING,
            center_y,
        );
        let agent_state = pips.activities.get(dot_i).and_then(|s| s.as_ref());
        let color = crate::ui::activity::pip_color(
            agent_state,
            pips.focused_idx == Some(dot_i),
            colors,
            time,
            dot_i,
        );
        let glyph_state = agent_state.filter(|_| !pips.static_health_pips.contains(&dot_i));
        crate::ui::activity::paint_pip(
            ui.painter(),
            center,
            PANE_PIP_RADIUS,
            glyph_state,
            color,
        );
    }
    if pips.count > capped {
        let overflow_x = start_x + dots_width(capped) + style::SPACE_XS;
        let preferred = if pips.focused_idx.is_some_and(|idx| idx >= capped) {
            colors.accent
        } else {
            colors.text_dim
        };
        let overflow_color =
            super::foreground::Surface::over(colors.bg_sidebar, Color32::TRANSPARENT)
                .foreground(preferred, super::foreground::Role::SmallText);
        crate::ui::snap::text_snapped(
            ui.painter(),
            Pos2::new(overflow_x, center_y),
            Align2::LEFT_CENTER,
            format!("+{}", pips.count - capped),
            egui::FontId::proportional(style::TEXT_HINT),
            overflow_color,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_color_paints_the_primary_line_in_the_requested_state_colour() {
        let host = crate::testing::HostHarness::new();
        let colors = host.app.colors;
        let paint = |body: Option<Color32>| {
            let mut output = None;
            for frame in 0..2 {
                output = Some(host.app.ctx.run_ui(
                    egui::RawInput {
                        time: Some(frame as f64),
                        ..Default::default()
                    },
                    |ui| {
                        egui::CentralPanel::default().show_inside(ui, |ui| {
                            let mut row = ListRow::new("Planner state").dense();
                            if let Some(color) = body {
                                row = row.body_color(color);
                            }
                            row.show(ui, &colors);
                        });
                    },
                ));
            }
            output
                .unwrap()
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.job.text == "Planner state" => {
                        Some(t.override_text_color.unwrap_or(t.fallback_color))
                    }
                    _ => None,
                })
                .expect("row body")
        };
        let plain = paint(None);
        let stated = paint(Some(colors.danger));
        assert_ne!(plain, stated, "state colour must reach the painted body");
        let surface = crate::ui::foreground::Surface::over(colors.bg_sidebar, colors.bg_sidebar);
        assert_eq!(
            stated,
            surface.foreground(colors.danger, crate::ui::foreground::Role::SmallText),
            "state colour is contrast-corrected against the row surface"
        );
    }

    fn collect_chip_shapes(
        shape: &egui::Shape,
        fill: Color32,
        rects: &mut Vec<egui::Rect>,
        texts: &mut Vec<(Pos2, std::sync::Arc<egui::Galley>)>,
    ) {
        match shape {
            egui::Shape::Rect(rect) if rect.fill == fill => rects.push(rect.rect),
            egui::Shape::Text(text) => texts.push((text.pos, text.galley.clone())),
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect_chip_shapes(shape, fill, rects, texts);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn adoption_reflowed_action_activates_row_through_pointer() {
        let ctx = egui::Context::default();
        crate::ui::theme::setup_fonts(&ctx);
        let colors = Colors::from_config(&crate::ui::theme::preset_colors("plexi-dusk").unwrap());
        let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
        let mut hit = egui::Rect::NOTHING;
        let mut clicked = false;
        for stage in 0..3 {
            let mut input = egui::RawInput::default();
            if stage > 0 {
                input.events.push(egui::Event::PointerMoved(hit.center()));
                input.events.push(egui::Event::PointerButton {
                    pos: hit.center(),
                    button: egui::PointerButton::Primary,
                    pressed: stage == 1,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            let _ = ctx.run_ui(input, |ui| {
                ui.set_width(160.0);
                clicked |= ListRow::new("Release evidence")
                    .trailing_action("Restore and open in a new window")
                    .show(ui, &colors)
                    .row_clicked();
            });
            let reports = super::super::layout_report::snapshot(&ctx);
            hit = reports
                .iter()
                .find(|r| r.surface == "list")
                .unwrap()
                .actions[0]
                .hit;
        }
        assert!(
            clicked,
            "the reflowed trailing action must activate its row"
        );
    }

    #[test]
    fn adoption_long_list_action_and_chip_fit() {
        let ctx = egui::Context::default();
        crate::ui::theme::setup_fonts(&ctx);
        let colors = crate::ui::theme::Colors::from_config(
            &crate::ui::theme::preset_colors("plexi-dusk").unwrap(),
        );
        let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(160.0);
            ui.set_clip_rect(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(180.0, 800.0),
            ));
            let capture =
                crate::ui::layout_report::Capture::begin(ui.painter(), "adoption-fixture");
            let result = ListRow::new("A long identifier with distinguishing suffix")
                .secondary("/an/unbroken/path/with/a/long/detail")
                .chip("an exceptionally long annotation that must collapse")
                .trailing_action("Wiederherstellen und in einem neuen Fenster öffnen")
                .show(ui, &colors);
            let rect = result.row.rect;
            capture.finish(ui.painter(), rect, colors.bg_sidebar, vec![], vec![]);
        });
        let reports = crate::ui::layout_report::snapshot(&ctx);
        assert!(!reports.is_empty());
        let errors: Vec<_> = reports.iter().flat_map(|r| r.violations()).collect();
        assert!(errors.is_empty(), "{errors:?}");
    }
    #[test]
    fn list_chip_text_has_optical_vertical_centering() {
        for ppp in [1.0_f32, 2.0] {
            let ctx = egui::Context::default();
            crate::ui::theme::setup_fonts(&ctx);
            ctx.set_pixels_per_point(ppp);
            let colors = crate::ui::theme::Colors::from_config(
                &crate::ui::theme::preset_colors("catppuccin-mocha").expect("preset"),
            );
            let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                draw_chip(ui, "view", &colors, 12.0, 30.0);
            });

            let mut rects = Vec::new();
            let mut texts = Vec::new();
            for clipped in &output.shapes {
                collect_chip_shapes(&clipped.shape, colors.bg_active, &mut rects, &mut texts);
            }
            let chip = rects
                .into_iter()
                .find(|rect| rect.width() > 10.0 && rect.height() > 10.0)
                .expect("chip background");
            let (text_pos, galley) = texts
                .into_iter()
                .find(|(_, galley)| galley.job.text == "view")
                .expect("chip text");
            let line_box_center = text_pos.y + galley.size().y / 2.0;
            let center_delta = line_box_center - chip.center().y;

            assert!(
                (-1.01..=-0.49).contains(&center_delta),
                "chip text line box should sit visibly above geometric center at ppp={ppp}: \
                 center_delta={center_delta}, line_box_center={line_box_center}, chip_center={}",
                chip.center().y
            );
        }
    }

    #[test]
    fn list_row_builder_keeps_optional_secondary_empty_by_default() {
        let row = ListRow::new("note").secondary("").chip("view");
        assert_eq!(row.body, "note");
        assert!(row.secondary.is_none());
        assert_eq!(row.chip, Some("view"));
        assert!(row.metadata_chips.is_empty());
        assert!(!row.selected);
    }

    #[test]
    fn trailing_metadata_reserves_text_width() {
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            let row_rect =
                egui::Rect::from_min_size(Pos2::ZERO, Vec2::new(240.0, style::LIST_ROW_H));
            let plain = ListRow::new("Assistant").secondary("Long plugin description");
            let metadata = ListRow::new("Assistant")
                .secondary("Long plugin description")
                .metadata_chips(&["view", "ws"]);

            assert!(
                metadata.lanes(ui, row_rect, true).text.right()
                    < plain.lanes(ui, row_rect, true).text.right(),
                "metadata lane must reduce the text area so secondary text truncates before chips"
            );
        });
    }

    #[test]
    fn pane_pips_reserve_the_metadata_lane() {
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            let row_rect =
                egui::Rect::from_min_size(Pos2::ZERO, Vec2::new(240.0, style::LIST_ROW_H));
            let row = ListRow::new("Workspace").pane_pips(ListRowPips {
                count: 3,
                focused_idx: Some(1),
                activities: vec![None, None, None],
                static_health_pips: vec![],
            });

            assert!(row.metadata_lane_width(ui) > 0.0);
            assert!(
                row.lanes(ui, row_rect, true).text.right()
                    < row_rect.right() - style::LIST_ROW_PAD_H
            );
        });
    }

    #[test]
    fn compact_metadata_keeps_pips_padded_below_chip() {
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            let primary_center_y = style::LIST_ROW_H / 2.0;
            let chip_bottom =
                primary_center_y - COMPACT_METADATA_CHIP_OFFSET_Y + chip_size(ui, "ctx").y / 2.0;
            let pip_top = primary_center_y + COMPACT_METADATA_PIP_OFFSET_Y - PANE_PIP_RADIUS;

            assert!(
                pip_top - chip_bottom >= style::SPACE_SM,
                "compact metadata pips need visible padding below the chip"
            );
        });
    }

    /// Drives one frame of a 30-row list with the given selection and reports
    /// whether the selected row's rect ended up fully inside the scroll
    /// viewport. Uses real egui layout — row spacing, section headers, and
    /// scroll state all behave exactly as in the plugin.
    fn run_list_frame(ctx: &egui::Context, selected: usize) -> bool {
        let mut visible = false;
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show_inside(ui, |ui| {
                egui::ScrollArea::vertical()
                    .animated(false)
                    .id_salt("scroll_into_view_test")
                    .max_height(200.0)
                    .show(ui, |ui| {
                        for i in 0..30 {
                            // Mimic the palette's inline section header midway
                            // through the list — non-uniform content is the
                            // case the old index*row_h math got wrong.
                            if i == 10 {
                                ui.label("SECTION");
                            }
                            let (rect, resp) = ui
                                .allocate_exact_size(egui::vec2(120.0, 48.0), egui::Sense::click());
                            scroll_row_into_view(ui, &resp, i == selected);
                            if i == selected {
                                visible = ui.clip_rect().contains_rect(rect);
                            }
                        }
                    });
            });
        });
        visible
    }

    #[test]
    fn scroll_row_into_view_keeps_selection_visible_past_fold() {
        let ctx = egui::Context::default();
        // Walk the selection down past the fold and back up; the selected row
        // must be fully visible on the frame the selection changes (the
        // request_discard re-pass renders at the corrected offset).
        for &selected in &[0usize, 8, 17, 29, 12, 3, 0] {
            assert!(
                run_list_frame(&ctx, selected),
                "row {selected} not fully visible after scroll_row_into_view"
            );
        }
    }

    #[test]
    fn two_line_with_pips_chip_clears_pips_vertically() {
        // For a two-line row (title + subtitle) with pane pips, the pane-type chip
        // must sit fully above the pip dots with no vertical overlap.
        // Chip is placed at primary_center_y; pips at secondary_center_y.
        // Use 1.5x TEXT_* as conservative upper-bound line heights — actual
        // rendered heights are always smaller, so if the bound holds, real
        // layout holds.
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            let primary_line_h = style::TEXT_CAPTION * 1.5;
            let secondary_line_h = style::TEXT_HINT * 1.5;
            let total_h = primary_line_h + 2.0 + secondary_line_h;
            let center_y = style::LIST_ROW_H / 2.0;
            let primary_center_y = center_y - total_h / 2.0 + primary_line_h / 2.0;
            let secondary_center_y =
                center_y - total_h / 2.0 + primary_line_h + 2.0 + secondary_line_h / 2.0;
            let chip_h = chip_size(ui, "ctx").y;
            let chip_bottom = primary_center_y + chip_h / 2.0;
            let pip_top = secondary_center_y - PANE_PIP_RADIUS;

            assert!(
                pip_top >= chip_bottom,
                "two-line row: pip top ({pip_top:.1}) must clear chip bottom ({chip_bottom:.1})"
            );
        });
    }
}
