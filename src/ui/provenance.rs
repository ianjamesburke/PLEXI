//! Pane-header provenance icon: who spawned the pane, as a glyph + tooltip.
//!
//! The GUI header paints no text for provenance; the badge string
//! (`format_spawned_by_badge`) stays the CLI/status API and is mapped here to
//! an icon kind and a hover tooltip.

use egui::{Color32, CornerRadius, Painter, Pos2, Rect, Stroke, Vec2};

use crate::host::pane::SPAWNED_BY_HUMAN_BADGE;
use crate::ui::style;

const SYSTEM_BADGE: &str = "spawned by system";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProvenanceKind {
    Human,
    Agent,
    System,
}

/// Icon kind for a provenance badge string.
pub(crate) fn provenance_kind(badge: &str) -> ProvenanceKind {
    if badge == SPAWNED_BY_HUMAN_BADGE {
        ProvenanceKind::Human
    } else if badge == SYSTEM_BADGE {
        ProvenanceKind::System
    } else {
        ProvenanceKind::Agent
    }
}

/// Hover tooltip for a provenance badge string: `"Spawned by you"`,
/// `"Spawned by <agent>"`, `"Spawned by system"`.
pub(crate) fn provenance_tooltip(badge: &str) -> String {
    let mut chars = badge.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Square icon rect, vertically centered in `bar_rect`, whose right edge is at `right`.
pub(crate) fn provenance_icon_rect(bar_rect: Rect, right: f32) -> Rect {
    let size = Vec2::splat(style::PROVENANCE_ICON_SIZE);
    Rect::from_min_size(
        Pos2::new(right - size.x, bar_rect.center().y - size.y / 2.0),
        size,
    )
}

/// Paint the glyph for `kind` filling `rect` (user bust, robot head, or gear-less
/// system square).
pub(crate) fn paint_provenance_icon(
    painter: &Painter,
    rect: Rect,
    kind: ProvenanceKind,
    color: Color32,
) {
    let s = rect.width().min(rect.height());
    let stroke = Stroke::new((s * 0.1).max(1.0), color);
    match kind {
        ProvenanceKind::Human => {
            painter.circle_filled(
                Pos2::new(rect.center().x, rect.min.y + s * 0.3),
                s * 0.22,
                color,
            );
            let body = Rect::from_min_max(
                Pos2::new(rect.center().x - s * 0.36, rect.min.y + s * 0.62),
                Pos2::new(rect.center().x + s * 0.36, rect.max.y - s * 0.04),
            );
            let r = (s * 0.3) as u8;
            painter.rect_filled(
                body,
                CornerRadius {
                    nw: r,
                    ne: r,
                    sw: 1,
                    se: 1,
                },
                color,
            );
        }
        ProvenanceKind::Agent => {
            let head = Rect::from_min_max(
                Pos2::new(rect.min.x + s * 0.1, rect.min.y + s * 0.3),
                Pos2::new(rect.max.x - s * 0.1, rect.max.y - s * 0.08),
            );
            painter.rect_stroke(
                head,
                CornerRadius::same((s * 0.2) as u8),
                stroke,
                egui::StrokeKind::Inside,
            );
            let eye_y = head.center().y;
            for dx in [-0.2_f32, 0.2] {
                painter.circle_filled(Pos2::new(rect.center().x + dx * s, eye_y), s * 0.07, color);
            }
            let top = Pos2::new(rect.center().x, rect.min.y + s * 0.1);
            painter.line_segment([top, Pos2::new(top.x, head.min.y)], stroke);
            painter.circle_filled(top, s * 0.08, color);
        }
        ProvenanceKind::System => {
            painter.rect_stroke(
                rect.shrink(s * 0.12),
                CornerRadius::same((s * 0.15) as u8),
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.circle_filled(rect.center(), s * 0.12, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tooltip_maps_badge_strings() {
        assert_eq!(provenance_tooltip(SPAWNED_BY_HUMAN_BADGE), "Spawned by you");
        assert_eq!(provenance_tooltip("spawned by researcher"), "Spawned by researcher");
        assert_eq!(provenance_tooltip("spawned by system"), "Spawned by system");
        assert_eq!(provenance_kind(SPAWNED_BY_HUMAN_BADGE), ProvenanceKind::Human);
        assert_eq!(provenance_kind("spawned by researcher"), ProvenanceKind::Agent);
        assert_eq!(provenance_kind("spawned by system"), ProvenanceKind::System);
    }

    #[test]
    fn icon_paints_shapes_but_no_text() {
        for kind in [ProvenanceKind::Human, ProvenanceKind::Agent, ProvenanceKind::System] {
            let ctx = egui::Context::default();
            let out = ctx.run_ui(Default::default(), |ui| {
                let rect = provenance_icon_rect(
                    Rect::from_min_size(Pos2::ZERO, Vec2::new(200.0, 24.0)),
                    100.0,
                );
                paint_provenance_icon(ui.painter(), rect, kind, Color32::WHITE);
            });
            let shapes: Vec<_> = out.shapes.iter().map(|s| &s.shape).collect();
            assert!(!shapes.is_empty());
            assert!(
                shapes.iter().all(|s| !matches!(s, egui::Shape::Text(_))),
                "{kind:?} icon must not paint text"
            );
        }
    }
}
