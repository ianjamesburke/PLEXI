//! Measured text elision.
//!
//! One algorithm and one ellipsis for every truncated label the host paints,
//! so a path in the sidebar, a path in the file browser, and a title in a
//! picker all shorten the same way. Every candidate is measured against the
//! real font — never a character count, which ignores both the font and the
//! width actually available.

use std::sync::Arc;

use egui::{Color32, FontId, Galley};

/// The one ellipsis the host elides with: U+2026, not three ASCII dots.
pub(crate) const ELLIPSIS: char = '\u{2026}';

/// Which end of the string is dropped when it does not fit.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Side {
    /// Drop the head — `…/plexi/src`. For paths, where the leaf identifies it.
    Leading,
    /// Drop the tail — `a very long note…`. For titles, which read front-first.
    Trailing,
}

/// Shorten `text` until it fits `max_width`, dropping characters from `side`
/// and marking the cut with [`ELLIPSIS`]. Returns the text unchanged when it
/// already fits, and an empty string when not even the ellipsis does.
pub(crate) fn elide(
    ui: &egui::Ui,
    text: &str,
    font_id: FontId,
    max_width: f32,
    side: Side,
) -> String {
    if max_width <= 0.0 {
        return String::new();
    }
    let width = |s: &str| {
        ui.fonts_mut(|f| {
            f.layout_no_wrap(s.to_string(), font_id.clone(), Color32::PLACEHOLDER)
                .size()
                .x
        })
    };
    if width(text) <= max_width {
        return text.to_string();
    }
    let ellipsis_only = ELLIPSIS.to_string();
    if width(&ellipsis_only) > max_width {
        return String::new();
    }

    // Grow the kept run one character at a time and stop at the first
    // candidate that overflows; the previous one is the widest that fits.
    let chars: Vec<char> = text.chars().collect();
    let mut best = ellipsis_only;
    for keep in 1..chars.len() {
        let candidate: String = match side {
            Side::Leading => std::iter::once(ELLIPSIS)
                .chain(chars[chars.len() - keep..].iter().copied())
                .collect(),
            Side::Trailing => chars[..keep]
                .iter()
                .copied()
                .chain(std::iter::once(ELLIPSIS))
                .collect(),
        };
        if width(&candidate) > max_width {
            break;
        }
        best = candidate;
    }
    best
}

/// Lay out an elided label. The galley carries [`Color32::PLACEHOLDER`], which
/// defers the color to paint time — measurement never has to know which of a
/// row's alpha-modulated tones the text will end up in.
pub(crate) fn elided_galley(
    ui: &egui::Ui,
    text: &str,
    font_id: FontId,
    max_width: f32,
    side: Side,
) -> Arc<Galley> {
    let text = elide(ui, text, font_id.clone(), max_width, side);
    ui.fonts_mut(|f| f.layout_no_wrap(text, font_id, Color32::PLACEHOLDER))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::style;

    fn with_ui(mut f: impl FnMut(&egui::Ui)) {
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| f(ui));
    }

    #[test]
    fn elided_text_fits_the_budget_on_one_line() {
        with_ui(|ui| {
            let font = FontId::proportional(style::TEXT_HINT);
            for side in [Side::Leading, Side::Trailing] {
                let out = elide(ui, "a very long note filename.md", font.clone(), 40.0, side);
                let galley =
                    ui.fonts_mut(|f| f.layout_no_wrap(out, font.clone(), Color32::PLACEHOLDER));
                assert!(galley.size().x <= 40.0);
                assert_eq!(galley.rows.len(), 1);
            }
        });
    }

    #[test]
    fn each_side_keeps_the_end_it_names() {
        with_ui(|ui| {
            let font = FontId::proportional(style::TEXT_HINT);
            let path = "/Users/someone/Documents/GitHub/plexi/src";
            let leading = elide(ui, path, font.clone(), 120.0, Side::Leading);
            let trailing = elide(ui, path, font.clone(), 120.0, Side::Trailing);
            assert!(leading.starts_with(ELLIPSIS), "leading elision marks the head");
            assert!(leading.ends_with("src"), "leading elision keeps the leaf");
            assert!(trailing.ends_with(ELLIPSIS), "trailing elision marks the tail");
            assert!(trailing.starts_with('/'), "trailing elision keeps the root");
        });
    }

    #[test]
    fn a_budget_that_fits_nothing_yields_nothing() {
        with_ui(|ui| {
            let font = FontId::proportional(style::TEXT_HINT);
            assert!(elide(ui, "anything", font.clone(), 0.0, Side::Leading).is_empty());
            assert!(elide(ui, "anything", font, 0.0, Side::Trailing).is_empty());
        });
    }

    #[test]
    fn text_that_already_fits_is_returned_untouched() {
        with_ui(|ui| {
            let font = FontId::proportional(style::TEXT_HINT);
            assert_eq!(elide(ui, "short", font, 10_000.0, Side::Leading), "short");
        });
    }
}
