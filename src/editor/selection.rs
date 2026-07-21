//! Selection helpers: select-all, word and line selection.
//!
//! Adapted from Ferrite <https://github.com/OlaProeis/Ferrite>
//! @ 3ba085c561670342d72c560efbf6b0b92b5c0b46 (selection.rs), MIT.

use super::buffer::TextBuffer;
use super::cursor::{Cursor, Selection};
use super::movement::{doc_end, line_len, line_text};

/// Selection spanning the whole document.
#[must_use]
pub fn select_all(buffer: &TextBuffer) -> Selection {
    Selection::new(Cursor::default(), doc_end(buffer))
}

/// Word boundaries around `cursor`, classifying chars as word
/// (alphanumeric/`_`), whitespace, or punctuation. Used for double-click.
#[must_use]
pub fn word_boundaries(buffer: &TextBuffer, cursor: Cursor) -> Selection {
    let chars: Vec<char> = line_text(buffer, cursor.line).chars().collect();
    let col = cursor.column.min(chars.len());
    if chars.is_empty() {
        return Selection::collapsed(Cursor::new(cursor.line, 0));
    }
    let probe = if col < chars.len() {
        chars[col]
    } else {
        chars[col - 1]
    };

    let class = |c: char| -> u8 {
        if c.is_alphanumeric() || c == '_' {
            0
        } else if c.is_whitespace() {
            1
        } else {
            2
        }
    };
    let target = class(probe);

    let mut start = col;
    while start > 0 && class(chars[start - 1]) == target {
        start -= 1;
    }
    let mut end = col;
    while end < chars.len() && class(chars[end]) == target {
        end += 1;
    }
    Selection::new(
        Cursor::new(cursor.line, start),
        Cursor::new(cursor.line, end),
    )
}

/// Full-line selection for `line`, including the trailing newline when one
/// exists (head lands at the start of the next line). Used for triple-click.
#[must_use]
pub fn line_selection(buffer: &TextBuffer, line: usize) -> Selection {
    let line = line.min(buffer.line_count().saturating_sub(1));
    let anchor = Cursor::new(line, 0);
    let head = if line + 1 < buffer.line_count() {
        Cursor::new(line + 1, 0)
    } else {
        Cursor::new(line, line_len(buffer, line))
    };
    Selection::new(anchor, head)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_all_spans_document() {
        let b = TextBuffer::from_string("one\ntwo\nthree");
        let sel = select_all(&b);
        assert_eq!(sel.anchor, Cursor::new(0, 0));
        assert_eq!(sel.head, Cursor::new(2, 5));
    }

    #[test]
    fn word_boundaries_on_word_and_punctuation() {
        let b = TextBuffer::from_string("foo bar_baz, qux");
        let sel = word_boundaries(&b, Cursor::new(0, 5));
        assert_eq!((sel.anchor.column, sel.head.column), (4, 11));
        // On the comma: punctuation run
        let sel = word_boundaries(&b, Cursor::new(0, 11));
        assert_eq!((sel.anchor.column, sel.head.column), (11, 12));
        // At end of line: uses preceding char
        let sel = word_boundaries(&b, Cursor::new(0, 16));
        assert_eq!((sel.anchor.column, sel.head.column), (13, 16));
    }

    #[test]
    fn word_boundaries_on_empty_line() {
        let b = TextBuffer::from_string("a\n\nb");
        let sel = word_boundaries(&b, Cursor::new(1, 0));
        assert!(!sel.is_range());
    }

    #[test]
    fn line_selection_includes_newline() {
        let b = TextBuffer::from_string("one\ntwo");
        let sel = line_selection(&b, 0);
        assert_eq!(sel.anchor, Cursor::new(0, 0));
        assert_eq!(sel.head, Cursor::new(1, 0));
        // Last line has no trailing newline
        let sel = line_selection(&b, 1);
        assert_eq!(sel.head, Cursor::new(1, 3));
    }
}
