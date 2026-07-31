//! Encapsulates the active-context index so it can only be mutated through
//! controlled methods. Direct field assignment (`active_context = n`) is a
//! compile error outside this module, which makes the class of "forgot to call
//! switch_workspace" bugs structurally impossible.
//!
//! Navigation (with minimap save/restore) → `PlexiApp::switch_workspace`
//! Structural ops (create/delete/reorder) → methods on this struct

use crate::host::context::Context;
use egui_tiles::TileId;

/// Where a sidebar reorder sends a top-level context within the active list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContextMove {
    Top,
    Up,
    Down,
    Bottom,
}

pub(crate) struct WorkspaceRouter {
    active: usize,
    contexts: Vec<Context>,
    /// Depth navigation stack. Each entry records the context_id, window_id,
    /// and focused tile to restore when zooming back out.
    pub(crate) depth_stack: Vec<(u64, u64, Option<TileId>)>,
}

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

    /// Indices of the contexts the sidebar is allowed to enumerate, in router
    /// order. The sidebar is top-level navigation only: every slot is a
    /// numbered, keyboard-addressable destination, so a context with a live
    /// parent is never listed. Subcontexts are reached from inside their parent
    /// via its Portal tile, which already carries their rolled-up state.
    ///
    /// A context whose `parent_id` names a context that no longer exists is an
    /// orphan and *is* included — otherwise deleting a parent would strand its
    /// children with no reachable entry point anywhere in the UI.
    pub(crate) fn top_level_order(&self) -> Vec<usize> {
        self.contexts
            .iter()
            .enumerate()
            .filter(|(_, c)| match c.parent_id {
                None => true,
                Some(pid) => !self.contexts.iter().any(|p| p.context_id == pid),
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// The index sequential context cycling should land on, in the requested
    /// direction, wrapping at both ends. `None` when there is nowhere else to go.
    ///
    /// Cycling is a sidebar affordance, so it enumerates exactly the rows the
    /// sidebar draws — top-level and unparked — for the same reason `Cmd+1..9`
    /// routes through `top_level_order`: every navigation surface agrees on one
    /// enumeration. Landing on a subcontext would activate a context with no
    /// row, no number, and no recorded `push_depth` for zoom-out to pop.
    ///
    /// The active context may itself be a subcontext, reached by descending
    /// through a Portal, and therefore absent from the list. The search compares
    /// router indices rather than list positions so that case still resolves to
    /// the nearest master in the requested direction.
    pub(crate) fn cycle_top_level(&self, forward: bool) -> Option<usize> {
        let order: Vec<usize> = self
            .top_level_order()
            .into_iter()
            .filter(|&i| !self.contexts[i].parked)
            .collect();
        let active = self.active;
        let target = if forward {
            order
                .iter()
                .copied()
                .find(|&i| i > active)
                .or_else(|| order.first().copied())
        } else {
            order
                .iter()
                .rev()
                .copied()
                .find(|&i| i < active)
                .or_else(|| order.last().copied())
        };
        target.filter(|&i| i != active)
    }

    /// Expand a top-level-only ordering into a full permutation of `0..len` by
    /// emitting each entry followed by its descendants in router order.
    ///
    /// `apply_order` requires every index exactly once; a sidebar reorder only
    /// ever knows about top-level rows, so it must re-inflate the hidden
    /// subtrees before committing or the omitted subcontexts would be dropped
    /// from the router entirely. Any index not reachable from `top_level` is
    /// appended, so the result is a permutation for any input.
    pub(crate) fn expand_subtrees(&self, top_level: &[usize]) -> Vec<usize> {
        let mut out: Vec<usize> = Vec::with_capacity(self.contexts.len());
        let mut seen = vec![false; self.contexts.len()];
        for &idx in top_level {
            self.push_subtree(idx, &mut out, &mut seen);
        }
        for (i, visited) in seen.iter_mut().enumerate() {
            if !*visited {
                out.push(i);
                *visited = true;
            }
        }
        out
    }

    /// Depth-first emit of `idx` and every descendant, guarding against the
    /// cycles a corrupt `parent_id` chain could otherwise turn into recursion.
    fn push_subtree(&self, idx: usize, out: &mut Vec<usize>, seen: &mut [bool]) {
        if seen[idx] {
            return;
        }
        seen[idx] = true;
        out.push(idx);
        let id = self.contexts[idx].context_id;
        for child in 0..self.contexts.len() {
            if self.contexts[child].parent_id == Some(id) {
                self.push_subtree(child, out, seen);
            }
        }
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

    /// Move a top-level context one slot (or to an end) within the sidebar's
    /// active list, carrying its subtree with it. Returns `false` when the move
    /// is a no-op — `idx` is parked, is a subcontext with no sidebar row, or is
    /// already at the requested end.
    ///
    /// Reordering must go through the visible list, never through raw router
    /// indices: subcontexts sit between top-level entries in `contexts` but have
    /// no sidebar row, so an index-adjacent swap would trade places with an
    /// invisible context and read as a dead menu item.
    pub(crate) fn reorder_top_level(&mut self, idx: usize, mv: ContextMove) -> bool {
        let top_level = self.top_level_order();
        let mut active: Vec<usize> = top_level
            .iter()
            .copied()
            .filter(|&i| !self.contexts[i].parked)
            .collect();
        let parked: Vec<usize> = top_level
            .iter()
            .copied()
            .filter(|&i| self.contexts[i].parked)
            .collect();
        let Some(pos) = active.iter().position(|&i| i == idx) else {
            return false;
        };
        let last = active.len() - 1;
        let new_pos = match mv {
            ContextMove::Top => 0,
            ContextMove::Up => pos.saturating_sub(1),
            ContextMove::Down => (pos + 1).min(last),
            ContextMove::Bottom => last,
        };
        if new_pos == pos {
            return false;
        }
        active.remove(pos);
        active.insert(new_pos, idx);
        active.extend(parked);
        let order = self.expand_subtrees(&active);
        self.apply_order(&order);
        true
    }

    /// Reorder the entire context vec to match `new_order`, a permutation of
    /// the current indices `0..len`. The active context is tracked by identity,
    /// so its index is rewritten to wherever it lands. Used by sidebar drag-drop
    /// to commit a fully re-partitioned (active/parked) ordering atomically.
    pub(crate) fn apply_order(&mut self, new_order: &[usize]) {
        debug_assert_eq!(
            new_order.len(),
            self.contexts.len(),
            "apply_order needs a full permutation"
        );
        let active_id = self.contexts[self.active].context_id;
        let mut slots: Vec<Option<Context>> = self.contexts.drain(..).map(Some).collect();
        let mut reordered: Vec<Context> = Vec::with_capacity(slots.len());
        for &idx in new_order {
            reordered.push(
                slots[idx]
                    .take()
                    .expect("apply_order: each index referenced exactly once"),
            );
        }
        self.contexts = reordered;
        self.active = self
            .contexts
            .iter()
            .position(|c| c.context_id == active_id)
            .expect("apply_order: active context survives reorder");
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
            root: PathBuf::from("/tmp"),
            description: None,
            context_id: id,
            parent_id,
            depth,
            parked: false,
        }
    }

    /// Stint 0241: the sidebar lists master contexts only. A context with a
    /// live parent is never enumerated, so it can never be numbered or rendered.
    #[test]
    fn top_level_order_excludes_subcontexts() {
        let router = WorkspaceRouter::new(
            vec![
                make_ctx(10, None, 0),
                make_ctx(11, Some(10), 1),
                make_ctx(12, Some(10), 1),
                make_ctx(20, None, 0),
            ],
            0,
        );
        assert_eq!(router.top_level_order(), vec![0, 3]);
    }

    /// Numbering must not shift when subcontexts appear: ctx 20 stays the
    /// second sidebar slot (Cmd+2) no matter how many children ctx 10 grows.
    #[test]
    fn top_level_order_numbering_is_stable_across_subcontexts() {
        let without = WorkspaceRouter::new(vec![make_ctx(10, None, 0), make_ctx(20, None, 0)], 0);
        let ids_without: Vec<u64> = without
            .top_level_order()
            .iter()
            .map(|&i| without.get(i).context_id)
            .collect();

        let with = WorkspaceRouter::new(
            vec![
                make_ctx(10, None, 0),
                make_ctx(11, Some(10), 1),
                make_ctx(12, Some(10), 1),
                make_ctx(13, Some(11), 2),
                make_ctx(20, None, 0),
            ],
            0,
        );
        let ids_with: Vec<u64> = with
            .top_level_order()
            .iter()
            .map(|&i| with.get(i).context_id)
            .collect();

        assert_eq!(ids_without, vec![10, 20]);
        assert_eq!(
            ids_with, ids_without,
            "subcontexts must not shift top-level sidebar numbering"
        );
    }

    /// Stint 0605: cycling walks the sidebar's enumeration, so it steps master
    /// to master and never activates a hidden subcontext.
    #[test]
    fn cycle_top_level_skips_subcontexts_and_wraps() {
        let contexts = vec![
            make_ctx(10, None, 0),
            make_ctx(11, Some(10), 1),
            make_ctx(12, Some(10), 1),
            make_ctx(20, None, 0),
            make_ctx(30, None, 0),
        ];
        let masters = [0usize, 3, 4];

        for &from in &masters {
            let router = WorkspaceRouter::new(contexts.clone(), from);
            let next = router.cycle_top_level(true).expect("next exists");
            let prev = router.cycle_top_level(false).expect("prev exists");
            for idx in [next, prev] {
                assert!(
                    masters.contains(&idx),
                    "cycling from {from} yielded subcontext idx {idx}"
                );
                assert!(
                    router.get(idx).parent_id.is_none(),
                    "cycling from {from} yielded idx {idx} with a live parent"
                );
            }
        }

        // Last master wraps forward to the first, first wraps back to the last.
        let last = WorkspaceRouter::new(contexts.clone(), 4);
        assert_eq!(last.cycle_top_level(true), Some(0));
        let first = WorkspaceRouter::new(contexts, 0);
        assert_eq!(first.cycle_top_level(false), Some(4));
    }

    /// Parked contexts have no sidebar row either, so cycling steps over them.
    #[test]
    fn cycle_top_level_skips_parked() {
        let mut contexts = vec![
            make_ctx(10, None, 0),
            make_ctx(20, None, 0),
            make_ctx(30, None, 0),
        ];
        contexts[1].parked = true;
        let router = WorkspaceRouter::new(contexts, 0);
        assert_eq!(router.cycle_top_level(true), Some(2));
        assert_eq!(router.cycle_top_level(false), Some(2));
    }

    /// A lone master has nowhere to cycle to; cycling must be a no-op rather
    /// than re-activating the context it is already on.
    #[test]
    fn cycle_top_level_is_none_with_a_single_destination() {
        let router =
            WorkspaceRouter::new(vec![make_ctx(10, None, 0), make_ctx(11, Some(10), 1)], 0);
        assert_eq!(router.cycle_top_level(true), None);
        assert_eq!(router.cycle_top_level(false), None);
    }

    /// Descending through a Portal makes a subcontext active, so it has no slot
    /// in the enumeration. Cycling must still resolve to the nearest master.
    #[test]
    fn cycle_top_level_from_an_active_subcontext_resolves_to_masters() {
        let contexts = vec![
            make_ctx(10, None, 0),
            make_ctx(11, Some(10), 1),
            make_ctx(20, None, 0),
        ];
        let router = WorkspaceRouter::new(contexts, 1);
        assert_eq!(router.cycle_top_level(true), Some(2));
        assert_eq!(router.cycle_top_level(false), Some(0));
    }

    /// A context whose parent was deleted has no portal to be reached through,
    /// so it must fall back to a top-level row rather than become unreachable.
    #[test]
    fn top_level_order_includes_orphans_whose_parent_is_gone() {
        let router =
            WorkspaceRouter::new(vec![make_ctx(10, None, 0), make_ctx(11, Some(999), 1)], 0);
        assert_eq!(router.top_level_order(), vec![0, 1]);
    }

    /// A sidebar reorder only knows top-level rows; expanding must re-inflate
    /// the hidden subtrees so `apply_order` still gets a full permutation and
    /// no subcontext is dropped from the router.
    #[test]
    fn expand_subtrees_reinflates_hidden_children() {
        let router = WorkspaceRouter::new(
            vec![
                make_ctx(10, None, 0),
                make_ctx(11, Some(10), 1),
                make_ctx(20, None, 0),
                make_ctx(21, Some(20), 1),
            ],
            0,
        );
        // Sidebar drag put ctx 20 above ctx 10.
        let expanded = router.expand_subtrees(&[2, 0]);
        assert_eq!(expanded, vec![2, 3, 0, 1]);
        assert_eq!(
            expanded.len(),
            router.len(),
            "expand_subtrees must produce a full permutation"
        );
    }

    /// The end-to-end guard for the drop path: reordering top-level rows must
    /// move each parent's children with it and never delete a subcontext.
    #[test]
    fn reorder_top_level_carries_subtree_and_preserves_all_contexts() {
        let mut router = WorkspaceRouter::new(
            vec![
                make_ctx(10, None, 0),
                make_ctx(11, Some(10), 1),
                make_ctx(20, None, 0),
                make_ctx(21, Some(20), 1),
            ],
            0,
        );
        assert!(router.reorder_top_level(2, ContextMove::Top));
        let ids: Vec<u64> = router.iter().map(|c| c.context_id).collect();
        assert_eq!(ids, vec![20, 21, 10, 11]);
        // Active context (ctx 10) tracked by identity through the reorder.
        assert_eq!(router.active().context_id, 10);
    }

    /// An index-adjacent swap would have traded places with an invisible
    /// subcontext; moving the *last* top-level row down must be a no-op even
    /// though a subcontext sits after it in the router vec.
    #[test]
    fn reorder_top_level_down_at_end_is_noop_despite_trailing_subcontext() {
        let mut router = WorkspaceRouter::new(
            vec![
                make_ctx(10, None, 0),
                make_ctx(20, None, 0),
                make_ctx(21, Some(20), 1),
            ],
            0,
        );
        assert!(!router.reorder_top_level(1, ContextMove::Down));
        let ids: Vec<u64> = router.iter().map(|c| c.context_id).collect();
        assert_eq!(ids, vec![10, 20, 21]);
    }

    /// A subcontext has no sidebar row, so it has nothing to reorder.
    #[test]
    fn reorder_top_level_rejects_a_subcontext() {
        let mut router =
            WorkspaceRouter::new(vec![make_ctx(10, None, 0), make_ctx(11, Some(10), 1)], 0);
        assert!(!router.reorder_top_level(1, ContextMove::Top));
        let ids: Vec<u64> = router.iter().map(|c| c.context_id).collect();
        assert_eq!(ids, vec![10, 11]);
    }

    /// Parked contexts keep their own list; a move within the active list must
    /// not drag a parked context into it.
    #[test]
    fn reorder_top_level_keeps_parked_contexts_in_their_section() {
        let mut router = WorkspaceRouter::new(
            vec![
                make_ctx(10, None, 0),
                make_ctx(20, None, 0),
                make_ctx(30, None, 0),
            ],
            0,
        );
        router.get_mut(1).parked = true;
        assert!(router.reorder_top_level(2, ContextMove::Top));
        let ids: Vec<u64> = router.iter().map(|c| c.context_id).collect();
        assert_eq!(
            ids,
            vec![30, 10, 20],
            "parked ctx 20 stays after the active list"
        );
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
    fn apply_order_permutes_and_tracks_active_by_identity() {
        let mut router = WorkspaceRouter::new(
            vec![
                make_ctx(10, None, 0),
                make_ctx(20, None, 0),
                make_ctx(30, None, 0),
            ],
            1, // active = ctx 20
        );
        // Move ctx 20 (index 1) to the front: new order [20, 10, 30].
        router.apply_order(&[1, 0, 2]);
        let ids: Vec<u64> = router.iter().map(|c| c.context_id).collect();
        assert_eq!(ids, vec![20, 10, 30]);
        // Active still points at ctx 20, now at index 0.
        assert_eq!(router.active_idx(), 0);
        assert_eq!(router.active().context_id, 20);
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
}
