//! Pure layout calculations for Niri scrolling column strips (`SPEC.md` R31–R36, R40, R42).
//!
//! Evaluates a pure `Strip` state against a `StripViewport` to compute:
//! - Geometry and screen rects for all panes (visible and clipped)
//! - Dividers between adjacent columns and within stacked panes
//! - Geometry for left and right clipped neighbor peek strips
//! - Focused pane outline frame

use crate::spatial::tiling::PaneId;
use crate::strip::model::{ColumnId, Strip};
use crate::ui::style;
use egui::{Pos2, Rect, Vec2};

/// Viewport bounds and parameters for strip layout.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StripViewport {
    /// Full canvas bounding rectangle available to the strip.
    pub canvas_rect: Rect,
    /// Inter-pane and inter-column gap in pixels.
    pub gap: f32,
    /// Width reserved for clipped-neighbor peek indicators.
    pub peek_width: f32,
}

impl Default for StripViewport {
    fn default() -> Self {
        Self {
            canvas_rect: Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 800.0)),
            gap: 4.0,
            peek_width: style::STRIP_PEEK_W,
        }
    }
}

impl StripViewport {
    pub fn new(canvas_rect: Rect, gap: f32) -> Self {
        Self {
            canvas_rect,
            gap: gap.clamp(0.0, 20.0),
            peek_width: style::STRIP_PEEK_W,
        }
    }
}

/// Geometry and status of a single pane in the strip layout.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaneLayout {
    pub pane_id: PaneId,
    pub column_id: ColumnId,
    pub rect: Rect,
    /// True if this pane is fully inside the visible two-column window.
    pub is_on_screen: bool,
    /// True if this pane is the single focused pane of the strip.
    pub is_focused: bool,
}

/// Geometry of a draggable or reset-able divider.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DividerLayout {
    pub rect: Rect,
    pub is_vertical: bool,
    pub left_col: Option<ColumnId>,
    pub right_col: Option<ColumnId>,
}

/// Fully resolved pure layout result.
#[derive(Debug, Clone, PartialEq)]
pub struct StripLayout {
    pub panes: Vec<PaneLayout>,
    pub dividers: Vec<DividerLayout>,
    pub peek_left: Option<Rect>,
    pub peek_right: Option<Rect>,
    pub focus_frame: Option<Rect>,
}

/// Pure strip layout function.
/// Takes strip state and viewport, and produces the complete layout.
pub fn layout_strip(strip: &Strip, viewport: &StripViewport) -> StripLayout {
    let mut panes = Vec::new();
    let mut dividers = Vec::new();

    let n = strip.columns.len();
    if n == 0 {
        return StripLayout {
            panes,
            dividers,
            peek_left: None,
            peek_right: None,
            focus_frame: None,
        };
    }

    let canvas_rect = viewport.canvas_rect;
    let canvas_w = canvas_rect.width();
    let canvas_h = canvas_rect.height();
    let gap = viewport.gap;
    let peek_w = viewport.peek_width;
    let lo = strip.scroll_offset.min(strip.max_scroll_offset());

    let has_left = strip.has_left_clipped();
    let has_right = strip.has_right_clipped();

    // Calculate column widths
    let column_widths = compute_column_widths(strip, canvas_w, gap, peek_w, lo);

    // Calculate x positions for each column
    let mut col_x = canvas_rect.min.x;
    let mut col_rects = Vec::with_capacity(n);

    for (idx, &w) in column_widths.iter().enumerate() {
        let is_visible = idx >= lo && idx < lo + 2;
        let rect = Rect::from_min_size(Pos2::new(col_x, canvas_rect.min.y), Vec2::new(w, canvas_h));
        col_rects.push((rect, is_visible));
        col_x += w + gap;
    }

    let mut focused_pane_rect = None;

    // Layout panes within each column
    for (col_idx, col) in strip.columns.iter().enumerate() {
        let (c_rect, is_col_visible) = col_rects[col_idx];
        let pane_count = col.panes.len();

        if pane_count == 1 {
            let pid = col.panes[0];
            let is_focused = strip.focused_pane == Some(pid);
            if is_focused {
                focused_pane_rect = Some(c_rect);
            }
            panes.push(PaneLayout {
                pane_id: pid,
                column_id: col.id,
                rect: c_rect,
                is_on_screen: is_col_visible,
                is_focused,
            });
        } else if pane_count > 1 {
            let total_gaps = gap * (pane_count - 1) as f32;
            let available_h = (c_rect.height() - total_gaps).max(1.0);
            let sum_heights: f32 = col.heights.iter().take(pane_count).sum();
            let sum_heights = if sum_heights <= 0.0 { 1.0 } else { sum_heights };

            let mut y = c_rect.min.y;
            for (p_idx, &pid) in col.panes.iter().enumerate() {
                let share = col.heights.get(p_idx).copied().unwrap_or(1.0);
                let h = (available_h * (share / sum_heights)).max(1.0);
                let p_rect = Rect::from_min_size(Pos2::new(c_rect.min.x, y), Vec2::new(c_rect.width(), h));
                let is_focused = strip.focused_pane == Some(pid);
                if is_focused {
                    focused_pane_rect = Some(p_rect);
                }
                panes.push(PaneLayout {
                    pane_id: pid,
                    column_id: col.id,
                    rect: p_rect,
                    is_on_screen: is_col_visible,
                    is_focused,
                });

                if p_idx + 1 < pane_count {
                    let divider_rect = Rect::from_min_size(
                        Pos2::new(c_rect.min.x, y + h),
                        Vec2::new(c_rect.width(), gap),
                    );
                    dividers.push(DividerLayout {
                        rect: divider_rect,
                        is_vertical: false,
                        left_col: Some(col.id),
                        right_col: Some(col.id),
                    });
                }
                y += h + gap;
            }
        }

        // Column divider to the next visible column
        if col_idx + 1 < n {
            let (next_rect, _) = col_rects[col_idx + 1];
            let divider_rect = Rect::from_min_max(
                Pos2::new(c_rect.max.x, canvas_rect.min.y),
                Pos2::new(next_rect.min.x, canvas_rect.max.y),
            );
            dividers.push(DividerLayout {
                rect: divider_rect,
                is_vertical: true,
                left_col: Some(col.id),
                right_col: Some(strip.columns[col_idx + 1].id),
            });
        }
    }

    // Peek indicator strips
    let peek_left = if has_left {
        Some(Rect::from_min_size(
            canvas_rect.min,
            Vec2::new(peek_w, canvas_h),
        ))
    } else {
        None
    };

    let peek_right = if has_right {
        Some(Rect::from_min_size(
            Pos2::new(canvas_rect.max.x - peek_w, canvas_rect.min.y),
            Vec2::new(peek_w, canvas_h),
        ))
    } else {
        None
    };

    // Focus outline rect (expanded by gap)
    let focus_frame = focused_pane_rect.map(|r| r.expand(gap));

    StripLayout {
        panes,
        dividers,
        peek_left,
        peek_right,
        focus_frame,
    }
}

/// Compute column widths respecting presets, custom ratios, and peek strips.
fn compute_column_widths(
    strip: &Strip,
    canvas_w: f32,
    gap: f32,
    peek_w: f32,
    lo: usize,
) -> Vec<f32> {
    let n = strip.columns.len();
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![canvas_w];
    }
    if n == 2 {
        let r0 = strip.columns[0].width.ratio();
        let r1 = strip.columns[1].width.ratio();
        let sum = (r0 + r1).max(f32::EPSILON);
        let avail = (canvas_w - gap).max(1.0);
        return vec![avail * (r0 / sum), avail * (r1 / sum)];
    }

    let left_count = lo;
    let right_count = n.saturating_sub(lo + 2);

    let left_total = if left_count > 0 { peek_w } else { 0.0 };
    let right_total = if right_count > 0 { peek_w } else { 0.0 };

    let total_gaps = gap * (n - 1) as f32;
    let avail = (canvas_w - total_gaps).max(1.0);
    let visible_avail = (avail - left_total - right_total).max(2.0);

    let vis0_ratio = strip.columns.get(lo).map(|c| c.width.ratio()).unwrap_or(0.35);
    let vis1_ratio = strip.columns.get(lo + 1).map(|c| c.width.ratio()).unwrap_or(0.65);
    let vis_sum = (vis0_ratio + vis1_ratio).max(f32::EPSILON);

    let mut widths = Vec::with_capacity(n);
    for idx in 0..n {
        if idx < lo {
            widths.push(left_total / left_count as f32);
        } else if idx >= lo + 2 {
            widths.push(right_total / right_count as f32);
        } else if idx == lo {
            widths.push((visible_avail * (vis0_ratio / vis_sum)).max(1.0));
        } else {
            widths.push((visible_avail * (vis1_ratio / vis_sum)).max(1.0));
        }
    }
    widths
}
