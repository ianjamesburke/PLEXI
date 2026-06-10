use egui::{Align2, Color32, CornerRadius, Pos2, Response, Stroke, StrokeKind, Vec2};

use crate::ui::style;
use crate::ui::theme::Colors;

pub struct ListRow<'a> {
    body: &'a str,
    secondary: Option<&'a str>,
    leading_chip: Option<&'a str>,
    trailing: Option<&'a str>,
    selected: bool,
    danger_trailing: bool,
}

impl<'a> ListRow<'a> {
    pub fn new(body: &'a str) -> Self {
        Self {
            body,
            secondary: None,
            leading_chip: None,
            trailing: None,
            selected: false,
            danger_trailing: false,
        }
    }

    pub fn secondary(mut self, secondary: &'a str) -> Self {
        self.secondary = (!secondary.is_empty()).then_some(secondary);
        self
    }

    pub fn leading_chip(mut self, label: &'a str) -> Self {
        self.leading_chip = Some(label);
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
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), style::LIST_ROW_H),
            egui::Sense::click(),
        );
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        let fill = if self.selected {
            colors.bg_active
        } else if response.hovered() {
            colors.bg_hover
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, style::RADIUS_MD, fill);

        let mut x = rect.left() + style::LIST_ROW_PAD_H;
        let center_y = rect.center().y;

        if let Some(label) = self.leading_chip {
            let chip = chip_rect(ui, label, colors, x, center_y);
            x = chip.right() + style::LIST_ROW_GAP;
        }

        let trailing_response = self.trailing.map(|label| {
            let size = trailing_size(ui, label);
            let trailing_rect = egui::Rect::from_center_size(
                Pos2::new(
                    rect.right() - style::LIST_ROW_PAD_H - size.x / 2.0,
                    center_y,
                ),
                size,
            );
            let id = response.id.with("_trailing_action");
            let trailing_response = ui.interact(trailing_rect, id, egui::Sense::click());
            let color = if self.danger_trailing && trailing_response.hovered() {
                colors.danger
            } else {
                colors.text_dim
            };
            ui.painter().text(
                trailing_rect.center(),
                Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional(style::TEXT_BODY),
                color,
            );
            trailing_response
        });

        let text_right = self
            .trailing
            .map(|label| {
                rect.right()
                    - style::LIST_ROW_PAD_H
                    - trailing_size(ui, label).x
                    - style::LIST_ROW_GAP
            })
            .unwrap_or_else(|| rect.right() - style::LIST_ROW_PAD_H);
        let max_text_width = (text_right - x).max(0.0);
        draw_text_block(ui, &self, colors, x, center_y, max_text_width);

        ListRowResponse {
            row: response,
            trailing: trailing_response,
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

fn draw_text_block(
    ui: &mut egui::Ui,
    row: &ListRow<'_>,
    colors: &Colors,
    x: f32,
    center_y: f32,
    max_width: f32,
) {
    let primary_color = if row.selected {
        colors.text_primary
    } else {
        colors.text_dim
    };
    let primary_font = egui::FontId::proportional(style::TEXT_HINT);
    let primary = elided_galley(ui, row.body, primary_font, primary_color, max_width);

    if let Some(secondary) = row.secondary {
        let secondary_font = egui::FontId::proportional(style::TEXT_HINT);
        let secondary_galley =
            elided_galley(ui, secondary, secondary_font, colors.text_dim, max_width);
        let total_h = primary.size().y + 2.0 + secondary_galley.size().y;
        let primary_pos = Pos2::new(x, center_y - total_h / 2.0);
        let secondary_pos = Pos2::new(x, primary_pos.y + primary.size().y + 2.0);
        ui.painter().galley(primary_pos, primary, primary_color);
        ui.painter()
            .galley(secondary_pos, secondary_galley, colors.text_dim);
    } else {
        let pos = Pos2::new(x, center_y - primary.size().y / 2.0);
        ui.painter().galley(pos, primary, primary_color);
    }
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

fn trailing_size(ui: &egui::Ui, label: &str) -> Vec2 {
    let galley = ui.fonts(|f| {
        f.layout_no_wrap(
            label.to_string(),
            egui::FontId::proportional(style::TEXT_BODY),
            Color32::WHITE,
        )
    });
    Vec2::new(galley.size().x.max(24.0), style::LIST_ROW_H)
}

fn chip_rect(ui: &egui::Ui, label: &str, colors: &Colors, x: f32, center_y: f32) -> egui::Rect {
    let galley = ui.fonts(|f| {
        f.layout_no_wrap(
            label.to_string(),
            egui::FontId::proportional(style::TEXT_HINT),
            colors.text_primary,
        )
    });
    let size = Vec2::new(
        galley.size().x + style::SPACE_SM,
        galley.size().y + style::SPACE_XS,
    );
    let rect = egui::Rect::from_min_size(Pos2::new(x, center_y - size.y / 2.0), size);
    ui.painter()
        .rect_filled(rect, CornerRadius::same(4), colors.bg_active);
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(4),
        Stroke::new(1.0, colors.border),
        StrokeKind::Inside,
    );
    ui.painter().galley(
        Pos2::new(
            rect.center().x - galley.size().x / 2.0,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        colors.text_primary,
    );
    rect
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_row_builder_keeps_optional_secondary_empty_by_default() {
        let row = ListRow::new("note").secondary("").leading_chip("app");
        assert_eq!(row.body, "note");
        assert!(row.secondary.is_none());
        assert_eq!(row.leading_chip, Some("app"));
        assert!(!row.selected);
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
