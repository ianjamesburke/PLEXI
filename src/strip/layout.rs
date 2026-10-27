//! Pure layout calculations for Niri continuous scrolling column strips (`SPEC.md` R31–R36, R40, R42).
//!
//! Evaluates a pure `Strip` state against a `StripViewport` to produce one presentation
//! snapshot per layout generation supplying:
//! - Unclipped outer bounds (`outer_rect`), viewport-intersected clip bounds (`clip_rect`),
//!   and pointer hit bounds (`hit_rect`) for every pane
//! - Visibility flags (`is_visible`, `is_on_screen`, `is_partially_visible`) and focus state
//! - Dividers between adjacent columns and within stacked panes
//! - Clipped-neighbor indicator metadata and focused pane outline geometry

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
    /// Width used for optional clipped-neighbor indicator metadata.
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

/// Geometry and presentation status of a single pane in the strip layout snapshot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaneLayout {
    pub pane_id: PaneId,
    pub column_id: ColumnId,
    /// Effective screen rect (`clip_rect` when visible in the viewport, `outer_rect` when off-screen).
    pub rect: Rect,
    /// Unclipped pane rect in screen coordinates (translated by `-scroll_offset_px`).
    pub outer_rect: Rect,
    /// Intersection of `outer_rect` with the strip `canvas_rect` (empty if off-screen).
    pub clip_rect: Rect,
    /// Pointer hit-test rect (`clip_rect` when visible, `Rect::NOTHING` when off-screen).
    pub hit_rect: Rect,
    /// True if this pane has a non-empty intersection with the viewport (`clip_rect` > 0.5px).
    pub is_visible: bool,
    /// True if this pane is fully inside the viewport horizontally.
    pub is_on_screen: bool,
    /// True if this pane is visible in the viewport but clipped by the left or right canvas edge.
    pub is_partially_visible: bool,
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

/// Fully resolved presentation snapshot for a strip layout generation.
#[derive(Debug, Clone, PartialEq)]
pub struct StripLayout {
    pub panes: Vec<PaneLayout>,
    pub dividers: Vec<DividerLayout>,
    pub peek_left: Option<Rect>,
    pub peek_right: Option<Rect>,
    pub focus_frame: Option<Rect>,
    /// Effective continuous horizontal scroll offset in logical points.
    pub scroll_offset_px: f32,
    /// Total unclipped width of the strip in logical points.
    pub total_width_px: f32,
    /// Canvas bounding rectangle used to compute this snapshot.
    pub canvas_rect: Rect,
}

impl StripLayout {
    /// Look up the resolved presentation layout for `pane_id`.
    pub fn pane(&self, pane_id: PaneId) -> Option<&PaneLayout> {
        self.panes.iter().find(|p| p.pane_id == pane_id)
    }
}

/// Pure strip layout function.
/// Evaluates `strip` against `viewport` to produce the complete presentation snapshot.
pub fn layout_strip(strip: &Strip, viewport: &StripViewport) -> StripLayout {
    let mut panes = Vec::new();
    let mut dividers = Vec::new();

    let canvas_rect = viewport.canvas_rect;
    let n = strip.columns.len();
    if n == 0 {
        return StripLayout {
            panes,
            dividers,
            peek_left: None,
            peek_right: None,
            focus_frame: None,
            scroll_offset_px: 0.0,
            total_width_px: 0.0,
            canvas_rect,
        };
    }

    let canvas_w = canvas_rect.width().max(1.0);
    let canvas_h = canvas_rect.height().max(1.0);
    let gap = viewport.gap;
    let peek_w = viewport.peek_width;

    // Resolve each column's intrinsic logical-point width and strip-local x coordinate.
    let intervals = strip.column_intervals(canvas_w, gap);
    let total_width_px = intervals.last().map(|&(x, w)| x + w).unwrap_or(0.0);
    let max_scroll_px = (total_width_px - canvas_w).max(0.0);

    // If `strip.scroll_offset` was mutated directly by a caller/test without calling a `Strip`
    // method, honor that column offset; otherwise use the continuous `scroll_offset_px`.
    let scroll_offset_px = if strip.scroll_offset != strip.last_synced_scroll_offset {
        if strip.scroll_offset >= strip.max_scroll_offset() {
            max_scroll_px
        } else {
            intervals
                .get(strip.scroll_offset)
                .map(|&(x, _)| x.min(max_scroll_px))
                .unwrap_or(0.0)
        }
    } else {
        strip.scroll_offset_px.clamp(0.0, max_scroll_px)
    };

    let has_left = scroll_offset_px > 0.5;
    let has_right = scroll_offset_px + 0.5 < max_scroll_px;

    let mut col_rects = Vec::with_capacity(n);
    for &(local_x, w) in &intervals {
        let screen_x = canvas_rect.min.x + local_x - scroll_offset_px;
        let outer_rect = Rect::from_min_size(Pos2::new(screen_x, canvas_rect.min.y), Vec2::new(w, canvas_h));
        col_rects.push(outer_rect);
    }

    let mut focused_pane_rect = None;

    for (col_idx, col) in strip.columns.iter().enumerate() {
        let c_outer = col_rects[col_idx];
        let pane_count = col.panes.len();

        if pane_count == 1 {
            let pid = col.panes[0];
            let p_layout = build_pane_layout(
                pid,
                col.id,
                c_outer,
                canvas_rect,
                strip.focused_pane == Some(pid),
            );
            if p_layout.is_focused {
                focused_pane_rect = Some(if p_layout.is_visible {
                    p_layout.clip_rect
                } else {
                    p_layout.outer_rect
                });
            }
            panes.push(p_layout);
        } else if pane_count > 1 {
            let total_gaps = gap * (pane_count - 1) as f32;
            let available_h = (c_outer.height() - total_gaps).max(1.0);
            let sum_heights: f32 = col.heights.iter().take(pane_count).sum();
            let sum_heights = if sum_heights <= 0.0 { 1.0 } else { sum_heights };

            let mut y = c_outer.min.y;
            for (p_idx, &pid) in col.panes.iter().enumerate() {
                let share = col.heights.get(p_idx).copied().unwrap_or(1.0);
                let h = if p_idx + 1 == pane_count {
                    (c_outer.max.y - y).max(1.0)
                } else {
                    (available_h * (share / sum_heights)).max(1.0)
                };
                let p_outer = Rect::from_min_size(Pos2::new(c_outer.min.x, y), Vec2::new(c_outer.width(), h));
                let p_layout = build_pane_layout(
                    pid,
                    col.id,
                    p_outer,
                    canvas_rect,
                    strip.focused_pane == Some(pid),
                );
                if p_layout.is_focused {
                    focused_pane_rect = Some(if p_layout.is_visible {
                        p_layout.clip_rect
                    } else {
                        p_layout.outer_rect
                    });
                }
                panes.push(p_layout);

                if p_idx + 1 < pane_count {
                    let divider_rect = Rect::from_min_size(
                        Pos2::new(c_outer.min.x, y + h),
                        Vec2::new(c_outer.width(), gap),
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

        if col_idx + 1 < n {
            let next_outer = col_rects[col_idx + 1];
            let divider_rect = Rect::from_min_max(
                Pos2::new(c_outer.max.x, canvas_rect.min.y),
                Pos2::new(next_outer.min.x, canvas_rect.max.y),
            );
            dividers.push(DividerLayout {
                rect: divider_rect,
                is_vertical: true,
                left_col: Some(col.id),
                right_col: Some(strip.columns[col_idx + 1].id),
            });
        }
    }

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

    let focus_frame = focused_pane_rect.map(|r| r.expand(gap));

    StripLayout {
        panes,
        dividers,
        peek_left,
        peek_right,
        focus_frame,
        scroll_offset_px,
        total_width_px,
        canvas_rect,
    }
}

fn build_pane_layout(
    pane_id: PaneId,
    column_id: ColumnId,
    outer_rect: Rect,
    canvas_rect: Rect,
    is_focused: bool,
) -> PaneLayout {
    let raw_clip = outer_rect.intersect(canvas_rect);
    let is_visible = raw_clip.width() > 0.5 && raw_clip.height() > 0.5;
    let clip_rect = if is_visible {
        raw_clip
    } else {
        Rect::NOTHING
    };
    let is_on_screen = is_visible
        && outer_rect.min.x >= canvas_rect.min.x - 0.5
        && outer_rect.max.x <= canvas_rect.max.x + 0.5
        && outer_rect.min.y >= canvas_rect.min.y - 0.5
        && outer_rect.max.y <= canvas_rect.max.y + 0.5;
    let is_partially_visible = is_visible && !is_on_screen;
    let hit_rect = if is_visible { clip_rect } else { Rect::NOTHING };
    let rect = if is_visible { clip_rect } else { outer_rect };

    PaneLayout {
        pane_id,
        column_id,
        rect,
        outer_rect,
        clip_rect,
        hit_rect,
        is_visible,
        is_on_screen,
        is_partially_visible,
        is_focused,
    }
}
