//! Pane geometry snapshot types retained for host and CLI compatibility.
//! The renderer fills these from the active window's tile tree.

use crate::spatial::tiling::PaneId;
use crate::strip::model::ColumnId;
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
