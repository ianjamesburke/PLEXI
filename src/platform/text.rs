//! Char-safe text truncation, shared by every site that has to fit text into a
//! budget — transcript previews in the host UI and tool output headed for a
//! model alike.
//!
//! One wording for the omission marker, everywhere: it always names how many
//! characters were dropped, so a reader (human or model) can tell a short
//! answer from a clipped one.

use std::borrow::Cow;

/// Smallest budget worth spending on a marker. Below this the text is simply
/// cut — a marker would eat the whole allowance.
const MIN_MARKER_BUDGET: usize = 12;

/// How a truncated span is laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// Keep a head and a tail, newlines intact, marker on its own line.
    Block,
    /// Collapse to a single line: input newlines become spaces and the marker
    /// sits inline between the head and the tail.
    Inline,
    /// Keep the head only, marker at the end. For text read from the top.
    Head,
}

impl Style {
    fn marker(self, omitted: usize) -> String {
        match self {
            Style::Block => format!("\n[{omitted} chars omitted]\n"),
            Style::Inline => format!(" [{omitted} chars omitted] "),
            Style::Head => format!("… [{omitted} chars omitted]"),
        }
    }
}

/// Truncate `text` to at most `budget` characters.
///
/// The budget covers the marker, so the result never exceeds it. Returns the
/// text unchanged when it already fits.
pub fn head_tail(text: &str, budget: usize, style: Style) -> String {
    let source: Cow<'_, str> = match style {
        Style::Inline => Cow::Owned(text.replace('\n', " ")),
        Style::Block | Style::Head => Cow::Borrowed(text),
    };
    let chars: Vec<char> = source.chars().collect();
    if chars.len() <= budget {
        return source.into_owned();
    }
    if budget < MIN_MARKER_BUDGET {
        return chars.into_iter().take(budget).collect();
    }

    // Two passes: the marker names the omitted count, and how much text is
    // omitted depends on how many characters the marker itself costs.
    let mut keep = budget;
    for _ in 0..2 {
        let omitted = chars.len().saturating_sub(keep);
        keep = budget.saturating_sub(style.marker(omitted).chars().count());
    }
    let (head, tail) = match style {
        Style::Head => (keep, 0),
        Style::Block | Style::Inline => {
            let head = keep.div_ceil(2);
            (head, keep.saturating_sub(head))
        }
    };
    let omitted = chars.len().saturating_sub(head + tail);

    let mut out: String = chars.iter().take(head).collect();
    out.push_str(&style.marker(omitted));
    out.extend(chars.iter().skip(chars.len() - tail));
    // A longer omitted-count in the second pass can push the marker past the
    // budget; clamp so the guarantee holds for every input.
    out.chars().take(budget).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_passes_through_unchanged() {
        assert_eq!(head_tail("short", 100, Style::Block), "short");
        assert_eq!(head_tail("short", 100, Style::Head), "short");
    }

    #[test]
    fn inline_style_flattens_newlines_even_when_untruncated() {
        assert_eq!(head_tail("a\nb", 100, Style::Inline), "a b");
    }

    #[test]
    fn every_style_respects_the_budget() {
        let long = "x".repeat(5_000);
        for style in [Style::Block, Style::Inline, Style::Head] {
            for budget in [12, 13, 40, 200, 4_999] {
                let out = head_tail(&long, budget, style);
                assert!(
                    out.chars().count() <= budget,
                    "{style:?} at budget {budget} produced {} chars",
                    out.chars().count()
                );
            }
        }
    }

    #[test]
    fn block_keeps_head_and_tail_around_the_marker() {
        let text = format!("HEAD{}TAIL", "-".repeat(500));
        let out = head_tail(&text, 120, Style::Block);
        assert!(out.starts_with("HEAD"), "{out}");
        assert!(out.ends_with("TAIL"), "{out}");
        assert!(out.contains("chars omitted"), "{out}");
    }

    #[test]
    fn head_style_drops_the_tail() {
        let text = format!("HEAD{}TAIL", "-".repeat(500));
        let out = head_tail(&text, 120, Style::Head);
        assert!(out.starts_with("HEAD"), "{out}");
        assert!(out.ends_with("chars omitted]"), "{out}");
    }

    #[test]
    fn a_budget_too_small_for_a_marker_just_cuts() {
        let out = head_tail("abcdefghij", 4, Style::Block);
        assert_eq!(out, "abcd");
    }

    #[test]
    fn multi_byte_characters_are_never_split() {
        let text = "é".repeat(400);
        let out = head_tail(&text, 60, Style::Block);
        assert!(out.chars().count() <= 60);
        assert!(out.starts_with('é'));
    }
}
