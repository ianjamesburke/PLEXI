//! The badge pill primitive.
//!
//! One owner of badge geometry — padding, radius, text size, and the
//! width floor that keeps a one- or two-character pill from collapsing
//! into a stub. Every badge the host paints goes through here: the WIT
//! `Badge` node, the sidebar notification count, and the terminal
//! outside-workspace tag. The tokens live in [`style`] and are shared
//! with the Python SDK so both sides agree on pill size.

use std::sync::Arc;

use egui::{Color32, Galley, Pos2, Rect, Vec2};

use crate::ui::style;
use crate::ui::theme::Colors;

/// Font every badge label is laid out with.
pub(crate) fn badge_font() -> egui::FontId {
    egui::FontId::proportional(style::TEXT_META)
}

/// Pill size for an already-laid-out label. The width floors at the pill
/// height so a single glyph stays round rather than reading as a sliver.
pub(crate) fn badge_size(text_size: Vec2) -> Vec2 {
    let h = text_size.y + style::BADGE_PAD_V * 2.0;
    Vec2::new((text_size.x + style::BADGE_PAD_H * 2.0).max(h), h)
}

/// Lay out a badge label. Pass [`Color32::PLACEHOLDER`] when the galley is
/// only being measured, or when the paint pass supplies the final color.
pub(crate) fn badge_galley(ui: &egui::Ui, text: &str, color: Color32) -> Arc<Galley> {
    ui.fonts_mut(|f| f.layout_no_wrap(text.to_string(), badge_font(), color))
}

/// Paint a pill of `fill` inside `rect` with `galley` centered in it.
pub(crate) fn paint_badge(
    painter: &egui::Painter,
    rect: Rect,
    galley: Arc<Galley>,
    fill: Color32,
    fg: Color32,
) {
    painter.rect_filled(rect, style::RADIUS_BADGE, fill);
    let text_size = galley.size();
    crate::ui::snap::galley_snapped(
        painter,
        Pos2::new(
            rect.center().x - text_size.x / 2.0,
            rect.center().y - text_size.y / 2.0,
        ),
        galley,
        fg,
    );
}

/// Allocate and paint a badge inline in the current layout. The label color
/// is whichever of the theme's foregrounds reads legibly on `fill`.
pub(crate) fn badge(ui: &mut egui::Ui, text: &str, fill: Color32, colors: &Colors) -> egui::Response {
    let fg = colors.text_on(fill);
    let galley = badge_galley(ui, text, fg);
    let size = badge_size(galley.size());
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    paint_badge(ui.painter(), rect, galley, fill, fg);
    response
}
