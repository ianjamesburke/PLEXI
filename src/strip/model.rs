//! Authoritative Niri-style scrolling column strip model (`SPEC.md` R31–R36, R40, R42).
//!
//! Provides the single source of truth for:
//! - Columns in a context
//! - Panes stacked within each column
//! - Column widths (presets or custom user resizes)
//! - Single active focused pane within the strip
//! - Horizontal scroll offset

use crate::app::zoom::ColumnWidthPreset;
use crate::spatial::tiling::PaneId;
use crate::ui::style;

/// Unique identifier for a column within a strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct ColumnId(pub usize);

/// Column width definition: preset ratio (35/65) or custom user-resized ratio.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ColumnWidth {
    /// Standard preset (35% parent or 65% child).
    Preset(ColumnWidthPreset),
    /// Custom normalized width ratio (0.05 ..= 0.95).
    CustomRatio(f32),
    /// Custom exact pixel width.
    CustomPixels(f32),
}

impl Default for ColumnWidth {
    fn default() -> Self {
        Self::Preset(ColumnWidthPreset::Child65)
    }
}

impl ColumnWidth {
    /// Return the ratio of available width this column requests.
    pub fn ratio(&self) -> f32 {
        match *self {
            Self::Preset(preset) => preset.ratio(),
            Self::CustomRatio(r) => r.clamp(0.05, 0.95),
            Self::CustomPixels(px) => {
                let canvas_w = style::STRIP_FALLBACK_CANVAS_W;
                (px / canvas_w).clamp(0.05, 0.95)
            }
        }
    }

    /// True if this column width is currently a custom user override.
    pub fn is_custom(&self) -> bool {
        matches!(self, Self::CustomRatio(_) | Self::CustomPixels(_))
    }
}

/// One column in a context's strip, holding one or more vertically stacked panes.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Column {
    /// Unique column identifier (or zero-based index).
    pub id: ColumnId,
    /// Width configuration of this column.
    pub width: ColumnWidth,
    /// Panes stacked vertically in this column, from top to bottom.
    pub panes: Vec<PaneId>,
    /// Relative height shares for stacked panes (normalized when laid out).
    pub heights: Vec<f32>,
    /// Last focused pane in this column.
    pub last_focused: Option<PaneId>,
}

impl Column {
    /// Create a new column with a single pane and a width preset.
    pub fn new(id: ColumnId, pane_id: PaneId, width: ColumnWidth) -> Self {
        Self {
            id,
            width,
            panes: vec![pane_id],
            heights: vec![1.0],
            last_focused: Some(pane_id),
        }
    }

    /// Create a new column with multiple stacked panes.
    pub fn new_stacked(id: ColumnId, panes: Vec<PaneId>, width: ColumnWidth) -> Self {
        let n = panes.len().max(1);
        let first = panes.first().copied();
        Self {
            id,
            width,
            panes,
            heights: vec![1.0; n],
            last_focused: first,
        }
    }

    /// Check if this column contains `pane_id`.
    pub fn contains_pane(&self, pane_id: PaneId) -> bool {
        self.panes.contains(&pane_id)
    }

    /// Focus a pane in this column if present.
    pub fn focus_pane(&mut self, pane_id: PaneId) -> bool {
        if self.contains_pane(pane_id) {
            self.last_focused = Some(pane_id);
            true
        } else {
            false
        }
    }

    /// Remove a pane from this column. Returns true if removed.
    pub fn remove_pane(&mut self, pane_id: PaneId) -> bool {
        if let Some(pos) = self.panes.iter().position(|&p| p == pane_id) {
            self.panes.remove(pos);
            if pos < self.heights.len() {
                self.heights.remove(pos);
            }
            if self.last_focused == Some(pane_id) {
                self.last_focused = self.panes.get(pos).or_else(|| self.panes.last()).copied();
            }
            true
        } else {
            false
        }
    }

    /// Append a pane to this vertical stack.
    pub fn push_pane(&mut self, pane_id: PaneId) {
        if !self.panes.contains(&pane_id) {
            self.panes.push(pane_id);
            self.heights.push(1.0);
            self.last_focused = Some(pane_id);
        }
    }
}

/// The authoritative strip for a context.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Strip {
    /// The context owning this strip.
    pub context_id: u64,
    /// Columns in the strip from left to right.
    pub columns: Vec<Column>,
    /// Currently focused pane in the strip (must exist in one column if non-empty).
    pub focused_pane: Option<PaneId>,
    /// First visible column index (the left visible column).
    pub scroll_offset: usize,
}

impl Strip {
    /// Create an empty strip for a context.
    pub fn new(context_id: u64) -> Self {
        Self {
            context_id,
            columns: Vec::new(),
            focused_pane: None,
            scroll_offset: 0,
        }
    }

    /// Number of columns in the strip.
    pub fn column_count(&self) -> usize {
        self.columns.len()
    }

    /// Total number of panes across all columns.
    pub fn pane_count(&self) -> usize {
        self.columns.iter().map(|c| c.panes.len()).sum()
    }

    /// Remove a pane from the strip, removing the column if empty.
    pub fn remove_pane(&mut self, pane_id: PaneId) -> bool {
        let Some(col_idx) = self.find_column_for_pane(pane_id) else {
            return false;
        };
        self.columns[col_idx].remove_pane(pane_id);
        if self.columns[col_idx].panes.is_empty() {
            self.columns.remove(col_idx);
            for (i, c) in self.columns.iter_mut().enumerate() {
                c.id = ColumnId(i);
            }
        }
        if self.focused_pane == Some(pane_id) {
            self.focused_pane = self
                .columns
                .get(col_idx)
                .and_then(|c| c.last_focused)
                .or_else(|| self.columns.last().and_then(|c| c.last_focused));
        }
        true
    }

    /// Find column containing `pane_id`.
    pub fn find_column_for_pane(&self, pane_id: PaneId) -> Option<usize> {
        self.columns.iter().position(|col| col.contains_pane(pane_id))
    }

    /// Return true if the strip contains `pane_id`.
    pub fn contains_pane(&self, pane_id: PaneId) -> bool {
        self.find_column_for_pane(pane_id).is_some()
    }

    /// Set the focused pane, automatically recording column memory and scrolling to keep it visible.
    pub fn focus_pane(&mut self, pane_id: PaneId) -> bool {
        let Some(col_idx) = self.find_column_for_pane(pane_id) else {
            return false;
        };
        self.columns[col_idx].focus_pane(pane_id);
        self.focused_pane = Some(pane_id);
        self.ensure_column_visible(col_idx);
        true
    }

    /// Scroll offset clamp so `scroll_offset` never exceeds `column_count.saturating_sub(2)`.
    pub fn max_scroll_offset(&self) -> usize {
        self.columns.len().saturating_sub(2)
    }

    /// Ensure that `col_idx` is within the 2 visible columns starting at `scroll_offset`.
    pub fn ensure_column_visible(&mut self, col_idx: usize) {
        let max_lo = self.max_scroll_offset();
        if col_idx < self.scroll_offset {
            self.scroll_offset = col_idx;
        } else if col_idx >= self.scroll_offset + 2 {
            self.scroll_offset = col_idx.saturating_sub(1);
        }
        self.scroll_offset = self.scroll_offset.min(max_lo);
    }

    /// Add a column to the right of the strip.
    pub fn push_column(&mut self, mut col: Column) {
        col.id = ColumnId(self.columns.len());
        if self.focused_pane.is_none() {
            self.focused_pane = col.last_focused;
        }
        self.columns.push(col);
    }

    /// Insert a new column right of `col_idx` (or at the end).
    pub fn insert_column_right_of(&mut self, col_idx: usize, pane_id: PaneId, width: ColumnWidth) -> usize {
        let insert_idx = (col_idx + 1).min(self.columns.len());
        let new_col = Column::new(ColumnId(insert_idx), pane_id, width);
        self.columns.insert(insert_idx, new_col);
        // Re-index columns
        for (i, c) in self.columns.iter_mut().enumerate() {
            c.id = ColumnId(i);
        }
        self.focus_pane(pane_id);
        insert_idx
    }

    /// Set a custom width ratio on column `col_idx`.
    pub fn set_column_width(&mut self, col_idx: usize, width: ColumnWidth) -> bool {
        if let Some(col) = self.columns.get_mut(col_idx) {
            col.width = width;
            true
        } else {
            false
        }
    }

    /// Reset all column widths in the strip to their default presets (col 0: 35%, others: 65%).
    pub fn reset_column_widths(&mut self) {
        for (idx, col) in self.columns.iter_mut().enumerate() {
            let preset = if idx == 0 {
                ColumnWidthPreset::Parent35
            } else {
                ColumnWidthPreset::Child65
            };
            col.width = ColumnWidth::Preset(preset);
        }
    }

    /// True if there are off-screen columns to the left of the visible pair.
    pub fn has_left_clipped(&self) -> bool {
        self.columns.len() > 2 && self.scroll_offset > 0
    }

    /// True if there are off-screen columns to the right of the visible pair.
    pub fn has_right_clipped(&self) -> bool {
        self.columns.len() > 2 && self.scroll_offset + 2 < self.columns.len()
    }
}
