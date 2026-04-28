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
    /// **Horizontal moves**: exact match only; updates `preferred_page_x` to
    /// the x of the page landed on.
    ///
    /// **Vertical moves**: first tries the exact `(preferred_page_x, target_y)`
    /// position, then falls back to the page on `target_y` whose `grid_x` is
    /// closest to `preferred_page_x`. This lets navigation reach rows that
    /// have no page at the current column while preserving the remembered
    /// horizontal position across multiple up/down presses.
    pub(crate) fn navigate_page(&mut self, dx: i32, dy: i32) {
        let active = &self.contexts[self.active_context];
        let cur_x = active.grid_x;
        let cur_y = active.grid_y;

        if dx != 0 {
            // Horizontal: exact match required.
            let target_x = cur_x as i32 + dx;
            if target_x < 0 {
                return;
            }
            let tx = target_x as u32;
            if let Some(idx) = self.contexts.iter().position(|c| c.grid_x == tx && c.grid_y == cur_y) {
                self.active_context = idx;
                self.preferred_page_x = tx;
            }
        } else if dy != 0 {
            // Vertical: land on the page at target_y closest to preferred_page_x.
            let target_y = cur_y as i32 + dy;
            if target_y < 0 {
                return;
            }
            let ty = target_y as u32;
            let preferred_x = self.preferred_page_x;
            if let Some((idx, _)) = self.contexts.iter().enumerate()
                .filter(|(_, c)| c.grid_y == ty)
                .min_by_key(|(_, c)| (c.grid_x as i64 - preferred_x as i64).unsigned_abs())
            {
                self.active_context = idx;
            }
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
