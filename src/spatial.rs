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
    /// Navigate to the page adjacent to the current one in the given
    /// direction. No-op if there is no page in that direction.
    ///
    /// `dx` = +1 for right, -1 for left; `dy` = +1 for down, -1 for up.
    pub(crate) fn navigate_page(&mut self, dx: i32, dy: i32) {
        let active = &self.contexts[self.active_context];
        let target_x = active.grid_x as i32 + dx;
        let target_y = active.grid_y as i32 + dy;

        if target_x < 0 || target_y < 0 {
            return;
        }

        let tx = target_x as u32;
        let ty = target_y as u32;

        if let Some(idx) = self.contexts.iter().position(|c| c.grid_x == tx && c.grid_y == ty) {
            self.active_context = idx;
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
