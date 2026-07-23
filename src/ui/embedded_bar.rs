//! Embedded pane-chrome bars (find/replace, status strips) pinned to the
//! bottom edge of a pane's content area.
//!
//! Panes lay out their content by hand (raw rects, not egui panels), so a bar
//! flush against the pane's bottom edge has no safe area and its controls'
//! bottoms get clipped by the pane clip rect. This primitive owns that
//! geometry: the caller reserves [`BAR_TOTAL_H`] at the bottom of its content
//! and hands the band to [`embedded_bottom_bar`], which fills the background
//! and runs the content inside a horizontally-inset, vertically-centered child
//! ui. Callers never hand-place padding, and the safe insets guarantee the
//! controls clear the pane edge.

use crate::ui::style;
use egui::Ui;

/// Vertical padding above and below the bar's interactive content. The bottom
/// copy is the safe area that keeps controls off the pane's clip edge.
const BAR_INSET_V: f32 = style::SPACE_XS;

/// Horizontal padding at each end of the bar.
const BAR_INSET_H: f32 = style::SPACE_SM;

/// Interactive content height — comfortably fits a standard small form button
/// and single-line input without clipping.
pub const BAR_CONTENT_H: f32 = 26.0;

/// Total height a caller reserves for the bar band (content + top and bottom
/// safe insets).
pub const BAR_TOTAL_H: f32 = BAR_CONTENT_H + 2.0 * BAR_INSET_V;

/// Render a chrome bar filling `rect` (a band of height [`BAR_TOTAL_H`] at the
/// bottom of a pane). Paints `fill`, then lays out `content` inside a child ui
/// inset by the safe margins and vertically centered, so its widgets can never
/// be clipped by the pane's bottom edge.
pub fn embedded_bottom_bar(
    ui: &mut Ui,
    rect: egui::Rect,
    fill: egui::Color32,
    content: impl FnOnce(&mut Ui),
) {
    ui.painter().rect_filled(rect, 0.0, fill);
    let inner = rect.shrink2(egui::vec2(BAR_INSET_H, BAR_INSET_V));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner));
    child.horizontal_centered(content);
}
