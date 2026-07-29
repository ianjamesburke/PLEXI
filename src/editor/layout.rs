//! Source-to-display layout for editor presentation.
//!
//! Source cursors remain authoritative. Each source line supplies display text
//! plus a boundary map from display character boundaries back to source
//! columns. Soft wrapping then partitions that display text into visual rows.
//! Today the display text is identical to source text; the explicit map is the
//! seam for concealed markers and other non-identity presentation.

use std::ops::Range;

use super::cursor::Cursor;

/// One visual row within a source line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayRow {
    /// Character-boundary range in display text.
    pub display: Range<usize>,
    /// Source-column range represented by this row.
    pub source: Range<usize>,
}

/// Display mapping and visual rows for one source line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineLayout {
    pub source_line: usize,
    pub display_text: String,
    /// One source column for every display character boundary.
    pub display_to_source: Vec<usize>,
    pub rows: Vec<DisplayRow>,
}

impl LineLayout {
    /// Identity source/display mapping, partitioned by display-row char counts.
    #[must_use]
    pub fn identity(source_line: usize, text: String, row_char_counts: &[usize]) -> Self {
        let char_count = text.chars().count();
        let display_to_source = (0..=char_count).collect();
        Self::mapped(source_line, text, display_to_source, row_char_counts)
    }

    /// Builds a line from arbitrary display text and its source-boundary map.
    ///
    /// `display_to_source.len()` must be `display_text.chars().count() + 1`;
    /// entries must be monotonic. Repeated source columns are allowed for
    /// display-only characters and skipped columns for concealed source.
    #[must_use]
    pub fn mapped(
        source_line: usize,
        display_text: String,
        display_to_source: Vec<usize>,
        row_char_counts: &[usize],
    ) -> Self {
        let char_count = display_text.chars().count();
        assert_eq!(display_to_source.len(), char_count + 1);
        assert!(display_to_source.windows(2).all(|pair| pair[0] <= pair[1]));

        let counts = if row_char_counts.is_empty() {
            vec![char_count]
        } else {
            row_char_counts.to_vec()
        };
        assert_eq!(counts.iter().sum::<usize>(), char_count);

        let mut display_start = 0;
        let rows = counts
            .into_iter()
            .map(|count| {
                let display_end = display_start + count;
                let row = DisplayRow {
                    display: display_start..display_end,
                    source: display_to_source[display_start]..display_to_source[display_end],
                };
                display_start = display_end;
                row
            })
            .collect();
        Self {
            source_line,
            display_text,
            display_to_source,
            rows,
        }
    }

    #[must_use]
    pub fn source_column_for_display(&self, display_column: usize) -> usize {
        self.display_to_source[display_column.min(self.display_to_source.len() - 1)]
    }

    /// Display boundary nearest a source column. At concealed spans this
    /// chooses the first boundary at or after the source column.
    #[must_use]
    pub fn display_column_for_source(&self, source_column: usize) -> usize {
        self.display_to_source
            .partition_point(|mapped| *mapped < source_column)
            .min(self.display_to_source.len() - 1)
    }

    #[must_use]
    pub fn row_for_source_column(&self, source_column: usize) -> usize {
        let display = self.display_column_for_source(source_column);
        self.rows
            .iter()
            .position(|row| {
                display < row.display.end
                    || (display == row.display.end
                        && row.display.end == self.display_to_source.len() - 1)
            })
            .unwrap_or_else(|| self.rows.len().saturating_sub(1))
    }
}

/// Whole-document source-line/display-row mapping.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DisplayLayout {
    lines: Vec<LineLayout>,
    first_rows: Vec<usize>,
    display_row_count: usize,
}

impl DisplayLayout {
    #[must_use]
    pub fn new(lines: Vec<LineLayout>) -> Self {
        let mut first_rows = Vec::with_capacity(lines.len());
        let mut display_row_count = 0;
        for line in &lines {
            first_rows.push(display_row_count);
            display_row_count += line.rows.len().max(1);
        }
        Self {
            lines,
            first_rows,
            display_row_count,
        }
    }

    #[must_use]
    pub fn line(&self, source_line: usize) -> Option<&LineLayout> {
        self.lines.get(source_line)
    }

    #[must_use]
    pub fn source_line_count(&self) -> usize {
        self.lines.len()
    }

    #[must_use]
    pub fn display_row_count(&self) -> usize {
        self.display_row_count
    }

    #[must_use]
    pub fn first_display_row(&self, source_line: usize) -> usize {
        self.first_rows.get(source_line).copied().unwrap_or(0)
    }

    #[must_use]
    pub fn row_count(&self, source_line: usize) -> usize {
        self.lines
            .get(source_line)
            .map_or(1, |line| line.rows.len().max(1))
    }

    #[must_use]
    pub fn source_line_at_display_row(&self, display_row: usize) -> usize {
        if self.lines.is_empty() {
            return 0;
        }
        self.first_rows
            .partition_point(|first| *first <= display_row)
            .saturating_sub(1)
            .min(self.lines.len() - 1)
    }

    #[must_use]
    pub fn display_row_for_cursor(&self, cursor: Cursor) -> usize {
        let Some(line) = self.line(cursor.line) else {
            return 0;
        };
        self.first_display_row(cursor.line) + line.row_for_source_column(cursor.column)
    }

    #[must_use]
    pub fn cursor_at_display_row_boundary(&self, display_row: usize, end: bool) -> Cursor {
        let line_index = self.source_line_at_display_row(display_row);
        let Some(line) = self.line(line_index) else {
            return Cursor::default();
        };
        let row_index = display_row.saturating_sub(self.first_display_row(line_index));
        let row = &line.rows[row_index.min(line.rows.len().saturating_sub(1))];
        Cursor::new(
            line_index,
            if end {
                row.source.end
            } else {
                row.source.start
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapped_layout_supports_non_identity_source_boundaries() {
        let line = LineLayout::mapped(0, "hello".into(), vec![2, 3, 4, 8, 9, 10], &[3, 2]);
        assert_eq!(line.rows[0].source, 2..8);
        assert_eq!(line.rows[1].source, 8..10);
        assert_eq!(line.source_column_for_display(3), 8);
        assert_eq!(line.display_column_for_source(7), 3);
    }

    #[test]
    fn document_maps_source_cursors_to_visual_rows_and_boundaries() {
        let layout = DisplayLayout::new(vec![
            LineLayout::identity(0, "abcdefgh".into(), &[3, 3, 2]),
            LineLayout::identity(1, "xy".into(), &[2]),
        ]);
        assert_eq!(layout.display_row_count(), 4);
        assert_eq!(layout.display_row_for_cursor(Cursor::new(0, 4)), 1);
        assert_eq!(
            layout.cursor_at_display_row_boundary(2, false),
            Cursor::new(0, 6)
        );
        assert_eq!(
            layout.cursor_at_display_row_boundary(2, true),
            Cursor::new(0, 8)
        );
        assert_eq!(layout.source_line_at_display_row(3), 1);
    }
}
