//! Virtual scrolling viewport math. Pure — no egui.
//!
//! Adapted from Ferrite <https://github.com/OlaProeis/Ferrite>
//! @ 3ba085c561670342d72c560efbf6b0b92b5c0b46 (view.rs), MIT.
//! Diverges from upstream: no soft-wrap; uniform line height.

use std::ops::Range;

/// Extra lines laid out above/below the viewport so partially visible rows
/// paint fully during scroll.
const OVERSCAN_LINES: usize = 1;

/// Scroll/viewport state for a uniform-line-height document view.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewState {
    /// Vertical scroll offset in points (0 = top).
    pub scroll_y: f32,
    /// Visible height in points.
    pub viewport_height: f32,
    /// Height of one line in points.
    pub line_height: f32,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            scroll_y: 0.0,
            viewport_height: 0.0,
            line_height: 16.0,
        }
    }
}

impl ViewState {
    /// Total content height for `line_count` lines.
    #[must_use]
    pub fn content_height(&self, line_count: usize) -> f32 {
        line_count as f32 * self.line_height
    }

    /// The window of lines to lay out, with overscan, clamped to the document.
    #[must_use]
    pub fn visible_lines(&self, line_count: usize) -> Range<usize> {
        if self.line_height <= 0.0 || line_count == 0 {
            return 0..0;
        }
        let first = (self.scroll_y / self.line_height).floor().max(0.0) as usize;
        let visible = (self.viewport_height / self.line_height).ceil() as usize + 1;
        let start = first.saturating_sub(OVERSCAN_LINES);
        let end = (first + visible + OVERSCAN_LINES).min(line_count);
        start..end.max(start)
    }

    /// Top y-offset of `line` relative to the content origin.
    #[must_use]
    pub fn line_top(&self, line: usize) -> f32 {
        line as f32 * self.line_height
    }

    /// Clamps `scroll_y` to the scrollable range for `line_count` lines.
    pub fn clamp_scroll(&mut self, line_count: usize) {
        let max = (self.content_height(line_count) - self.viewport_height).max(0.0);
        self.scroll_y = self.scroll_y.clamp(0.0, max);
    }

    /// Adjusts `scroll_y` minimally so `line` is fully visible.
    pub fn scroll_to_line(&mut self, line: usize, line_count: usize) {
        let top = self.line_top(line);
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

    fn view() -> ViewState {
        ViewState {
            scroll_y: 0.0,
            viewport_height: 100.0,
            line_height: 10.0,
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
