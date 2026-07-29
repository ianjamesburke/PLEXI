//! Virtual scrolling viewport math. Pure — no egui.
//!
//! Adapted from Ferrite <https://github.com/OlaProeis/Ferrite>
//! @ 3ba085c561670342d72c560efbf6b0b92b5c0b46 (view.rs), MIT.
//! Diverges from upstream: source lines may occupy multiple display rows;
//! optional per-line extra height (`line_extras`) follows the final display
//! row for inline attachment strips (Live Preview images, 0478).

use std::ops::Range;

use super::cursor::Cursor;
use super::layout::DisplayLayout;

/// Extra lines laid out above/below the viewport so partially visible rows
/// paint fully during scroll.
const OVERSCAN_LINES: usize = 1;

/// Scroll/viewport state for a uniform-display-row-height document view.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewState {
    /// Vertical scroll offset in points (0 = top).
    pub scroll_y: f32,
    /// Horizontal scroll offset in points (0 = left). Wrapped note modes pin
    /// this to zero; code mode uses it for unwrapped long lines.
    pub scroll_x: f32,
    /// Visible height in points.
    pub viewport_height: f32,
    /// Visible width in points.
    pub viewport_width: f32,
    /// Height of one line in points.
    pub line_height: f32,
    /// Source-line to display-row mapping for the current width/presentation.
    pub layout: DisplayLayout,
    /// Extra height below specific lines (sorted by line index, no
    /// duplicates): reserved for inline attachment strips rendered under the
    /// line's text. Set each frame by the widget; empty for uniform layout.
    pub line_extras: Vec<(usize, f32)>,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            scroll_y: 0.0,
            scroll_x: 0.0,
            viewport_height: 0.0,
            viewport_width: 0.0,
            line_height: 16.0,
            layout: DisplayLayout::default(),
            line_extras: Vec::new(),
        }
    }
}

impl ViewState {
    /// Sum of extra heights for lines strictly before `line`.
    fn extra_before(&self, line: usize) -> f32 {
        self.line_extras
            .iter()
            .take_while(|(l, _)| *l < line)
            .map(|(_, e)| e)
            .sum()
    }

    /// Extra height reserved below `line` (0 for uniform lines).
    #[must_use]
    pub fn line_extra(&self, line: usize) -> f32 {
        self.line_extras
            .iter()
            .find(|(l, _)| *l == line)
            .map_or(0.0, |(_, e)| *e)
    }

    /// Total content height for `line_count` lines.
    #[must_use]
    pub fn content_height(&self, line_count: usize) -> f32 {
        let row_count = if self.layout.source_line_count() == line_count {
            self.layout.display_row_count()
        } else {
            line_count
        };
        row_count as f32 * self.line_height
            + self
                .line_extras
                .iter()
                .filter(|(l, _)| *l < line_count)
                .map(|(_, e)| e)
                .sum::<f32>()
    }

    /// Line index containing content y-offset `y` (clamped to the document).
    #[must_use]
    pub fn line_at_y(&self, y: f32, line_count: usize) -> usize {
        if self.line_height <= 0.0 || line_count == 0 {
            return 0;
        }
        let y = y.max(0.0);
        let mut acc = 0.0_f32;
        for &(l, e) in &self.line_extras {
            let top = self.line_text_top(l) + self.line_text_height(l) + acc;
            if y < top {
                break;
            }
            if y < top + e {
                return l.min(line_count.saturating_sub(1));
            }
            acc += e;
        }
        let display_row = (((y - acc) / self.line_height).floor()).max(0.0) as usize;
        if self.layout.source_line_count() == line_count {
            self.layout.source_line_at_display_row(
                display_row.min(self.layout.display_row_count().saturating_sub(1)),
            )
        } else {
            display_row.min(line_count.saturating_sub(1))
        }
    }

    /// The window of lines to lay out, with overscan, clamped to the document.
    #[must_use]
    pub fn visible_lines(&self, line_count: usize) -> Range<usize> {
        if self.line_height <= 0.0 || line_count == 0 {
            return 0..0;
        }
        let first = self.line_at_y(self.scroll_y, line_count);
        let last = self.line_at_y(self.scroll_y + self.viewport_height, line_count);
        let start = first.saturating_sub(OVERSCAN_LINES);
        let end = (last + 1 + OVERSCAN_LINES).min(line_count);
        start..end.max(start)
    }

    /// Top y-offset of `line` relative to the content origin.
    #[must_use]
    pub fn line_top(&self, line: usize) -> f32 {
        self.line_text_top(line) + self.extra_before(line)
    }

    fn line_text_top(&self, line: usize) -> f32 {
        let row = if self.layout.source_line_count() > line {
            self.layout.first_display_row(line)
        } else {
            line
        };
        row as f32 * self.line_height
    }

    /// Height occupied by a source line's display rows.
    #[must_use]
    pub fn line_text_height(&self, line: usize) -> f32 {
        let rows = if self.layout.source_line_count() > line {
            self.layout.row_count(line)
        } else {
            1
        };
        rows as f32 * self.line_height
    }

    /// Clamps `scroll_y` to the scrollable range for `line_count` lines.
    pub fn clamp_scroll(&mut self, line_count: usize) {
        let max = (self.content_height(line_count) - self.viewport_height).max(0.0);
        self.scroll_y = self.scroll_y.clamp(0.0, max);
    }

    /// Adjusts `scroll_x` minimally so a caret at horizontal offset `x`
    /// (relative to the content origin) is visible, with a small margin.
    pub fn scroll_to_x(&mut self, x: f32) {
        const MARGIN: f32 = 8.0;
        if self.viewport_width <= 0.0 {
            return;
        }
        if x < self.scroll_x + MARGIN {
            self.scroll_x = (x - MARGIN).max(0.0);
        } else if x > self.scroll_x + self.viewport_width - MARGIN {
            self.scroll_x = x - self.viewport_width + MARGIN;
        }
    }

    /// Adjusts `scroll_y` minimally so a source line is fully visible.
    pub fn scroll_to_line(&mut self, line: usize, line_count: usize) {
        let top = self.line_top(line);
        let bottom = top + self.line_text_height(line) + self.line_extra(line);
        if top < self.scroll_y {
            self.scroll_y = top;
        } else if bottom > self.scroll_y + self.viewport_height {
            self.scroll_y = bottom - self.viewport_height;
        }
        self.clamp_scroll(line_count);
    }

    /// Adjusts `scroll_y` minimally so the cursor's display row is visible.
    pub fn scroll_to_cursor(&mut self, cursor: Cursor, line_count: usize) {
        let display_row = if self.layout.source_line_count() == line_count {
            self.layout.display_row_for_cursor(cursor)
        } else {
            cursor.line
        };
        let top = display_row as f32 * self.line_height + self.extra_before(cursor.line);
        let bottom = top + self.line_height;
        if top < self.scroll_y {
            self.scroll_y = top;
        } else if bottom > self.scroll_y + self.viewport_height {
            self.scroll_y = bottom - self.viewport_height;
        }
        self.clamp_scroll(line_count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::layout::LineLayout;

    fn view() -> ViewState {
        ViewState {
            viewport_height: 100.0,
            line_height: 10.0,
            ..ViewState::default()
        }
    }

    #[test]
    fn visible_lines_windows_large_document() {
        let mut v = view();
        let lines = 10_000;
        assert_eq!(v.visible_lines(lines), 0..12);

        v.scroll_y = 50_000.0; // line 5000 at top
        let r = v.visible_lines(lines);
        assert_eq!(r, 4999..5012);
        // Windowing: never proportional to document size.
        assert!(r.len() < 20);

        // Scrolled to the very bottom.
        v.scroll_y = v.content_height(lines) - v.viewport_height;
        let r = v.visible_lines(lines);
        assert_eq!(r.end, lines);
        assert!(r.len() < 20);
    }

    #[test]
    fn visible_lines_empty_and_degenerate() {
        let v = view();
        assert_eq!(v.visible_lines(0), 0..0);
        let z = ViewState {
            line_height: 0.0,
            ..view()
        };
        assert_eq!(z.visible_lines(100), 0..0);
    }

    #[test]
    fn clamp_scroll_bounds() {
        let mut v = view();
        v.scroll_y = -5.0;
        v.clamp_scroll(50);
        assert_eq!(v.scroll_y, 0.0);
        v.scroll_y = 1e9;
        v.clamp_scroll(50);
        assert_eq!(v.scroll_y, 400.0); // 500 content - 100 viewport

        // Content shorter than viewport pins to 0.
        v.clamp_scroll(5);
        assert_eq!(v.scroll_y, 0.0);
    }

    #[test]
    fn scroll_to_x_keeps_caret_horizontally_visible() {
        let mut v = ViewState {
            viewport_width: 100.0,
            ..view()
        };
        v.scroll_to_x(50.0); // already visible
        assert_eq!(v.scroll_x, 0.0);
        v.scroll_to_x(200.0); // off the right edge
        assert_eq!(v.scroll_x, 108.0);
        v.scroll_to_x(20.0); // off the left edge
        assert_eq!(v.scroll_x, 12.0);
        v.scroll_to_x(0.0); // back to the start clamps at 0
        assert_eq!(v.scroll_x, 0.0);
    }

    #[test]
    fn line_extras_shift_tops_heights_and_hit_mapping() {
        let mut v = view();
        v.line_extras = vec![(2, 50.0), (5, 20.0)];
        assert_eq!(v.line_top(0), 0.0);
        assert_eq!(v.line_top(2), 20.0);
        assert_eq!(v.line_top(3), 80.0); // 30 uniform + 50 extra
        assert_eq!(v.line_top(6), 130.0); // 60 uniform + 70 extras
        assert_eq!(v.content_height(10), 170.0);
        // y→line inverts line_top across uniform and extra regions.
        assert_eq!(v.line_at_y(0.0, 10), 0);
        assert_eq!(v.line_at_y(25.0, 10), 2); // inside line 2's text row
        assert_eq!(v.line_at_y(45.0, 10), 2); // inside line 2's image strip
        assert_eq!(v.line_at_y(80.0, 10), 3);
        assert_eq!(v.line_at_y(1e9, 10), 9);
        // Round-trip: every line's top maps back to itself.
        for line in 0..10 {
            assert_eq!(v.line_at_y(v.line_top(line), 10), line);
        }
    }

    #[test]
    fn wrapped_rows_drive_height_y_mapping_and_extras_follow_final_row() {
        let mut v = view();
        v.layout = DisplayLayout::new(vec![
            LineLayout::identity(0, "abcdefgh".into(), &[3, 3, 2]),
            LineLayout::identity(1, "xy".into(), &[2]),
        ]);
        v.line_extras = vec![(0, 20.0)];

        assert_eq!(v.line_text_height(0), 30.0);
        assert_eq!(v.line_top(1), 50.0);
        assert_eq!(v.content_height(2), 60.0);
        assert_eq!(v.line_at_y(25.0, 2), 0);
        assert_eq!(v.line_at_y(40.0, 2), 0);
        assert_eq!(v.line_at_y(50.0, 2), 1);

        v.scroll_y = 0.0;
        v.viewport_height = 10.0;
        v.scroll_to_cursor(Cursor::new(0, 7), 2);
        assert_eq!(v.scroll_y, 20.0);
    }

    #[test]
    fn scroll_to_line_moves_minimally() {
        let mut v = view();
        v.scroll_to_line(5, 100); // already visible
        assert_eq!(v.scroll_y, 0.0);
        v.scroll_to_line(20, 100); // below: bottom-align
        assert_eq!(v.scroll_y, 110.0);
        v.scroll_to_line(3, 100); // above: top-align
        assert_eq!(v.scroll_y, 30.0);
    }
}
