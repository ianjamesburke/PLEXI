//! Spatial page-grid navigation.
//!
//! Implements the 2-D page grid on top of the existing `contexts` Vec.
//! Each `Context` carries `(grid_x, grid_y)` coordinates. This module
//! provides `PlexiApp` methods for navigating between pages and finding
//! nearby pages after deletion. Page *creation* lives in
//! `pane_ops/workspace.rs` (alongside `new_context`) so it can access
//! the `pub(super)` `create_single_pane_tree` helper.
//!
//! # Navigation vs. Creation — the invariant
//!
//! `navigate_page` is a **pure navigation** function: it only switches
//! `active_context` to an *existing* page. It never creates. The creation
//! policy (e.g. "new rows start at column 0") lives exclusively in
//! `navigate_page`. Never put creation semantics into `navigate_page` or
//! `find_navigation_target`.
//!
//! This separation is enforced by `find_navigation_target` being a free
//! function with no access to `&mut PlexiApp`. Unit tests in this module
//! cover both contracts independently; any regression in either will
//! immediately fail a test.

use crate::app::PlexiApp;

/// Pure navigation target search — no application state, fully unit-testable.
///
/// `pages` is a flat slice of `(grid_x, grid_y, workspace_id)` where the
/// slice index matches the context index in `PlexiApp::contexts`.
///
/// **Horizontal** (`dx != 0`): exact column match in the current row.
/// **Vertical** (`dy != 0`): closest-by-column in the target row, preferring
/// the column recorded in `last_page_x_per_row` for that row (the most
/// recently visited column), falling back to `cur_x` if the row is unvisited.
///
/// Returns `None` if no page exists in the target direction.
fn find_navigation_target(
    pages: &[(u32, u32, u64)],
    workspace_id: u64,
    cur_x: u32,
    cur_y: u32,
    dx: i32,
    dy: i32,
    last_page_x_per_row: &std::collections::HashMap<u32, u32>,
) -> Option<usize> {
    if dx != 0 {
        let tx = cur_x as i32 + dx;
        if tx < 0 {
            return None;
        }
        let tx = tx as u32;
        pages
            .iter()
            .position(|&(gx, gy, ws)| gx == tx && gy == cur_y && ws == workspace_id)
    } else if dy != 0 {
        let ty = cur_y as i32 + dy;
        if ty < 0 {
            return None;
        }
        let ty = ty as u32;
        let preferred_x = last_page_x_per_row.get(&ty).copied().unwrap_or(cur_x);
        pages
            .iter()
            .enumerate()
            .filter(|(_, &(_, gy, ws))| gy == ty && ws == workspace_id)
            .min_by_key(|(_, &(gx, _, _))| (gx as i64 - preferred_x as i64).unsigned_abs())
            .map(|(i, _)| i)
    } else {
        None
    }
}

impl PlexiApp {
    /// Navigate to the adjacent page in the given direction.
    ///
    /// `dx` = +1 right, -1 left; `dy` = +1 down, -1 up.
    ///
    /// **Horizontal**: exact column match in the current row.
    /// **Vertical**: closest-by-column in the target row, biased toward the
    /// most recently visited column in that row (`last_page_x_per_row`).
    ///
    /// Does **not** create pages — silently no-ops at the grid boundary.
    pub(crate) fn navigate_page(&mut self, dx: i32, dy: i32) {
        let cur_x = self.windows[self.active_window].grid_x;
        let cur_y = self.windows[self.active_window].grid_y;
        let ws_id = self.router.active().context_id;

        let pages: Vec<(u32, u32, u64)> = self
            .windows
            .iter()
            .map(|w| (w.grid_x, w.grid_y, w.context_id))
            .collect();

        // Record the current column before leaving this row so that returning
        // to it later prefers the same column (the minimap trail indicator also
        // reads this map).
        if dy != 0 {
            self.last_page_x_per_row.insert(cur_y, cur_x);
        }

        let new_idx =
            find_navigation_target(&pages, ws_id, cur_x, cur_y, dx, dy, &self.last_page_x_per_row);

        if let Some(idx) = new_idx {
            self.active_window = idx;
            let w = &self.windows[idx];
            self.last_page_x_per_row.insert(w.grid_y, w.grid_x);
            self.context_active_window.insert(ws_id, w.window_id);
            let wid = w.window_id;
            self.record_context_visit(wid);
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
        let ws_id = self.router.active().context_id;
        self.windows
            .iter()
            .enumerate()
            .filter(|(_, c)| c.context_id == ws_id)
            .min_by_key(|(_, c)| {
                let dx = (c.grid_x as i64 - removed_x as i64).unsigned_abs();
                let dy = (c.grid_y as i64 - removed_y as i64).unsigned_abs();
                dx + dy
            })
            .map(|(i, _)| i)
            .unwrap_or(0)
    }
}

// ── Unit tests ───────────────────────────────────────────────────────────────
//
// These tests cover `find_navigation_target` directly. They are the
// machine-enforced contract for navigation behaviour. If you change
// `navigate_page` or `find_navigation_target` and a test breaks, you have
// changed a documented behaviour — update the test intentionally rather than
// silently.
//
// Creation policy (e.g. "vertical creates start at col 0") lives in
// `pane_ops/workspace.rs` (create_page_at), not in this function.

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Build a page list from (grid_x, grid_y) pairs, all in workspace 1.
    fn pages(grid: &[(u32, u32)]) -> Vec<(u32, u32, u64)> {
        grid.iter().map(|&(x, y)| (x, y, 1u64)).collect()
    }

    // ── Vertical navigation ───────────────────────────────────────────────

    #[test]
    fn up_prefers_same_column_when_unvisited() {
        // row 0: [0,0] [1,0] [2,0]
        // row 1: [0,1]  ← current
        // Going up with no memory → falls back to cur_x=0 → lands on (0,0).
        let ps = pages(&[(0, 0), (1, 0), (2, 0), (0, 1)]);
        let result = find_navigation_target(&ps, 1, 0, 1, 0, -1, &HashMap::new());
        assert_eq!(result, Some(0)); // (0,0)
    }

    #[test]
    fn up_uses_last_page_x_per_row_memory() {
        // row 0: [0,0] [1,0] [2,0]
        // row 1: [0,1]  ← current
        // Memory says row 0 was last at x=1 → should land on (1,0).
        // This is the exact scenario from the reported bug: user was at (1,0),
        // navigated down to (0,1), then pressed up — should return to (1,0).
        let ps = pages(&[(0, 0), (1, 0), (2, 0), (0, 1)]);
        let mut mem = HashMap::new();
        mem.insert(0u32, 1u32);
        let result = find_navigation_target(&ps, 1, 0, 1, 0, -1, &mem);
        assert_eq!(result, Some(1)); // (1,0)
    }

    #[test]
    fn down_prefers_same_column_when_unvisited() {
        // row 0: [0,0]  ← current
        // row 1: [0,1] [1,1] [2,1]
        // Going down with no memory → falls back to cur_x=0 → lands on (0,1).
        let ps = pages(&[(0, 0), (0, 1), (1, 1), (2, 1)]);
        let result = find_navigation_target(&ps, 1, 0, 0, 0, 1, &HashMap::new());
        assert_eq!(result, Some(1)); // (0,1)
    }

    #[test]
    fn down_uses_last_page_x_per_row_memory() {
        // row 0: [0,0]  ← current
        // row 1: [0,1] [1,1]
        // Memory says row 1 was last at x=1 → should land on (1,1).
        let ps = pages(&[(0, 0), (0, 1), (1, 1)]);
        let mut mem = HashMap::new();
        mem.insert(1u32, 1u32);
        let result = find_navigation_target(&ps, 1, 0, 0, 0, 1, &mem);
        assert_eq!(result, Some(2)); // (1,1)
    }

    #[test]
    fn vertical_picks_closest_when_memory_misses() {
        // row 0: [0,0] [3,0]   — gap; nothing at x=1 or x=2
        // row 1: [2,1]  ← current; no memory for row 0
        // Falls back to cur_x=2; closest is (3,0) at distance 1 vs (0,0) at 2.
        let ps = pages(&[(0, 0), (3, 0), (2, 1)]);
        let result = find_navigation_target(&ps, 1, 2, 1, 0, -1, &HashMap::new());
        assert_eq!(result, Some(1)); // (3,0)
    }

    #[test]
    fn up_at_top_row_returns_none() {
        let ps = pages(&[(0, 0)]);
        assert_eq!(
            find_navigation_target(&ps, 1, 0, 0, 0, -1, &HashMap::new()),
            None
        );
    }

    #[test]
    fn down_with_no_row_below_returns_none() {
        let ps = pages(&[(0, 0)]);
        assert_eq!(
            find_navigation_target(&ps, 1, 0, 0, 0, 1, &HashMap::new()),
            None
        );
    }

    // ── Horizontal navigation ─────────────────────────────────────────────

    #[test]
    fn right_exact_match() {
        // row 0: [0,0] [1,0] [2,0]  ← at (0,0)
        let ps = pages(&[(0, 0), (1, 0), (2, 0)]);
        let result = find_navigation_target(&ps, 1, 0, 0, 1, 0, &HashMap::new());
        assert_eq!(result, Some(1)); // (1,0)
    }

    #[test]
    fn left_exact_match() {
        let ps = pages(&[(0, 0), (1, 0), (2, 0)]);
        let result = find_navigation_target(&ps, 1, 2, 0, -1, 0, &HashMap::new());
        assert_eq!(result, Some(1)); // (1,0)
    }

    #[test]
    fn left_at_first_column_returns_none() {
        let ps = pages(&[(0, 0), (1, 0)]);
        assert_eq!(
            find_navigation_target(&ps, 1, 0, 0, -1, 0, &HashMap::new()),
            None
        );
    }

    #[test]
    fn horizontal_skips_gap_returns_none() {
        // [0,0] [2,0] — no page at (1,0)
        let ps = pages(&[(0, 0), (2, 0)]);
        // From (0,0), right → looks for (1,0) → not found.
        assert_eq!(
            find_navigation_target(&ps, 1, 0, 0, 1, 0, &HashMap::new()),
            None
        );
    }

    // ── Workspace isolation ───────────────────────────────────────────────

    #[test]
    fn ignores_pages_in_other_workspaces() {
        // workspace 1: [0,0]
        // workspace 2: [1,0]  ← in the same row/col but different workspace
        let ps = vec![(0u32, 0u32, 1u64), (1u32, 0u32, 2u64)];
        // From (0,0) in workspace 1, going right should find nothing.
        assert_eq!(
            find_navigation_target(&ps, 1, 0, 0, 1, 0, &HashMap::new()),
            None
        );
    }
}
