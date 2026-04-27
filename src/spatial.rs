//! Spatial page-grid navigation.
//!
//! Implements the 2-D page grid on top of the existing `contexts` Vec.
//! Each `Context` carries `(grid_x, grid_y)` coordinates. This module
//! provides `PlexiApp` methods for navigating between pages and finding
//! nearby pages after deletion. Page *creation* lives in
//! `pane_ops/workspace.rs` (alongside `new_context`) so it can access
//! the `pub(super)` `create_single_pane_tree` helper.

use crate::app::PlexiApp;

impl PlexiApp {
    /// Navigate to the adjacent page in the given direction.
    ///
    /// `dx` = +1 right, -1 left; `dy` = +1 down, -1 up.
    ///
    /// **Horizontal moves**: exact match only.
    ///
    /// **Vertical moves**: consult `last_page_x_per_row` to land on the most
    /// recently visited page in the target row. Falls back to the page whose
    /// `grid_x` is closest to the current column when the target row has never
    /// been visited. Records the current and destination positions in
    /// `last_page_x_per_row` on every successful move.
    pub(crate) fn navigate_page(&mut self, dx: i32, dy: i32) {
        let cur_x = self.contexts[self.active_context].grid_x;
        let cur_y = self.contexts[self.active_context].grid_y;

        let new_idx = if dx != 0 {
            // Horizontal: exact match only.
            let tx = cur_x as i32 + dx;
            if tx < 0 {
                return;
            }
            self.contexts
                .iter()
                .position(|c| c.grid_x == tx as u32 && c.grid_y == cur_y)
        } else if dy != 0 {
            let ty = cur_y as i32 + dy;
            if ty < 0 {
                return;
            }
            let ty = ty as u32;
            // Record where we are on the current row before leaving.
            self.last_page_x_per_row.insert(cur_y, cur_x);
            // Use per-row history; fall back to cur_x if this row was never visited.
            let preferred_x = self.last_page_x_per_row.get(&ty).copied().unwrap_or(cur_x);
            self.contexts
                .iter()
                .enumerate()
                .filter(|(_, c)| c.grid_y == ty)
                .min_by_key(|(_, c)| (c.grid_x as i64 - preferred_x as i64).unsigned_abs())
                .map(|(i, _)| i)
        } else {
            return;
        };

        if let Some(idx) = new_idx {
            self.active_context = idx;
            let c = &self.contexts[idx];
            // Record where we landed on the destination row.
            self.last_page_x_per_row.insert(c.grid_y, c.grid_x);
        }
    }

    /// After a context is deleted, find the nearest remaining context by grid
    /// proximity (smallest Manhattan distance to the removed page's
    /// coordinates). Returns the `active_context` index to switch to.
    pub(crate) fn nearest_context_after_delete(
        &self,
        removed_x: u32,
        removed_y: u32,
    ) -> usize {
        self.contexts
            .iter()
            .enumerate()
            .min_by_key(|(_, c)| {
                let dx = (c.grid_x as i64 - removed_x as i64).unsigned_abs();
                let dy = (c.grid_y as i64 - removed_y as i64).unsigned_abs();
                dx + dy
            })
            .map(|(i, _)| i)
            .unwrap_or(0)
    }
}
