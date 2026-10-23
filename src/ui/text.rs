//! Measured text elision.
//!
//! One algorithm and one ellipsis for every truncated label the host paints,
//! so a path in the sidebar, a path in the file browser, and a title in a
//! picker all shorten the same way. Every candidate is measured against the
//! real font — never a character count, which ignores both the font and the
//! width actually available.

use std::sync::Arc;
use unicode_segmentation::UnicodeSegmentation;

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
    /// Preserve the head and distinguishing suffix of a filename or identifier.
    Middle,
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

    // One line by contract: `layout_no_wrap` still breaks on `\n`, and a
    // multi-line galley's width is its widest line, so an embedded break both
    // defeats the ellipsis and grows the row. Collapse breaks to a space
    // before measuring.
    let collapsed;
    let text = if text.contains(['\n', '\r']) {
        collapsed = text
            .split(['\n', '\r'])
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        collapsed.as_str()
    } else {
        text
    };

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

    let graphemes: Vec<&str> = text.graphemes(true).collect();
    let mut best = ellipsis_only;
    for keep in 1..graphemes.len() {
        let candidate = match side {
            Side::Leading => format!(
                "{ELLIPSIS}{}",
                graphemes[graphemes.len() - keep..].concat().trim_start()
            ),
            Side::Trailing => format!("{}{ELLIPSIS}", graphemes[..keep].concat().trim_end()),
            Side::Middle => {
                let head = keep.div_ceil(2);
                let tail = keep / 2;
                format!(
                    "{}{ELLIPSIS}{}",
                    graphemes[..head].concat().trim_end(),
                    graphemes[graphemes.len() - tail..].concat().trim_start()
                )
            }
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

/// Locale-independent counts for compact host chrome.
pub(crate) fn format_count(count: usize) -> String {
    let digits = count.to_string();
    let mut result = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            result.push(',');
        }
        result.push(digit);
    }
    result
}

pub(crate) fn format_count_plural(count: usize, singular: &str, plural: &str) -> String {
    format!(
        "{} {}",
        format_count(count),
        if count == 1 { singular } else { plural }
    )
}

/// Badge labels have a bounded magnitude rather than growing without limit.
pub(crate) fn compact_count(count: usize) -> String {
    if count >= 1_000_000 {
        "1M".into()
    } else if count >= 1_000 {
        format!("{}K", count / 1_000)
    } else {
        count.to_string()
    }
}

/// Reserve a fixed digit capacity with monospace blanks. Choose capacity for
/// the counter's domain; values exceeding it expand rather than lose digits.
pub(crate) fn tabular_count(count: usize, capacity: usize, size: f32) -> egui::RichText {
    egui::RichText::new(format!("{count:>capacity$}")).font(FontId::monospace(size))
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
    fn middle_preserves_filename_extensions_and_identifier_tails() {
        with_ui(|ui| {
            for (text, suffix) in [
                (
                    "Q3 Board Deck — FINAL (revised) v12 [approved by legal].pdf",
                    ".pdf",
                ),
                (
                    "IMG_20250914_183022_HDR_portrait_edited_edited.HEIC",
                    ".HEIC",
                ),
                ("12345678-1234-5678-9012-abcdef123456", "123456"),
                ("👩🏽‍💻family👨‍👩‍👧‍👦long title\nfile.pdf", ".pdf"),
            ] {
                let galley =
                    elided_galley(ui, text, FontId::proportional(14.0), 160.0, Side::Middle);
                assert!(galley
                    .text()
                    .starts_with(text.graphemes(true).next().unwrap()));
                assert!(galley.text().ends_with(suffix), "{}", galley.text());
                assert!(galley.text().contains(ELLIPSIS));
                assert!(!galley.text().contains(['\n', '\r']));
                assert!(galley.size().x <= 160.0);
                assert_eq!(galley.rows.len(), 1);
                for part in galley.text().split(ELLIPSIS) {
                    assert!(!part.starts_with('\u{200d}') && !part.ends_with('\u{200d}'));
                }
            }
        });
    }

    #[test]
    fn count_labels_and_fixed_capacity_numerals() {
        assert_eq!(format_count_plural(1, "item", "items"), "1 item");
        assert_eq!(format_count_plural(1284, "item", "items"), "1,284 items");
        assert_eq!(format_count(1_000_001), "1,000,001");
        assert_eq!(compact_count(999), "999");
        assert_eq!(compact_count(1284), "1K");
        assert_eq!(compact_count(9_000_000), "1M");
        with_ui(|ui| {
            let width = |count| {
                let text = tabular_count(count, 6, 12.0);
                ui.fonts_mut(|f| {
                    f.layout_no_wrap(
                        text.text().to_owned(),
                        FontId::monospace(12.0),
                        Color32::WHITE,
                    )
                    .size()
                    .x
                })
            };
            assert_eq!(width(99), width(100));
            assert_eq!(width(0), width(999999));
        });
    }

    #[test]
    fn elided_text_fits_the_budget_on_one_line() {
        with_ui(|ui| {
            let font = FontId::proportional(style::TEXT_HINT);
            for side in [Side::Leading, Side::Middle, Side::Trailing] {
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
            assert!(
                leading.starts_with(ELLIPSIS),
                "leading elision marks the head"
            );
            assert!(leading.ends_with("src"), "leading elision keeps the leaf");
            assert!(
                trailing.ends_with(ELLIPSIS),
                "trailing elision marks the tail"
            );
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

    /// Agent detail text (hook output, diffs) carries line breaks. A row is
    /// one line by contract, so breaks must collapse before elision — the
    /// palette flashed a two-line subtitle while the multi-line detail was
    /// live (`layout_no_wrap` still honours `\n`, and the width check only
    /// measures the widest line so the ellipsis never triggered).
    #[test]
    fn line_breaks_collapse_to_one_line() {
        with_ui(|ui| {
            let font = FontId::proportional(style::TEXT_HINT);
            let out = elide(
                ui,
                "ws \u{b7} agent 1\nBash(ls)\r\n--- a/x",
                font.clone(),
                60.0,
                Side::Trailing,
            );
            assert!(!out.contains('\n') && !out.contains('\r'));
            let galley =
                ui.fonts_mut(|f| f.layout_no_wrap(out.clone(), font, Color32::PLACEHOLDER));
            assert_eq!(galley.rows.len(), 1, "{out:?}");
            assert!(galley.size().x <= 60.0);
            assert!(out.ends_with(ELLIPSIS));
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
