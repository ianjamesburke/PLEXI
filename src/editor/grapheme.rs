//! Grapheme cluster boundary helpers for caret navigation.
//!
//! Adapted from Ferrite <https://github.com/OlaProeis/Ferrite>
//! @ 3ba085c561670342d72c560efbf6b0b92b5c0b46, MIT.

use unicode_segmentation::UnicodeSegmentation;

/// Char-column of the previous grapheme cluster boundary (snaps backward when
/// `char_col` falls inside a cluster; 0 at the start).
#[must_use]
pub fn prev_grapheme_boundary(line_text: &str, char_col: usize) -> usize {
    let mut prev_start = 0;
    let mut char_offset = 0;
    for grapheme in line_text.graphemes(true) {
        if char_offset >= char_col {
            return prev_start;
        }
        prev_start = char_offset;
        char_offset += grapheme.chars().count();
    }
    prev_start
}

/// Char-column just past the grapheme cluster containing `char_col`
/// (total char count at the end).
#[must_use]
pub fn next_grapheme_boundary(line_text: &str, char_col: usize) -> usize {
    let mut char_offset = 0;
    for grapheme in line_text.graphemes(true) {
        let grapheme_end = char_offset + grapheme.chars().count();
        if char_col < grapheme_end {
            return grapheme_end;
        }
        char_offset = grapheme_end;
    }
    char_offset
}

/// Snaps `char_col` to the start of the grapheme cluster containing it
/// (identity when already on a boundary; total char count when past the end).
#[must_use]
pub fn snap_to_boundary(line_text: &str, char_col: usize) -> usize {
    let mut offset = 0;
    for grapheme in line_text.graphemes(true) {
        let end = offset + grapheme.chars().count();
        if char_col < end {
            return offset;
        }
        offset = end;
    }
    offset
}

/// Line text with exactly one trailing `\n` / `\r\n` terminator stripped, as
/// returned by `TextBuffer::get_line`. A `\r` that is document content (not
/// part of a CRLF terminator) is preserved.
#[must_use]
pub fn line_text_stripped(line: &str) -> &str {
    if let Some(s) = line.strip_suffix("\r\n") {
        s
    } else {
        line.strip_suffix('\n').unwrap_or(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin_boundaries() {
        assert_eq!(prev_grapheme_boundary("Hello", 3), 2);
        assert_eq!(prev_grapheme_boundary("Hello", 0), 0);
        assert_eq!(next_grapheme_boundary("Hello", 2), 3);
        assert_eq!(next_grapheme_boundary("Hello", 5), 5);
    }

    #[test]
    fn emoji_zwj_family_is_single_cluster() {
        // 👨‍👩‍👧 = 5 chars, 1 grapheme
        let s = "👨\u{200D}👩\u{200D}👧";
        assert_eq!(s.chars().count(), 5);
        assert_eq!(next_grapheme_boundary(s, 0), 5);
        assert_eq!(prev_grapheme_boundary(s, 5), 0);
        // Mid-cluster snaps
        assert_eq!(prev_grapheme_boundary(s, 2), 0);
        assert_eq!(next_grapheme_boundary(s, 2), 5);
    }

    #[test]
    fn emoji_with_surrounding_latin() {
        let s = "ab👨\u{200D}👩\u{200D}👧cd";
        assert_eq!(next_grapheme_boundary(s, 1), 2);
        assert_eq!(next_grapheme_boundary(s, 2), 7);
        assert_eq!(prev_grapheme_boundary(s, 7), 2);
        assert_eq!(prev_grapheme_boundary(s, 2), 1);
    }

    #[test]
    fn combining_diacritics_are_one_cluster() {
        // "e" + combining acute = 2 chars, 1 grapheme
        let s = "e\u{0301}";
        assert_eq!(next_grapheme_boundary(s, 0), 2);
        assert_eq!(prev_grapheme_boundary(s, 2), 0);
    }

    #[test]
    fn korean_conjoining_jamo() {
        let s = "\u{1112}\u{1161}\u{11AB}"; // 3 chars, 1 grapheme
        assert_eq!(next_grapheme_boundary(s, 0), 3);
        assert_eq!(prev_grapheme_boundary(s, 3), 0);
    }

    #[test]
    fn empty_string_boundaries() {
        assert_eq!(prev_grapheme_boundary("", 0), 0);
        assert_eq!(next_grapheme_boundary("", 0), 0);
    }

    #[test]
    fn strips_exactly_one_line_terminator() {
        assert_eq!(line_text_stripped("Hello\n"), "Hello");
        assert_eq!(line_text_stripped("Hello\r\n"), "Hello");
        assert_eq!(line_text_stripped("Hello"), "Hello");
        // A bare trailing \r is content, not a terminator.
        assert_eq!(line_text_stripped("Hello\r"), "Hello\r");
    }

    #[test]
    fn snap_to_boundary_lands_on_cluster_starts() {
        let s = "e\u{0301}x"; // é(2 chars) + x
        assert_eq!(snap_to_boundary(s, 0), 0);
        assert_eq!(snap_to_boundary(s, 1), 0); // inside the cluster
        assert_eq!(snap_to_boundary(s, 2), 2);
        assert_eq!(snap_to_boundary(s, 3), 3);
        assert_eq!(snap_to_boundary(s, 99), 3);
    }
}
