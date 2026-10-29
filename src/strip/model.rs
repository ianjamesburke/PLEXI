//! Authoritative Niri-style continuous scrolling column strip model (`SPEC.md` R31–R36, R40, R42).
//!
//! Provides the single source of truth for:
//! - Columns in a context with stable `ColumnId`
//! - Panes stacked within each column and their relative height shares
//! - Intrinsic column widths (presets, custom user ratios, or fixed logical points)
//! - Single active focused pane within the strip and per-column `last_focused` memory
//! - Continuous logical-point horizontal viewport scroll offset (`scroll_offset_px`)

use crate::app::zoom::ColumnWidthPreset;
use crate::spatial::tiling::PaneId;
use crate::ui::style;

fn default_strip_gap() -> f32 {
    4.0
}

/// Unique, stable identifier for a column within a strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct ColumnId(pub usize);

/// Column width definition: preset ratio (35/65), custom ratio, or fixed logical points.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ColumnWidth {
    /// Standard preset (35% parent or 65% child).
    Preset(ColumnWidthPreset),
    /// Custom normalized width ratio (0.05 ..= 0.95).
    CustomRatio(f32),
    /// Custom exact pixel/point width.
    CustomPixels(f32),
    /// Explicit fixed logical-point width.
    FixedPx(f32),
}

impl Default for ColumnWidth {
    fn default() -> Self {
        Self::Preset(ColumnWidthPreset::Child65)
    }
}

impl ColumnWidth {
    /// Resolve this column's intrinsic width in logical points against `pair_avail_w`
    /// (`reference_width_px - gap`).
    pub fn resolve_px(&self, pair_avail_w: f32) -> f32 {
        match *self {
            Self::Preset(preset) => (pair_avail_w * preset.ratio()).max(1.0),
            Self::CustomRatio(r) => (pair_avail_w * r.clamp(0.05, 0.95)).max(1.0),
            Self::CustomPixels(px) | Self::FixedPx(px) => px.max(1.0),
        }
    }

    /// True if this column width is currently a custom user override.
    #[cfg(test)]
    pub fn is_custom(&self) -> bool {
        matches!(
            self,
            Self::CustomRatio(_) | Self::CustomPixels(_) | Self::FixedPx(_)
        )
    }
}

/// One column in a context's strip, holding one or more vertically stacked panes.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Column {
    /// Stable column identifier within the strip.
    pub id: ColumnId,
    /// Width configuration of this column.
    pub width: ColumnWidth,
    /// Panes stacked vertically in this column, from top to bottom.
    pub panes: Vec<PaneId>,
    /// Relative height shares for stacked panes (normalized when laid out).
    pub heights: Vec<f32>,
    /// Last focused pane in this column (`SPEC.md` § Focus memory).
    pub last_focused: Option<PaneId>,
}

impl Column {
    /// Create a new column with a single pane and a width configuration.
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

    /// Return the remembered focused pane if it is still live in this column,
    /// otherwise fall back to the top pane in the stack.
    pub fn remembered_or_first_pane(&self) -> Option<PaneId> {
        self.last_focused
            .filter(|pid| self.panes.contains(pid))
            .or_else(|| self.panes.first().copied())
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

    /// Append a pane to the bottom of this vertical stack.
    pub fn push_pane(&mut self, pane_id: PaneId) {
        if !self.panes.contains(&pane_id) {
            self.panes.push(pane_id);
            self.heights.push(1.0);
            self.last_focused = Some(pane_id);
        }
    }

    /// Insert `new_pane` immediately below `after_pane` in this vertical stack.
    pub fn insert_pane_below(&mut self, after_pane: PaneId, new_pane: PaneId) -> bool {
        if self.panes.contains(&new_pane) {
            self.last_focused = Some(new_pane);
            return true;
        }
        let Some(pos) = self.panes.iter().position(|&p| p == after_pane) else {
            return false;
        };
        let insert_at = (pos + 1).min(self.panes.len());
        self.panes.insert(insert_at, new_pane);
        let h_at = insert_at.min(self.heights.len());
        self.heights.insert(h_at, 1.0);
        self.last_focused = Some(new_pane);
        true
    }
}

/// The authoritative continuous horizontal strip for a context.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Strip {
    /// The context owning this strip.
    pub context_id: u64,
    /// Columns in the strip from left to right.
    pub columns: Vec<Column>,
    /// Currently focused pane in the strip (must exist in one column if non-empty).
    pub focused_pane: Option<PaneId>,
    /// Leading visible column index (synchronized with `scroll_offset_px`).
    pub scroll_offset: usize,
    /// Continuous logical-point horizontal viewport offset.
    #[serde(default)]
    pub scroll_offset_px: f32,
    /// Monotonic counter for assigning stable `ColumnId`s.
    #[serde(default)]
    pub next_column_id: usize,
    /// Reference viewport width in logical points used for preset resolution once established.
    #[serde(default)]
    pub reference_width_px: Option<f32>,
    /// Reference inter-column gap in logical points.
    #[serde(default = "default_strip_gap")]
    pub reference_gap_px: f32,
    /// Tracks the last `scroll_offset` produced by `Strip` methods so direct field assignment in
    /// unit tests (`strip.scroll_offset = k`) is respected by `layout_strip`.
    #[serde(default)]
    pub last_synced_scroll_offset: usize,
}

impl Strip {
    /// Create an empty strip for a context.
    pub fn new(context_id: u64) -> Self {
        Self {
            context_id,
            columns: Vec::new(),
            focused_pane: None,
            scroll_offset: 0,
            scroll_offset_px: 0.0,
            next_column_id: 0,
            reference_width_px: None,
            reference_gap_px: default_strip_gap(),
            last_synced_scroll_offset: 0,
        }
    }

    /// Allocate a fresh, stable `ColumnId` that does not collide with any existing column.
    pub fn alloc_column_id(&mut self) -> ColumnId {
        let max_existing = self.columns.iter().map(|c| c.id.0 + 1).max().unwrap_or(0);
        if self.next_column_id < max_existing {
            self.next_column_id = max_existing;
        }
        let id = ColumnId(self.next_column_id);
        self.next_column_id += 1;
        id
    }

    /// Number of columns in the strip.
    pub fn column_count(&self) -> usize {
        self.columns.len()
    }

    /// Total number of panes across all columns.
    #[cfg(test)]
    pub fn pane_count(&self) -> usize {
        self.columns.iter().map(|c| c.panes.len()).sum()
    }

    /// Effective viewport width in logical points for reveal and width resolution.
    pub fn effective_viewport_width(&self, override_w: Option<f32>) -> f32 {
        override_w
            .filter(|w| w.is_finite() && *w > 0.0)
            .or(self.reference_width_px.filter(|w| w.is_finite() && *w > 0.0))
            .unwrap_or(style::STRIP_FALLBACK_CANVAS_W)
    }

    /// Reference width used to resolve preset/ratio column widths.
    pub fn width_resolution_basis(&self, canvas_w: f32) -> f32 {
        if canvas_w.is_finite() && canvas_w > 0.0 {
            canvas_w
        } else {
            self.reference_width_px
                .filter(|w| w.is_finite() && *w > 0.0)
                .unwrap_or(style::STRIP_FALLBACK_CANVAS_W)
        }
    }

    /// Compute intrinsic logical-point widths for all columns.
    pub fn intrinsic_column_widths(&self, canvas_w: f32, gap: f32) -> Vec<f32> {
        let n = self.columns.len();
        if n == 0 {
            return Vec::new();
        }
        let basis_w = self.width_resolution_basis(canvas_w);
        if n == 1 {
            return match self.columns[0].width {
                ColumnWidth::CustomPixels(px) | ColumnWidth::FixedPx(px) => vec![px.max(1.0)],
                ColumnWidth::CustomRatio(r) => vec![(basis_w * r.clamp(0.05, 0.95)).max(1.0)],
                ColumnWidth::Preset(_) => vec![canvas_w.max(1.0)],
            };
        }
        let pair_avail_w = (basis_w - gap).max(1.0);
        self.columns
            .iter()
            .map(|c| c.width.resolve_px(pair_avail_w))
            .collect()
    }

    /// Compute strip-local `(x_start, width)` intervals for every column.
    pub fn column_intervals(&self, canvas_w: f32, gap: f32) -> Vec<(f32, f32)> {
        let widths = self.intrinsic_column_widths(canvas_w, gap);
        let mut x = 0.0_f32;
        let mut out = Vec::with_capacity(widths.len());
        for w in widths {
            out.push((x, w));
            x += w + gap;
        }
        out
    }

    /// Total content width of the strip in logical points.
    pub fn total_content_width(&self, canvas_w: f32, gap: f32) -> f32 {
        let intervals = self.column_intervals(canvas_w, gap);
        intervals.last().map(|&(x, w)| x + w).unwrap_or(0.0)
    }

    /// Maximum valid `scroll_offset_px` for `canvas_w`.
    pub fn max_scroll_offset_px(&self, canvas_w: f32, gap: f32) -> f32 {
        (self.total_content_width(canvas_w, gap) - canvas_w).max(0.0)
    }

    /// Synchronize the integer `scroll_offset` index from `scroll_offset_px`.
    fn sync_index_from_px(&mut self, canvas_w: f32, gap: f32) {
        if self.columns.len() <= 1 {
            self.scroll_offset = 0;
            self.last_synced_scroll_offset = 0;
            return;
        }
        let intervals = self.column_intervals(canvas_w, gap);
        let max_idx = self.max_scroll_offset();
        // Find the first column that is predominantly in view (at least half its width or starting at/after scroll_offset_px).
        let idx = intervals
            .iter()
            .position(|&(x, w)| x + w * 0.5 > self.scroll_offset_px + 0.5)
            .unwrap_or(max_idx)
            .min(max_idx);
        self.scroll_offset = idx;
        self.last_synced_scroll_offset = idx;
    }

    /// Set continuous scroll offset in logical points, clamped to valid strip bounds.
    pub fn set_scroll_offset_px(&mut self, offset_px: f32, canvas_w: Option<f32>) {
        let cw = self.effective_viewport_width(canvas_w);
        let gap = self.reference_gap_px;
        let max_px = self.max_scroll_offset_px(cw, gap);
        self.scroll_offset_px = offset_px.clamp(0.0, max_px);
        self.sync_index_from_px(cw, gap);
    }

    /// Reveal column `col_idx` using the smallest `scroll_offset_px` change that brings
    /// the column inside `[scroll_offset_px, scroll_offset_px + canvas_w]`.
    /// An oversized column (`width > canvas_w`) aligns its leading edge.
    pub fn reveal_column_with_viewport(&mut self, col_idx: usize, canvas_w: f32, gap: f32) {
        if col_idx >= self.columns.len() {
            return;
        }
        // If `scroll_offset` was mutated directly before this call, sync `scroll_offset_px` first.
        if self.scroll_offset != self.last_synced_scroll_offset {
            let intervals = self.column_intervals(canvas_w, gap);
            let max_px = self.max_scroll_offset_px(canvas_w, gap);
            self.scroll_offset_px = if self.scroll_offset >= self.max_scroll_offset() {
                max_px
            } else {
                intervals
                    .get(self.scroll_offset)
                    .map(|&(x, _)| x.min(max_px))
                    .unwrap_or(0.0)
            };
        }
        let intervals = self.column_intervals(canvas_w, gap);
        let (col_x, col_w) = intervals[col_idx];
        let max_px = self.max_scroll_offset_px(canvas_w, gap);

        let mut next_px = self.scroll_offset_px.clamp(0.0, max_px);
        if col_w >= canvas_w || col_x < next_px {
            next_px = col_x;
        } else if col_x + col_w > next_px + canvas_w {
            next_px = col_x + col_w - canvas_w;
        }
        self.scroll_offset_px = next_px.clamp(0.0, max_px);
        self.sync_index_from_px(canvas_w, gap);
    }

    /// Reveal the interval spanning `[col_a, col_b]` when both fit within the viewport;
    /// otherwise reveal `col_b` (the newly created / primary target column).
    pub fn reveal_pair_with_viewport(
        &mut self,
        col_a: usize,
        col_b: usize,
        canvas_w: f32,
        gap: f32,
    ) {
        if self.columns.is_empty() {
            return;
        }
        let left_idx = col_a.min(col_b).min(self.columns.len() - 1);
        let right_idx = col_a.max(col_b).min(self.columns.len() - 1);
        let intervals = self.column_intervals(canvas_w, gap);
        let (left_x, _) = intervals[left_idx];
        let (right_x, right_w) = intervals[right_idx];
        let span_w = (right_x + right_w) - left_x;
        if span_w <= canvas_w + 0.5 {
            let max_px = self.max_scroll_offset_px(canvas_w, gap);
            let mut next_px = self.scroll_offset_px.clamp(0.0, max_px);
            if left_x < next_px {
                next_px = left_x;
            } else if right_x + right_w > next_px + canvas_w {
                next_px = (right_x + right_w) - canvas_w;
            }
            self.scroll_offset_px = next_px.clamp(0.0, max_px);
            self.sync_index_from_px(canvas_w, gap);
        } else {
            self.reveal_column_with_viewport(col_b, canvas_w, gap);
        }
    }

    /// Remove a pane from the strip, removing the column if empty while preserving
    /// every remaining column's stable `ColumnId`.
    pub fn remove_pane(&mut self, pane_id: PaneId) -> bool {
        let Some(col_idx) = self.find_column_for_pane(pane_id) else {
            return false;
        };
        self.columns[col_idx].remove_pane(pane_id);
        if self.columns[col_idx].panes.is_empty() {
            self.columns.remove(col_idx);
        }
        if self.focused_pane == Some(pane_id) {
            let next_focus = self
                .columns
                .get(col_idx)
                .and_then(|c| c.remembered_or_first_pane())
                .or_else(|| {
                    col_idx
                        .checked_sub(1)
                        .and_then(|i| self.columns.get(i))
                        .and_then(|c| c.remembered_or_first_pane())
                })
                .or_else(|| self.columns.last().and_then(|c| c.remembered_or_first_pane()));
            self.focused_pane = next_focus;
            if let Some(pid) = next_focus {
                let _ = self.focus_pane(pid);
            }
        }
        let cw = self.effective_viewport_width(None);
        let gap = self.reference_gap_px;
        let max_px = self.max_scroll_offset_px(cw, gap);
        self.scroll_offset_px = self.scroll_offset_px.clamp(0.0, max_px);
        self.sync_index_from_px(cw, gap);
        true
    }

    /// Find column index containing `pane_id`.
    pub fn find_column_for_pane(&self, pane_id: PaneId) -> Option<usize> {
        self.columns.iter().position(|col| col.contains_pane(pane_id))
    }

    /// Find `(col_idx, row_idx)` for `pane_id`.
    pub fn find_pane_position(&self, pane_id: PaneId) -> Option<(usize, usize)> {
        for (col_idx, col) in self.columns.iter().enumerate() {
            if let Some(row_idx) = col.panes.iter().position(|&p| p == pane_id) {
                return Some((col_idx, row_idx));
            }
        }
        None
    }

    /// Find column index by stable `ColumnId`.
    #[cfg(test)]
    pub fn find_column_by_id(&self, id: ColumnId) -> Option<usize> {
        self.columns.iter().position(|col| col.id == id)
    }

    /// Return true if the strip contains `pane_id`.
    pub fn contains_pane(&self, pane_id: PaneId) -> bool {
        self.find_column_for_pane(pane_id).is_some()
    }

    /// Set the focused pane, recording column memory and revealing its column with minimal scroll delta.
    pub fn focus_pane(&mut self, pane_id: PaneId) -> bool {
        let Some(col_idx) = self.find_column_for_pane(pane_id) else {
            return false;
        };
        self.columns[col_idx].focus_pane(pane_id);
        self.focused_pane = Some(pane_id);
        self.ensure_column_visible(col_idx);
        true
    }

    /// Scroll offset index clamp so `scroll_offset` never exceeds `column_count.saturating_sub(2)`.
    pub fn max_scroll_offset(&self) -> usize {
        self.columns.len().saturating_sub(2)
    }

    /// Reveal `col_idx` using minimal-delta continuous scroll offset.
    pub fn ensure_column_visible(&mut self, col_idx: usize) {
        let cw = self.effective_viewport_width(None);
        let gap = self.reference_gap_px;
        self.reveal_column_with_viewport(col_idx, cw, gap);
    }

    /// Add a column to the right of the strip, preserving its `ColumnId` if unique or
    /// allocating a fresh stable `ColumnId` if needed.
    pub fn push_column(&mut self, mut col: Column) {
        if self.columns.iter().any(|c| c.id == col.id) {
            col.id = self.alloc_column_id();
        } else {
            self.next_column_id = self.next_column_id.max(col.id.0 + 1);
        }
        if self.focused_pane.is_none() {
            self.focused_pane = col.last_focused;
        }
        self.columns.push(col);
    }

    /// Insert a new full-height column immediately to the right of `col_idx`, preserving
    /// all existing columns' stable `ColumnId`s and `last_focused` memory, focusing `pane_id`,
    /// and revealing the `(col_idx, insert_idx)` pair when it fits.
    pub fn insert_column_right_of(
        &mut self,
        col_idx: usize,
        pane_id: PaneId,
        width: ColumnWidth,
    ) -> usize {
        if self.columns.len() == 1 && matches!(self.columns[0].width, ColumnWidth::Preset(_)) {
            self.columns[0].width = ColumnWidth::Preset(ColumnWidthPreset::Parent35);
        }
        let insert_idx = (col_idx + 1).min(self.columns.len());
        let new_id = self.alloc_column_id();
        let new_col = Column::new(new_id, pane_id, width);
        self.columns.insert(insert_idx, new_col);
        self.columns[insert_idx].focus_pane(pane_id);
        self.focused_pane = Some(pane_id);
        let cw = self.effective_viewport_width(None);
        let gap = self.reference_gap_px;
        if col_idx < insert_idx
            && matches!(self.columns[col_idx].width, ColumnWidth::CustomRatio(_) | ColumnWidth::CustomPixels(_) | ColumnWidth::FixedPx(_))
        {
            let widths = self.intrinsic_column_widths(cw, gap);
            if widths[col_idx] + gap + widths[insert_idx] > cw {
                let pair_width = ((cw - gap).max(2.0)) / 2.0;
                self.columns[col_idx].width = ColumnWidth::CustomPixels(pair_width);
                self.columns[insert_idx].width = ColumnWidth::CustomPixels(pair_width);
                log::info!("strip: adjusted oversized split pair context={} source={} new={} width={pair_width:.1}", self.context_id, col_idx, insert_idx);
            }
        }
        self.reveal_pair_with_viewport(col_idx.min(insert_idx), insert_idx, cw, gap);
        insert_idx
    }

    /// Insert `new_pane` immediately below `source_pane` in its column's vertical stack
    /// and focus `new_pane`.
    pub fn insert_pane_below_in_column(&mut self, source_pane: PaneId, new_pane: PaneId) -> bool {
        let Some(col_idx) = self.find_column_for_pane(source_pane) else {
            return false;
        };
        if !self.columns[col_idx].insert_pane_below(source_pane, new_pane) {
            return false;
        }
        self.focused_pane = Some(new_pane);
        self.ensure_column_visible(col_idx);
        true
    }

    /// Swap the positions of `pane_a` and `pane_b` in the strip (across columns or within
    /// a stack) without creating/destroying columns or moving whole stacks when a single
    /// stacked pane is swapped horizontally. Focus stays on whichever moved pane was focused.
    pub fn swap_panes(&mut self, pane_a: PaneId, pane_b: PaneId) -> bool {
        if pane_a == pane_b {
            return false;
        }
        let Some((col_a, row_a)) = self.find_pane_position(pane_a) else {
            return false;
        };
        let Some((col_b, row_b)) = self.find_pane_position(pane_b) else {
            return false;
        };
        self.columns[col_a].panes[row_a] = pane_b;
        self.columns[col_b].panes[row_b] = pane_a;

        if col_a == col_b {
            if row_a < self.columns[col_a].heights.len()
                && row_b < self.columns[col_a].heights.len()
            {
                self.columns[col_a].heights.swap(row_a, row_b);
            }
        } else {
            if self.columns[col_a].last_focused == Some(pane_a) {
                self.columns[col_a].last_focused = Some(pane_b);
            }
            if self.columns[col_b].last_focused == Some(pane_b) {
                self.columns[col_b].last_focused = Some(pane_a);
            }
        }

        if self.focused_pane == Some(pane_a) {
            self.columns[col_b].last_focused = Some(pane_a);
            self.ensure_column_visible(col_b);
        } else if self.focused_pane == Some(pane_b) {
            self.columns[col_a].last_focused = Some(pane_b);
            self.ensure_column_visible(col_a);
        }
        true
    }

    /// Set a custom width on column `col_idx`.
    #[cfg(test)]
    pub fn set_column_width(&mut self, col_idx: usize, width: ColumnWidth) -> bool {
        if let Some(col) = self.columns.get_mut(col_idx) {
            col.width = width;
            true
        } else {
            false
        }
    }

    /// Reset all column widths in the strip to their default presets (col 0: 35%, others: 65%).
    #[cfg(test)]
    pub fn reset_column_widths(&mut self) {
        for (idx, col) in self.columns.iter_mut().enumerate() {
            let preset = if idx == 0 {
                ColumnWidthPreset::Parent35
            } else {
                ColumnWidthPreset::Child65
            };
            col.width = ColumnWidth::Preset(preset);
        }
        let cw = self.effective_viewport_width(None);
        let gap = self.reference_gap_px;
        let max_px = self.max_scroll_offset_px(cw, gap);
        self.scroll_offset_px = self.scroll_offset_px.clamp(0.0, max_px);
        self.sync_index_from_px(cw, gap);
    }




}
