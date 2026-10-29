//! Compatibility model for persisted strip-era workspace data and context summaries.
//! Pane geometry and split shares are owned by each window's tile tree.

use crate::app::zoom::ColumnWidthPreset;
use crate::spatial::tiling::PaneId;
use crate::ui::style;

fn default_strip_gap() -> f32 {
    4.0
}

/// Unique, stable identifier for a column within a strip.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
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
    /// Legacy synchronization marker retained for saved-workspace compatibility.
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
            .or(self
                .reference_width_px
                .filter(|w| w.is_finite() && *w > 0.0))
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
                .or_else(|| {
                    self.columns
                        .last()
                        .and_then(|c| c.remembered_or_first_pane())
                });
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

    /// Extract selected panes in column/row order, retaining widths and height shares.
    pub fn extract_panes(
        &mut self,
        selected: &std::collections::HashSet<PaneId>,
        context_id: u64,
    ) -> Self {
        let mut extracted = Self::new(context_id);
        extracted.reference_width_px = self.reference_width_px;
        extracted.reference_gap_px = self.reference_gap_px;
        for column in &self.columns {
            let mut kept = column.clone();
            let excluded: Vec<_> = column
                .panes
                .iter()
                .copied()
                .filter(|id| !selected.contains(id))
                .collect();
            for id in excluded {
                kept.remove_pane(id);
            }
            if !kept.panes.is_empty() {
                extracted.push_column(kept);
            }
        }
        if let Some(focused) = self.focused_pane.filter(|id| selected.contains(id)) {
            extracted.focus_pane(focused);
        }
        for id in selected {
            self.remove_pane(*id);
        }
        extracted
    }

    /// Find column index containing `pane_id`.
    pub fn find_column_for_pane(&self, pane_id: PaneId) -> Option<usize> {
        self.columns
            .iter()
            .position(|col| col.contains_pane(pane_id))
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
}
