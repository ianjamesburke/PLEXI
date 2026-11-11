//! Pane header geometry shared by terminal, app, portal and tabbed callers.
//! The leading state glyph is authoritative: a narrow header never adds a
//! second copy. Provenance becomes a named icon before shrinking the title.

use super::{badge, bounded, style, theme::Colors};
use egui::{Color32, Galley, Rect};
use std::sync::Arc;

pub(crate) const OUTSIDE_LABEL: &str = "Outside workspace";
const OUTSIDE_TEXT: &str = "outside workspace";

pub(crate) struct Labels<'a> {
    pub outside: bool,
    pub waiting: Option<&'a str>,
    pub provenance: Option<&'a str>,
    pub state: Option<(&'a str, Color32)>,
}

struct Item {
    id: &'static str,
    rect: Rect,
    galley: Option<Arc<Galley>>,
    color: Color32,
}

/// Chip width for a label of `text_width` (0 for the arrow-only compact form): equal side
/// padding, and the fixed arrow gap only when there is text.
fn outside_chip_width(text_width: f32) -> f32 {
    let gap = if text_width > 0.0 { style::CHIP_ICON_GAP } else { 0.0 };
    (2.0 * style::CHIP_PAD_H + style::CHIP_ARROW + gap + text_width).max(style::PANE_HEADER_CHIP_HEIGHT)
}

/// Painted north-east arrow filling a `CHIP_ARROW` square centered on `center`.
fn paint_arrow(painter: &egui::Painter, center: egui::Pos2, color: Color32) {
    let half = style::CHIP_ARROW / 2.0;
    let stroke = egui::Stroke::new(1.2, color);
    let tail = center + egui::vec2(-half, half);
    let tip = center + egui::vec2(half, -half);
    painter.line_segment([tail, tip], stroke);
    painter.line_segment([tip, tip + egui::vec2(-half * 1.2, 0.0)], stroke);
    painter.line_segment([tip, tip + egui::vec2(0.0, half * 1.2)], stroke);
}

/// Text and arrow are centered on the glyph ink (not the line box), so the arrow reads as
/// aligned with the words. Arrow-and-text content is centered in the chip, so left and right padding are equal.
fn paint_outside_chip(ui: &mut egui::Ui, item: &Item, backdrop: Color32) {
    let fill = Color32::from_rgba_unmultiplied(item.color.r(), item.color.g(), item.color.b(), 28);
    let fg = super::foreground::Surface::over(backdrop, fill).foreground(item.color, super::foreground::Role::SmallText);
    let rect = item.rect;
    ui.painter().rect_filled(rect, style::RADIUS_BADGE, fill);
    let text_width = item.galley.as_ref().map_or(0.0, |g| g.size().x);
    let gap = if text_width > 0.0 { style::CHIP_ICON_GAP } else { 0.0 };
    let content = style::CHIP_ARROW + gap + text_width;
    let left = rect.center().x - content / 2.0;
    paint_arrow(ui.painter(), egui::pos2(left + style::CHIP_ARROW / 2.0, rect.center().y), fg);
    if let Some(galley) = item.galley.as_ref().filter(|g| !g.is_empty()) {
        let pos = egui::pos2(left + style::CHIP_ARROW + gap, rect.center().y - galley.mesh_bounds.center().y);
        ui.painter().add(egui::epaint::TextShape::new(pos, galley.clone(), fg));
    }
}

pub(crate) struct PaneHeader {
    pub text: Rect,
    pub outside: Option<Rect>,
    items: Vec<Item>,
}

impl PaneHeader {
    pub fn measure(
        ui: &egui::Ui,
        bar: Rect,
        leading: f32,
        labels: &Labels<'_>,
        colors: &Colors,
    ) -> Self {
        let right = (bar.right() - style::PANE_CONTENT_INSET).max(bar.left());
        let bounds = Rect::from_min_max(
            egui::pos2(
                (bar.left() + leading + style::SPACE_SM).min(right),
                style::pane_header_band(bar).top(),
            ),
            egui::pos2(right, bar.bottom()),
        );
        let make = |text: &str| {
            ui.painter()
                .layout_no_wrap(text.into(), badge::badge_font(), Color32::PLACEHOLDER)
        };
        let outside = labels.outside.then(|| make(OUTSIDE_TEXT));
        let compact = labels.outside.then(|| make(""));
        let state = labels.state.map(|(label, _)| make(label));
        let waiting = labels.waiting.map(make);
        let candidates = [
            ("outside", outside, colors.zoom_needs),
            (
                "state",
                state,
                labels.state.map_or(colors.text_dim, |(_, color)| color),
            ),
            ("waiting", waiting, colors.status_badge_color(colors.accent)),
            (
                "provenance",
                None,
                colors.text_secondary(colors.pane_header_bg()),
            ),
        ];
        let slots: Vec<_> = candidates
            .iter()
            .filter_map(|(id, galley, _)| {
                let width = galley
                    .as_ref()
                    .map(|g| if *id == "outside" { outside_chip_width(g.size().x) } else { badge::badge_size(g.size()).x })
                    .or_else(|| {
                        (*id == "provenance" && labels.provenance.is_some())
                            .then_some(style::PROVENANCE_ICON_SIZE)
                    })?;
                Some(bounded::Slot {
                    id,
                    width,
                    compact: compact
                        .as_ref()
                        .filter(|_| *id == "outside")
                        .map(|_| outside_chip_width(0.0)),
                    required: false,
                })
            })
            .collect();
        let Ok(row) = bounded::allocate(
            bounds,
            style::PANE_HEADER_TITLE_MIN,
            style::SPACE_XS,
            &slots,
        ) else {
            log::info!("ui: insufficient pane header space");
            return Self {
                text: Rect::from_min_size(bar.min, egui::Vec2::ZERO),
                outside: None,
                items: Vec::new(),
            };
        };
        let mut items = Vec::new();
        let mut outside_rect = None;
        for slot in row.slots {
            if slot.presentation == bounded::Presentation::Hidden {
                continue;
            }
            let Some((_, galley, color)) = candidates.iter().find(|(id, _, _)| *id == slot.id)
            else {
                continue;
            };
            let galley = if slot.presentation == bounded::Presentation::Compact {
                compact.clone()
            } else {
                galley.clone()
            };
            let height = style::PANE_HEADER_CHIP_HEIGHT;
            let Some(rect) = bounded::centered_text(
                slot.rect,
                slot.rect.center().x,
                egui::vec2(slot.rect.width(), height),
            ) else {
                continue;
            };
            let rect = if galley.is_none() {
                super::provenance::provenance_icon_rect(rect, rect.right())
            } else {
                rect
            };
            if slot.id == "outside" {
                outside_rect = Some(rect);
            }
            items.push(Item {
                id: slot.id,
                rect,
                galley,
                color: *color,
            });
        }
        Self {
            text: row.text,
            outside: outside_rect,
            items,
        }
    }

    pub fn record_slots(&self, capture: &mut super::layout_report::Capture) {
        for item in &self.items {
            if let Some(galley) = item.galley.as_ref().filter(|g| !g.is_empty()) {
                capture.text_slot(item.id, galley, item.rect);
            }
        }
    }

    pub fn paint(&self, ui: &mut egui::Ui, labels: &Labels<'_>, pane_id: u64, backdrop: Color32) {
        for item in &self.items {
            if item.id == "outside" {
                paint_outside_chip(ui, item, backdrop);
                ui.interact(
                    item.rect,
                    egui::Id::new(("outside", pane_id)),
                    egui::Sense::hover(),
                )
                .on_hover_text(OUTSIDE_LABEL);
            } else if let Some(galley) = &item.galley {
                let alpha = 36;
                badge::paint_badge(
                    ui.painter(),
                    item.rect,
                    galley.clone(),
                    Color32::from_rgba_unmultiplied(
                        item.color.r(),
                        item.color.g(),
                        item.color.b(),
                        alpha,
                    ),
                    item.color,
                    backdrop,
                );
            } else if let Some(label) = labels.provenance {
                super::provenance::paint_provenance_icon(
                    ui.painter(),
                    item.rect,
                    super::provenance::provenance_kind(label),
                    item.color,
                );
                ui.ctx().data_mut(|d| {
                    d.insert_temp(egui::Id::new(("provenance_icon_rect", pane_id)), item.rect)
                });
                ui.interact(
                    item.rect,
                    egui::Id::new(("provenance_icon", pane_id)),
                    egui::Sense::hover(),
                )
                .on_hover_text(super::provenance::provenance_tooltip(label));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_header_collapses_provenance_before_title_and_never_duplicates_state() {
        let ctx = egui::Context::default();
        let colors = Colors::from_config(&crate::config::ThemeConfig::default());
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            for width in [0.0, 30.0, 80.0, 160.0, 240.0, 400.0] {
                let bar = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 24.0));
                let layout = PaneHeader::measure(
                    ui,
                    bar,
                    24.0,
                    &Labels {
                        outside: true,
                        waiting: Some("3 results"),
                        provenance: Some("human"),
                        state: Some(("NEEDS YOU", colors.state_needs_you)),
                    },
                    &colors,
                );
                assert!(bar.contains_rect(layout.text));
                for item in &layout.items {
                    assert!(bar.contains_rect(item.rect));
                }
                if width == 160.0 {
                    assert!(layout.text.width() >= style::PANE_HEADER_TITLE_MIN);
                    assert_eq!(layout.items[0].id, "outside");
                    assert_eq!(layout.items[0].galley.as_ref().unwrap().text(), "", "compact outside chip is arrow-only");
                    assert!(!layout.items.iter().any(|item| item.id == "state"));
                }
            }
        });
    }
}
