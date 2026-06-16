//! Encapsulates the active-context index so it can only be mutated through
//! controlled methods. Direct field assignment (`active_context = n`) is a
//! compile error outside this module, which makes the class of "forgot to call
//! switch_workspace" bugs structurally impossible.
//!
//! Navigation (with minimap save/restore) → `PlexiApp::switch_workspace`
//! Structural ops (create/delete/reorder) → methods on this struct

use crate::host::context::Context;
use egui_tiles::TileId;

pub(crate) struct WorkspaceRouter {
    active: usize,
    contexts: Vec<Context>,
    /// Depth navigation stack. Each entry records the context_id, window_id,
    /// and focused tile to restore when zooming back out.
    pub(crate) depth_stack: Vec<(u64, u64, Option<TileId>)>,
}

const MAX_CONTEXT_DEPTH: u32 = 3;

impl WorkspaceRouter {
    pub(crate) fn new(contexts: Vec<Context>, active: usize) -> Self {
        debug_assert!(
            contexts.is_empty() || active < contexts.len(),
            "Active index out of bounds"
        );
        Self {
            active,
            contexts,
            depth_stack: Vec::new(),
        }
    }

    // ── Read ─────────────────────────────────────────────────────────────────

    pub(crate) fn active_idx(&self) -> usize {
        self.active
    }

    pub(crate) fn active(&self) -> &Context {
        &self.contexts[self.active]
    }

    pub(crate) fn get(&self, idx: usize) -> &Context {
        &self.contexts[idx]
    }

    pub(crate) fn get_mut(&mut self, idx: usize) -> &mut Context {
        &mut self.contexts[idx]
    }

    pub(crate) fn len(&self) -> usize {
        self.contexts.len()
    }

    pub(crate) fn iter(&self) -> std::slice::Iter<'_, Context> {
        self.contexts.iter()
    }

    pub(crate) fn as_slice(&self) -> &[Context] {
        &self.contexts
    }

    pub(crate) fn position<F: Fn(&Context) -> bool>(&self, f: F) -> Option<usize> {
        self.contexts.iter().position(f)
    }

    // ── Structural mutation (create / delete) ────────────────────────────────

    pub(crate) fn push(&mut self, ctx: Context) {
        self.contexts.push(ctx);
    }

    /// Insert `ctx` immediately after all existing descendants of `parent_id`.
    /// Descendants are detected by walking forward from the parent position while
    /// `depth > parent_depth`. Falls back to `push` if the parent is not found.
    /// Adjusts the active index to remain coherent.
    pub(crate) fn insert_after_subtree(&mut self, parent_id: u64, ctx: Context) {
        let Some(parent_pos) = self.contexts.iter().position(|c| c.context_id == parent_id) else {
            self.contexts.push(ctx);
            return;
        };
        let parent_depth = self.contexts[parent_pos].depth;
        let mut insert_pos = parent_pos + 1;
        while insert_pos < self.contexts.len() && self.contexts[insert_pos].depth > parent_depth {
            insert_pos += 1;
        }
        self.contexts.insert(insert_pos, ctx);
        if self.active >= insert_pos {
            self.active += 1;
        }
    }

    /// Set active to the last context (call after push for new-context flow).
    pub(crate) fn activate_last(&mut self) {
        self.active = self.contexts.len().saturating_sub(1);
    }

    /// Remove at `idx` and adjust the active index to stay coherent.
    pub(crate) fn remove_at(&mut self, idx: usize) -> Context {
        let ctx = self.contexts.remove(idx);
        if self.active >= self.contexts.len() {
            self.active = self.contexts.len().saturating_sub(1);
        } else if self.active > idx {
            self.active -= 1;
        }
        ctx
    }

    /// Raw set — used by `switch_workspace` (which handles minimap save/restore)
    /// and post-delete sync when the exact target index is known.
    /// Never call from action handlers; call `PlexiApp::switch_workspace` instead.
    pub(crate) fn set_active(&mut self, idx: usize) {
        debug_assert!(
            idx < self.contexts.len(),
            "Target active index out of bounds"
        );
        self.active = idx;
    }

    // ── Reorder ops (sidebar drag / move-to-top etc.) ────────────────────────
    // Each op maintains active coherence atomically.

    pub(crate) fn swap_tracking_active(&mut self, a: usize, b: usize) {
        self.contexts.swap(a, b);
        if self.active == a {
            self.active = b;
        } else if self.active == b {
            self.active = a;
        }
    }

    pub(crate) fn move_to_front_tracking_active(&mut self, idx: usize) {
        let ctx = self.contexts.remove(idx);
        self.contexts.insert(0, ctx);
        if self.active == idx {
            self.active = 0;
        } else if self.active < idx {
            self.active += 1;
        }
    }

    pub(crate) fn move_to_back_tracking_active(&mut self, idx: usize) {
        let last = self.contexts.len() - 1;
        let ctx = self.contexts.remove(idx);
        self.contexts.push(ctx);
        if self.active == idx {
            self.active = last;
        } else if self.active > idx {
            self.active -= 1;
        }
    }

    pub(crate) fn subtree_end(&self, idx: usize) -> usize {
        let base_depth = self.contexts[idx].depth;
        let mut end = idx + 1;
        while end < self.contexts.len() && self.contexts[end].depth > base_depth {
            end += 1;
        }
        end
    }

    fn is_descendant_id(&self, descendant_id: u64, ancestor_id: u64) -> bool {
        let mut current = self
            .contexts
            .iter()
            .find(|ctx| ctx.context_id == descendant_id)
            .and_then(|ctx| ctx.parent_id);
        while let Some(parent_id) = current {
            if parent_id == ancestor_id {
                return true;
            }
            current = self
                .contexts
                .iter()
                .find(|ctx| ctx.context_id == parent_id)
                .and_then(|ctx| ctx.parent_id);
        }
        false
    }

    pub(crate) fn can_move_subtree_to_parent(
        &self,
        idx: usize,
        new_parent_id: Option<u64>,
    ) -> bool {
        if idx >= self.contexts.len() {
            return false;
        }
        let moving_id = self.contexts[idx].context_id;
        let old_parent_id = self.contexts[idx].parent_id;
        if old_parent_id == new_parent_id {
            return false;
        }
        let new_depth = match new_parent_id {
            Some(parent_id) => {
                if parent_id == moving_id || self.is_descendant_id(parent_id, moving_id) {
                    return false;
                }
                let Some(parent) = self.contexts.iter().find(|ctx| ctx.context_id == parent_id)
                else {
                    return false;
                };
                parent.depth + 1
            }
            None => 0,
        };
        let end = self.subtree_end(idx);
        let old_depth = self.contexts[idx].depth;
        let deepest_relative = self.contexts[idx..end]
            .iter()
            .map(|ctx| ctx.depth.saturating_sub(old_depth))
            .max()
            .unwrap_or(0);
        new_depth + deepest_relative <= MAX_CONTEXT_DEPTH
    }

    pub(crate) fn move_subtree_to_parent(
        &mut self,
        idx: usize,
        new_parent_id: Option<u64>,
    ) -> bool {
        if !self.can_move_subtree_to_parent(idx, new_parent_id) {
            return false;
        }
        let old_parent_id = self.contexts[idx].parent_id;

        let active_id = self.contexts[self.active].context_id;
        let end = self.subtree_end(idx);
        let old_depth = self.contexts[idx].depth;
        let mut subtree: Vec<Context> = self.contexts.drain(idx..end).collect();

        let new_depth = match new_parent_id {
            Some(parent_id) => self
                .contexts
                .iter()
                .find(|ctx| ctx.context_id == parent_id)
                .map(|ctx| ctx.depth + 1)
                .unwrap_or(0),
            None => 0,
        };
        let depth_delta = new_depth as i32 - old_depth as i32;
        for ctx in &mut subtree {
            ctx.depth = ((ctx.depth as i32) + depth_delta).max(0) as u32;
        }
        subtree[0].parent_id = new_parent_id;

        let insert_pos = match new_parent_id {
            Some(parent_id) => {
                let parent_pos = self
                    .contexts
                    .iter()
                    .position(|ctx| ctx.context_id == parent_id)
                    .expect("validated new parent must still exist after drain");
                let parent_depth = self.contexts[parent_pos].depth;
                let mut pos = parent_pos + 1;
                while pos < self.contexts.len() && self.contexts[pos].depth > parent_depth {
                    pos += 1;
                }
                pos
            }
            None => {
                if let Some(former_parent_id) = old_parent_id {
                    if let Some(parent_pos) = self
                        .contexts
                        .iter()
                        .position(|ctx| ctx.context_id == former_parent_id)
                    {
                        let parent_depth = self.contexts[parent_pos].depth;
                        let mut pos = parent_pos + 1;
                        while pos < self.contexts.len() && self.contexts[pos].depth > parent_depth {
                            pos += 1;
                        }
                        pos
                    } else {
                        self.contexts.len()
                    }
                } else {
                    self.contexts.len()
                }
            }
        };

        self.contexts.splice(insert_pos..insert_pos, subtree);
        self.active = self
            .contexts
            .iter()
            .position(|ctx| ctx.context_id == active_id)
            .expect("move_subtree_to_parent: active context survives move");
        true
    }

    pub(crate) fn set_subtree_parked(&mut self, idx: usize, parked: bool) -> bool {
        if idx >= self.contexts.len() {
            return false;
        }
        let end = self.subtree_end(idx);
        for ctx in &mut self.contexts[idx..end] {
            ctx.parked = parked;
        }
        true
    }

    pub(crate) fn move_subtree_to_index(&mut self, idx: usize, insert_pos: usize) -> bool {
        if idx >= self.contexts.len() || insert_pos > self.contexts.len() {
            return false;
        }
        let end = self.subtree_end(idx);
        if (idx..end).contains(&insert_pos) {
            return false;
        }
        let active_id = self.contexts[self.active].context_id;
        let subtree_len = end - idx;
        let subtree: Vec<Context> = self.contexts.drain(idx..end).collect();
        let adjusted_insert = if insert_pos > idx {
            insert_pos - subtree_len
        } else {
            insert_pos
        };
        self.contexts
            .splice(adjusted_insert..adjusted_insert, subtree);
        self.active = self
            .contexts
            .iter()
            .position(|ctx| ctx.context_id == active_id)
            .expect("move_subtree_to_index: active context survives move");
        true
    }

    // ── Depth stack (fractal zoom navigation) ───────────────────────────────

    pub(crate) fn push_depth(
        &mut self,
        context_id: u64,
        window_id: u64,
        focused_tile: Option<TileId>,
    ) {
        self.depth_stack.push((context_id, window_id, focused_tile));
    }

    pub(crate) fn pop_depth(&mut self) -> Option<(u64, u64, Option<TileId>)> {
        self.depth_stack.pop()
    }

    pub(crate) fn current_depth(&self) -> usize {
        self.depth_stack.len()
    }

    /// Retain only depth_stack entries whose context_id satisfies `keep`.
    /// Used by `delete_context` to drop entries pointing to deleted contexts.
    pub(crate) fn retain_depth_stack<F: Fn(u64) -> bool>(&mut self, keep: F) {
        self.depth_stack.retain(|(ctx_id, _, _)| keep(*ctx_id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_ctx(id: u64, parent_id: Option<u64>, depth: u32) -> Context {
        Context {
            name: format!("ctx{id}"),
            path: PathBuf::from("/tmp"),
            root: None,
            description: None,
            context_id: id,
            parent_id,
            depth,
            parked: false,
        }
    }

    #[test]
    fn depth_stack_push_pop() {
        let mut router = WorkspaceRouter::new(vec![make_ctx(1, None, 0)], 0);
        assert_eq!(router.current_depth(), 0);
        router.push_depth(1, 10, None);
        assert_eq!(router.current_depth(), 1);
        let popped = router.pop_depth();
        assert_eq!(popped, Some((1, 10, None)));
        assert_eq!(router.current_depth(), 0);
    }

    #[test]
    fn depth_stack_empty_pop_returns_none() {
        let mut router = WorkspaceRouter::new(vec![make_ctx(1, None, 0)], 0);
        assert_eq!(router.pop_depth(), None);
    }

    #[test]
    fn retain_depth_stack_drops_matching_entries() {
        let mut router = WorkspaceRouter::new(vec![make_ctx(1, None, 0)], 0);
        router.push_depth(1, 10, None);
        router.push_depth(2, 20, None);
        router.push_depth(3, 30, None);
        assert_eq!(router.current_depth(), 3);
        router.retain_depth_stack(|cid| cid != 2);
        assert_eq!(router.current_depth(), 2);
        let remaining: Vec<u64> = router.depth_stack.iter().map(|(c, _, _)| *c).collect();
        assert_eq!(remaining, vec![1, 3]);
    }

    #[test]
    fn insert_after_subtree_no_existing_children() {
        // [A(d0)] -> insert child B(d1) under A -> [A, B]
        let mut router = WorkspaceRouter::new(vec![make_ctx(1, None, 0)], 0);
        router.insert_after_subtree(1, make_ctx(2, Some(1), 1));
        let ids: Vec<u64> = router.contexts.iter().map(|c| c.context_id).collect();
        assert_eq!(ids, vec![1, 2]);
    }

    #[test]
    fn insert_after_subtree_after_existing_sibling() {
        // [A(d0), B(d1)] -> insert C(d1) under A -> [A, B, C]
        let a = make_ctx(1, None, 0);
        let b = make_ctx(2, Some(1), 1);
        let mut router = WorkspaceRouter::new(vec![a, b], 0);
        router.insert_after_subtree(1, make_ctx(3, Some(1), 1));
        let ids: Vec<u64> = router.contexts.iter().map(|c| c.context_id).collect();
        assert_eq!(ids, vec![1, 2, 3]);
    }

    #[test]
    fn insert_after_subtree_skips_deep_descendants() {
        // [A(d0), B(d1), C(d2)] -> insert D(d1) under A -> [A, B, C, D]
        // C is a grandchild of A (via B), so D lands after C.
        let a = make_ctx(1, None, 0);
        let b = make_ctx(2, Some(1), 1);
        let c = make_ctx(3, Some(2), 2);
        let mut router = WorkspaceRouter::new(vec![a, b, c], 0);
        router.insert_after_subtree(1, make_ctx(4, Some(1), 1));
        let ids: Vec<u64> = router.contexts.iter().map(|c| c.context_id).collect();
        assert_eq!(ids, vec![1, 2, 3, 4]);
    }

    #[test]
    fn insert_after_subtree_between_top_level_contexts() {
        // [A(d0), B(d0)] -> insert C(d1) under A -> [A, C, B]
        let a = make_ctx(1, None, 0);
        let b = make_ctx(2, None, 0);
        let mut router = WorkspaceRouter::new(vec![a, b], 0);
        router.insert_after_subtree(1, make_ctx(3, Some(1), 1));
        let ids: Vec<u64> = router.contexts.iter().map(|c| c.context_id).collect();
        assert_eq!(ids, vec![1, 3, 2]);
    }

    #[test]
    fn insert_after_subtree_adjusts_active_index() {
        // [A(d0), B(d0)] active=1 (B) -> insert C under A -> [A, C, B] active=2
        let a = make_ctx(1, None, 0);
        let b = make_ctx(2, None, 0);
        let mut router = WorkspaceRouter::new(vec![a, b], 1);
        router.insert_after_subtree(1, make_ctx(3, Some(1), 1));
        assert_eq!(router.active_idx(), 2);
        assert_eq!(router.active().context_id, 2);
    }

    #[test]
    fn insert_after_subtree_unknown_parent_falls_back_to_push() {
        let mut router = WorkspaceRouter::new(vec![make_ctx(1, None, 0)], 0);
        router.insert_after_subtree(99, make_ctx(2, None, 0));
        let ids: Vec<u64> = router.contexts.iter().map(|c| c.context_id).collect();
        assert_eq!(ids, vec![1, 2]);
    }

    #[test]
    fn move_subtree_to_parent_keeps_descendants_attached() {
        let mut router = WorkspaceRouter::new(
            vec![
                make_ctx(1, None, 0),
                make_ctx(2, Some(1), 1),
                make_ctx(3, Some(2), 2),
                make_ctx(4, None, 0),
            ],
            1,
        );

        assert!(router.move_subtree_to_parent(3, Some(1)));
        let rows: Vec<(u64, Option<u64>, u32)> = router
            .iter()
            .map(|ctx| (ctx.context_id, ctx.parent_id, ctx.depth))
            .collect();
        assert_eq!(
            rows,
            vec![
                (1, None, 0),
                (2, Some(1), 1),
                (3, Some(2), 2),
                (4, Some(1), 1),
            ]
        );
        assert_eq!(
            router.active().context_id,
            2,
            "active context tracks by identity"
        );
    }

    #[test]
    fn move_subtree_to_parent_promotes_one_level() {
        let mut router = WorkspaceRouter::new(
            vec![
                make_ctx(1, None, 0),
                make_ctx(2, Some(1), 1),
                make_ctx(3, Some(2), 2),
                make_ctx(4, None, 0),
            ],
            2,
        );

        assert!(router.move_subtree_to_parent(2, None));
        let rows: Vec<(u64, Option<u64>, u32)> = router
            .iter()
            .map(|ctx| (ctx.context_id, ctx.parent_id, ctx.depth))
            .collect();
        assert_eq!(
            rows,
            vec![(1, None, 0), (2, Some(1), 1), (3, None, 0), (4, None, 0),]
        );
        assert_eq!(router.active().context_id, 3);
    }

    #[test]
    fn move_subtree_to_parent_rejects_direct_child_cycle() {
        let mut router =
            WorkspaceRouter::new(vec![make_ctx(1, None, 0), make_ctx(2, Some(1), 1)], 0);

        assert!(!router.can_move_subtree_to_parent(0, Some(2)));
        assert!(!router.move_subtree_to_parent(0, Some(2)));
        let rows: Vec<(u64, Option<u64>, u32)> = router
            .iter()
            .map(|ctx| (ctx.context_id, ctx.parent_id, ctx.depth))
            .collect();
        assert_eq!(rows, vec![(1, None, 0), (2, Some(1), 1)]);
    }

    #[test]
    fn move_subtree_to_parent_rejects_grandchild_cycle() {
        let mut router = WorkspaceRouter::new(
            vec![
                make_ctx(1, None, 0),
                make_ctx(2, Some(1), 1),
                make_ctx(3, Some(2), 2),
            ],
            0,
        );

        assert!(!router.can_move_subtree_to_parent(0, Some(3)));
        assert!(!router.move_subtree_to_parent(0, Some(3)));
        let rows: Vec<(u64, Option<u64>, u32)> = router
            .iter()
            .map(|ctx| (ctx.context_id, ctx.parent_id, ctx.depth))
            .collect();
        assert_eq!(rows, vec![(1, None, 0), (2, Some(1), 1), (3, Some(2), 2)]);
    }

    #[test]
    fn move_subtree_to_parent_rejects_moves_past_depth_cap() {
        let mut router = WorkspaceRouter::new(
            vec![
                make_ctx(1, None, 0),
                make_ctx(2, Some(1), 1),
                make_ctx(3, Some(2), 2),
                make_ctx(4, Some(3), 3),
                make_ctx(5, None, 0),
                make_ctx(6, Some(5), 1),
            ],
            0,
        );

        assert!(!router.can_move_subtree_to_parent(1, Some(6)));
        assert!(!router.move_subtree_to_parent(1, Some(6)));
    }

    #[test]
    fn move_subtree_to_index_keeps_subtree_contiguous() {
        let mut router = WorkspaceRouter::new(
            vec![
                make_ctx(1, None, 0),
                make_ctx(2, Some(1), 1),
                make_ctx(3, Some(2), 2),
                make_ctx(4, None, 0),
            ],
            1,
        );

        assert!(router.move_subtree_to_index(0, 4));
        let ids: Vec<u64> = router.iter().map(|ctx| ctx.context_id).collect();
        assert_eq!(ids, vec![4, 1, 2, 3]);
        assert_eq!(router.active().context_id, 2);
    }

    #[test]
    fn set_subtree_parked_updates_descendants() {
        let mut router = WorkspaceRouter::new(
            vec![
                make_ctx(1, None, 0),
                make_ctx(2, Some(1), 1),
                make_ctx(3, None, 0),
            ],
            0,
        );

        assert!(router.set_subtree_parked(0, true));
        let parked: Vec<bool> = router.iter().map(|ctx| ctx.parked).collect();
        assert_eq!(parked, vec![true, true, false]);
    }
}
