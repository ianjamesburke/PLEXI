//! Rope-backed text buffer.
//!
//! Adapted from Ferrite <https://github.com/OlaProeis/Ferrite>
//! @ 3ba085c561670342d72c560efbf6b0b92b5c0b46, MIT.

use ropey::Rope;
use std::borrow::Cow;

/// A text buffer backed by a rope for O(log n) edits and index conversion.
///
/// All indices are char indices unless a method name says otherwise.
/// Mutation is `pub(crate)`: outside this module tree, edits must go through
/// the transaction API ([`crate::editor::Document`]).
#[derive(Debug, Clone, Default)]
pub struct TextBuffer {
    rope: Rope,
}

impl TextBuffer {
    #[must_use]
    pub fn from_string(content: &str) -> Self {
        Self {
            rope: Rope::from_str(content),
        }
    }

    /// Inserts `text` at char position `pos`. Panics if out of bounds;
    /// callers validate via the transaction API.
    pub(crate) fn insert(&mut self, pos: usize, text: &str) {
        self.rope.insert(pos, text);
    }

    /// Removes `len` chars starting at `pos`. Panics if out of bounds;
    /// callers validate via the transaction API.
    pub(crate) fn remove(&mut self, pos: usize, len: usize) {
        if len > 0 {
            self.rope.remove(pos..pos + len);
        }
    }

    /// Returns the text between char positions `start_char..end_char`,
    /// clamped to the buffer.
    #[must_use]
    pub fn slice(&self, start_char: usize, end_char: usize) -> String {
        let start = start_char.min(self.len());
        let end = end_char.min(self.len());
        if start >= end {
            return String::new();
        }
        self.rope.slice(start..end).to_string()
    }

    /// Line content including any trailing newline. `None` when out of bounds.
    #[must_use]
    pub fn get_line(&self, line_idx: usize) -> Option<Cow<'_, str>> {
        self.rope.get_line(line_idx).map(Into::into)
    }

    /// Total number of lines; an empty buffer has one empty line.
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.rope.len_lines()
    }

    /// Char offset where `line_idx` begins. Panics if out of bounds.
    #[must_use]
    pub fn line_to_char(&self, line_idx: usize) -> usize {
        self.rope.line_to_char(line_idx)
    }

    /// Line index containing char `char_idx` (may be one past the end).
    /// Panics if out of bounds.
    #[must_use]
    pub fn char_to_line(&self, char_idx: usize) -> usize {
        self.rope.char_to_line(char_idx)
    }

    /// Total number of chars.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rope.len_chars()
    }

    #[cfg(test)]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rope.len_chars() == 0
    }
}

impl std::fmt::Display for TextBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.rope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_buffer_has_one_line() {
        let buffer = TextBuffer::default();
        assert_eq!(buffer.len(), 0);
        assert!(buffer.is_empty());
        assert_eq!(buffer.line_count(), 1);
    }

    #[test]
    fn insert_and_remove_round_trip() {
        let mut buffer = TextBuffer::from_string("Hello World");
        buffer.insert(5, " Beautiful");
        assert_eq!(buffer.to_string(), "Hello Beautiful World");
        buffer.remove(5, 10);
        assert_eq!(buffer.to_string(), "Hello World");
        buffer.remove(2, 0); // zero-length no-op
        assert_eq!(buffer.to_string(), "Hello World");
    }

    #[test]
    fn line_and_char_conversions() {
        let buffer = TextBuffer::from_string("Hello\nWorld\nTest");
        assert_eq!(buffer.line_count(), 3);
        assert_eq!(buffer.line_to_char(1), 6);
        assert_eq!(buffer.char_to_line(5), 0);
        assert_eq!(buffer.char_to_line(6), 1);
        assert_eq!(buffer.get_line(0).unwrap().as_ref(), "Hello\n");
        assert_eq!(buffer.get_line(2).unwrap().as_ref(), "Test");
        assert!(buffer.get_line(3).is_none());
    }

    #[test]
    fn slice_clamps_to_bounds() {
        let buffer = TextBuffer::from_string("Hello, World!");
        assert_eq!(buffer.slice(7, 12), "World");
        assert_eq!(buffer.slice(7, 100), "World!");
        assert_eq!(buffer.slice(5, 5), "");
    }

    #[test]
    fn unicode_char_indexing() {
        let buffer = TextBuffer::from_string("こんにちは\n世界");
        assert_eq!(buffer.len(), 8);
        assert_eq!(buffer.line_count(), 2);
        assert_eq!(buffer.get_line(1).unwrap().as_ref(), "世界");
        assert_eq!(buffer.slice(6, 8), "世界");
    }
}
