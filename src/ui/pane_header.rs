//! Pane header geometry shared by terminal, app, portal and tabbed callers.
//! The leading state glyph is authoritative: a narrow header never adds a
//! second copy. Provenance becomes a named icon before shrinking the title.

use super::{badge, bounded, style, theme::Colors};
use egui::{Color32, Galley, Rect};
use std::sync::Arc;

pub(crate) const OUTSIDE_LABEL: &str = "↗ outside workspace";

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
        let bounds = Rect::from_min_max(
            egui::pos2(
                (bar.left() + leading + style::SPACE_SM).min(bar.right()),
                bar.top(),
            ),
            bar.max,
        );
        let make = |text: &str| {
            ui.painter()
                .layout_no_wrap(text.into(), badge::badge_font(), Color32::PLACEHOLDER)
        };
        let outside = labels.outside.then(|| make(OUTSIDE_LABEL));
        let compact = labels.outside.then(|| make("↗"));
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
                    .map(|g| badge::badge_size(g.size()).x)
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
                        .map(|g| badge::badge_size(g.size()).x),
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
            let height = galley.as_ref().map_or(style::PROVENANCE_ICON_SIZE, |g| {
                badge::badge_size(g.size()).y
            });
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
            if let Some(galley) = &item.galley {
                capture.text_slot(item.id, galley, item.rect);
            }
        }
    }

    pub fn paint(&self, ui: &mut egui::Ui, labels: &Labels<'_>, pane_id: u64, backdrop: Color32) {
        for item in &self.items {
            if let Some(galley) = &item.galley {
                let alpha = if item.id == "outside" { 28 } else { 36 };
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
                if item.id == "outside" {
                    ui.interact(
                        item.rect,
                        egui::Id::new(("outside", pane_id)),
                        egui::Sense::hover(),
                    )
                    .on_hover_text(OUTSIDE_LABEL);
                }
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
                    assert_eq!(layout.items[0].galley.as_ref().unwrap().text(), "↗");
                    assert!(!layout.items.iter().any(|item| item.id == "state"));
                }
            }
        });
    }
}
