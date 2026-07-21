//! Pure caret movement over a `TextBuffer`.
//!
//! Adapted from Ferrite <https://github.com/OlaProeis/Ferrite>
//! @ 3ba085c561670342d72c560efbf6b0b92b5c0b46 (input/keyboard.rs), MIT.

use super::buffer::TextBuffer;
use super::cursor::Cursor;
use super::grapheme::{line_text_stripped, next_grapheme_boundary, prev_grapheme_boundary};

/// Char length of `line` excluding the trailing newline.
#[must_use]
pub fn line_len(buffer: &TextBuffer, line: usize) -> usize {
    buffer
        .get_line(line)
        .map(|l| line_text_stripped(&l).chars().count())
        .unwrap_or(0)
}

/// Stripped text of `line` (empty when out of bounds).
#[must_use]
pub fn line_text(buffer: &TextBuffer, line: usize) -> String {
    buffer
        .get_line(line)
        .map(|l| line_text_stripped(&l).to_string())
        .unwrap_or_default()
}

/// Clamps a cursor to valid (line, column) coordinates and snaps the column
/// to a grapheme boundary, so externally supplied positions (pointer
/// hit-tests, `SetCursor`) can never split a cluster.
#[must_use]
pub fn clamp(buffer: &TextBuffer, cursor: Cursor) -> Cursor {
    let line = cursor.line.min(buffer.line_count().saturating_sub(1));
    let text = line_text(buffer, line);
    let column = super::grapheme::snap_to_boundary(&text, cursor.column);
    Cursor::new(line, column)
}

/// Converts a cursor to an absolute char index.
#[must_use]
pub fn cursor_to_char(buffer: &TextBuffer, cursor: Cursor) -> usize {
    let cursor = clamp(buffer, cursor);
    buffer.line_to_char(cursor.line) + cursor.column
}

/// Converts an absolute char index to a cursor.
#[must_use]
pub fn char_to_cursor(buffer: &TextBuffer, char_idx: usize) -> Cursor {
    let char_idx = char_idx.min(buffer.len());
    let line = buffer.char_to_line(char_idx);
    Cursor::new(line, char_idx - buffer.line_to_char(line))
}

/// One grapheme left, wrapping to the previous line end.
#[must_use]
pub fn left(buffer: &TextBuffer, cursor: Cursor) -> Cursor {
    let cursor = clamp(buffer, cursor);
    if cursor.column > 0 {
        let text = line_text(buffer, cursor.line);
        Cursor::new(cursor.line, prev_grapheme_boundary(&text, cursor.column))
    } else if cursor.line > 0 {
        Cursor::new(cursor.line - 1, line_len(buffer, cursor.line - 1))
    } else {
        cursor
    }
}

/// One grapheme right, wrapping to the next line start.
#[must_use]
pub fn right(buffer: &TextBuffer, cursor: Cursor) -> Cursor {
    let cursor = clamp(buffer, cursor);
    let len = line_len(buffer, cursor.line);
    if cursor.column < len {
        let text = line_text(buffer, cursor.line);
        Cursor::new(cursor.line, next_grapheme_boundary(&text, cursor.column))
    } else if cursor.line + 1 < buffer.line_count() {
        Cursor::new(cursor.line + 1, 0)
    } else {
        cursor
    }
}

/// One line up, keeping `goal_column` (the column the user is aiming for
/// across short lines). Clamps to the target line's length.
/// On the first line, jumps to the document start (macOS convention).
#[must_use]
pub fn up(buffer: &TextBuffer, cursor: Cursor, goal_column: usize) -> Cursor {
    if cursor.line == 0 {
        return Cursor::new(0, 0);
    }
    clamp(buffer, Cursor::new(cursor.line - 1, goal_column))
}

/// One line down, keeping `goal_column`. Clamps to the target line's length.
/// On the last line, jumps to the document end (macOS convention).
#[must_use]
pub fn down(buffer: &TextBuffer, cursor: Cursor, goal_column: usize) -> Cursor {
    if cursor.line + 1 >= buffer.line_count() {
        return Cursor::new(cursor.line, line_len(buffer, cursor.line));
    }
    clamp(buffer, Cursor::new(cursor.line + 1, goal_column))
}

/// One page up: `rows` lines toward the document start, keeping `goal_column`.
/// On the first line, jumps to the document start.
#[must_use]
pub fn page_up(buffer: &TextBuffer, cursor: Cursor, goal_column: usize, rows: usize) -> Cursor {
    if cursor.line == 0 {
        return Cursor::new(0, 0);
    }
    clamp(
        buffer,
        Cursor::new(cursor.line.saturating_sub(rows.max(1)), goal_column),
    )
}

/// One page down: `rows` lines toward the document end, keeping `goal_column`.
/// On the last line, jumps to the document end.
#[must_use]
pub fn page_down(buffer: &TextBuffer, cursor: Cursor, goal_column: usize, rows: usize) -> Cursor {
    let last = buffer.line_count().saturating_sub(1);
    if cursor.line >= last {
        return doc_end(buffer);
    }
    clamp(
        buffer,
        Cursor::new((cursor.line + rows.max(1)).min(last), goal_column),
    )
}

/// Char class of a grapheme cluster (by its first char): word, whitespace,
/// or punctuation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CharClass {
    Word,
    Whitespace,
    Punctuation,
}

fn classify(c: char) -> CharClass {
    if c.is_alphanumeric() || c == '_' {
        CharClass::Word
    } else if c.is_whitespace() {
        CharClass::Whitespace
    } else {
        CharClass::Punctuation
    }
}

/// (start char-column, class) for each grapheme cluster of `text`, plus the
/// total char count. Word movement operates on whole clusters so the caret
/// never lands inside one (e.g. between a base char and a combining mark).
fn grapheme_classes(text: &str) -> (Vec<(usize, CharClass)>, usize) {
    use unicode_segmentation::UnicodeSegmentation;
    let mut clusters = Vec::new();
    let mut col = 0;
    for g in text.graphemes(true) {
        let class = classify(g.chars().next().unwrap_or(' '));
        clusters.push((col, class));
        col += g.chars().count();
    }
    (clusters, col)
}

/// Start of the previous word (skips whitespace backward, then the word).
#[must_use]
pub fn word_left(buffer: &TextBuffer, cursor: Cursor) -> Cursor {
    let mut cursor = clamp(buffer, cursor);
    if cursor.column == 0 {
        return left(buffer, cursor);
    }
    let (clusters, _) = grapheme_classes(&line_text(buffer, cursor.line));
    // Index of the first cluster at or past the caret.
    let mut i = clusters.partition_point(|(start, _)| *start < cursor.column);
    while i > 0 && clusters[i - 1].1 == CharClass::Whitespace {
        i -= 1;
    }
    if i > 0 {
        let class = clusters[i - 1].1;
        while i > 0 && clusters[i - 1].1 == class {
            i -= 1;
        }
    }
    cursor.column = clusters.get(i).map_or(0, |(start, _)| *start);
    cursor
}

/// End of the next word (skips whitespace forward, then the word).
#[must_use]
pub fn word_right(buffer: &TextBuffer, cursor: Cursor) -> Cursor {
    let mut cursor = clamp(buffer, cursor);
    let (clusters, total) = grapheme_classes(&line_text(buffer, cursor.line));
    if cursor.column >= total {
        return right(buffer, cursor);
    }
    // Index of the cluster containing the caret.
    let mut i = clusters
        .partition_point(|(start, _)| *start <= cursor.column)
        .saturating_sub(1);
    while i < clusters.len() && clusters[i].1 == CharClass::Whitespace {
        i += 1;
    }
    if i < clusters.len() {
        let class = clusters[i].1;
        while i < clusters.len() && clusters[i].1 == class {
            i += 1;
        }
    }
    cursor.column = clusters.get(i).map_or(total, |(start, _)| *start);
    cursor
}

/// Column 0 of the current line.
#[must_use]
pub fn line_start(cursor: Cursor) -> Cursor {
    Cursor::new(cursor.line, 0)
}

/// End of the current line.
#[must_use]
pub fn line_end(buffer: &TextBuffer, cursor: Cursor) -> Cursor {
    let cursor = clamp(buffer, cursor);
    Cursor::new(cursor.line, line_len(buffer, cursor.line))
}

/// Start of the document.
#[must_use]
pub fn doc_start() -> Cursor {
    Cursor::default()
}

/// End of the document.
#[must_use]
pub fn doc_end(buffer: &TextBuffer) -> Cursor {
    let line = buffer.line_count().saturating_sub(1);
    Cursor::new(line, line_len(buffer, line))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(s: &str) -> TextBuffer {
        TextBuffer::from_string(s)
    }

    #[test]
    fn left_right_wrap_across_lines() {
        let b = buf("ab\ncd");
        assert_eq!(right(&b, Cursor::new(0, 2)), Cursor::new(1, 0));
        assert_eq!(left(&b, Cursor::new(1, 0)), Cursor::new(0, 2));
        assert_eq!(left(&b, Cursor::new(0, 0)), Cursor::new(0, 0));
        assert_eq!(right(&b, Cursor::new(1, 2)), Cursor::new(1, 2));
    }

    #[test]
    fn left_right_move_by_grapheme() {
        let b = buf("a👨\u{200D}👩\u{200D}👧b");
        assert_eq!(right(&b, Cursor::new(0, 1)), Cursor::new(0, 6));
        assert_eq!(left(&b, Cursor::new(0, 6)), Cursor::new(0, 1));
    }

    #[test]
    fn up_down_respect_goal_column() {
        let b = buf("longest line\nab\nanother long");
        // Down from col 8 clamps to short line, goal restores on next down.
        let c = down(&b, Cursor::new(0, 8), 8);
        assert_eq!(c, Cursor::new(1, 2));
        let c = down(&b, c, 8);
        assert_eq!(c, Cursor::new(2, 8));
        let c = up(&b, c, 8);
        assert_eq!(c, Cursor::new(1, 2));
    }

    #[test]
    fn down_on_last_line_goes_to_line_end() {
        let b = buf("hello");
        assert_eq!(down(&b, Cursor::new(0, 2), 2), Cursor::new(0, 5));
    }

    #[test]
    fn word_movement() {
        let b = buf("foo bar_baz  qux");
        assert_eq!(word_right(&b, Cursor::new(0, 0)), Cursor::new(0, 3));
        assert_eq!(word_right(&b, Cursor::new(0, 3)), Cursor::new(0, 11));
        assert_eq!(word_left(&b, Cursor::new(0, 16)), Cursor::new(0, 13));
        assert_eq!(word_left(&b, Cursor::new(0, 11)), Cursor::new(0, 4));
    }

    #[test]
    fn word_movement_stays_on_grapheme_boundaries() {
        // "é" as e + combining acute (2 chars, 1 grapheme) followed by "x".
        let b = buf("e\u{0301}x y");
        assert_eq!(word_right(&b, Cursor::new(0, 0)), Cursor::new(0, 3));
        assert_eq!(word_left(&b, Cursor::new(0, 5)), Cursor::new(0, 4));
        assert_eq!(word_left(&b, Cursor::new(0, 3)), Cursor::new(0, 0));
    }

    #[test]
    fn word_movement_wraps_at_line_edges() {
        let b = buf("foo\nbar");
        assert_eq!(word_right(&b, Cursor::new(0, 3)), Cursor::new(1, 0));
        assert_eq!(word_left(&b, Cursor::new(1, 0)), Cursor::new(0, 3));
    }

    #[test]
    fn line_and_doc_movement() {
        let b = buf("one\ntwo three");
        assert_eq!(line_start(Cursor::new(1, 5)), Cursor::new(1, 0));
        assert_eq!(line_end(&b, Cursor::new(1, 0)), Cursor::new(1, 9));
        assert_eq!(doc_start(), Cursor::new(0, 0));
        assert_eq!(doc_end(&b), Cursor::new(1, 9));
    }

    #[test]
    fn char_index_round_trip() {
        let b = buf("héllo\nwörld");
        let c = Cursor::new(1, 3);
        assert_eq!(char_to_cursor(&b, cursor_to_char(&b, c)), c);
        assert_eq!(cursor_to_char(&b, Cursor::new(1, 0)), 6);
    }

    #[test]
    fn clamp_out_of_bounds() {
        let b = buf("ab\ncd");
        assert_eq!(clamp(&b, Cursor::new(9, 9)), Cursor::new(1, 2));
        assert_eq!(clamp(&b, Cursor::new(0, 9)), Cursor::new(0, 2));
    }

    #[test]
    fn clamp_snaps_inside_grapheme_cluster() {
        let b = buf("e\u{0301}x");
        assert_eq!(clamp(&b, Cursor::new(0, 1)), Cursor::new(0, 0));
        assert_eq!(clamp(&b, Cursor::new(0, 2)), Cursor::new(0, 2));
    }
}
