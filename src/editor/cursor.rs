//! Cursor position and anchor/head selection model.
//!
//! Adapted from Ferrite <https://github.com/OlaProeis/Ferrite>
//! @ 3ba085c561670342d72c560efbf6b0b92b5c0b46, MIT.

/// Caret position: zero-indexed (line, char column).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize,
)]
pub struct Cursor {
    pub line: usize,
    pub column: usize,
}

impl Cursor {
    #[must_use]
    pub fn new(line: usize, column: usize) -> Self {
        Self { line, column }
    }
}

impl PartialOrd for Cursor {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Cursor {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.line, self.column).cmp(&(other.line, other.column))
    }
}

/// Selection between an `anchor` (fixed) and `head` (moving caret).
/// Anchor and head may be in either document order.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize,
)]
pub struct Selection {
    pub anchor: Cursor,
    pub head: Cursor,
}

impl Selection {
    #[must_use]
    pub fn new(anchor: Cursor, head: Cursor) -> Self {
        Self { anchor, head }
    }

    /// Cursor with no range.
    #[must_use]
    pub fn collapsed(cursor: Cursor) -> Self {
        Self {
            anchor: cursor,
            head: cursor,
        }
    }

    #[must_use]
    pub fn is_range(&self) -> bool {
        self.anchor != self.head
    }

    /// Returns (start, end) in document order.
    #[must_use]
    pub fn ordered(&self) -> (Cursor, Cursor) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    /// True when `line` intersects the selection's line span.
    #[must_use]
    pub fn touches_line(&self, line: usize) -> bool {
        let (start, end) = self.ordered();
        line >= start.line && line <= end.line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_ordering_is_line_then_column() {
        assert!(Cursor::new(0, 5) < Cursor::new(0, 10));
        assert!(Cursor::new(0, 10) < Cursor::new(1, 0));
        assert_eq!(Cursor::new(2, 3), Cursor::new(2, 3));
    }

    #[test]
    fn ordered_normalizes_backward_selection() {
        let sel = Selection::new(Cursor::new(2, 5), Cursor::new(0, 3));
        let (start, end) = sel.ordered();
        assert_eq!(start, Cursor::new(0, 3));
        assert_eq!(end, Cursor::new(2, 5));
        assert!(sel.is_range());
    }

    #[test]
    fn collapsed_selection_has_no_range() {
        let sel = Selection::collapsed(Cursor::new(1, 4));
        assert!(!sel.is_range());
        assert_eq!(sel.anchor, sel.head);
    }

    #[test]
    fn touches_line_spans_multiline_selection() {
        let sel = Selection::new(Cursor::new(2, 5), Cursor::new(4, 1));
        assert!(!sel.touches_line(1));
        assert!(sel.touches_line(2));
        assert!(sel.touches_line(3));
        assert!(sel.touches_line(4));
        assert!(!sel.touches_line(5));
    }
}
