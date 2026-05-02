//! Encapsulates the active-context index so it can only be mutated through
//! controlled methods. Direct field assignment (`active_context = n`) is a
//! compile error outside this module, which makes the class of "forgot to call
//! switch_workspace" bugs structurally impossible.
//!
//! Navigation (with minimap save/restore) → `PlexiApp::switch_workspace`
//! Structural ops (create/delete/reorder) → methods on this struct

use crate::context::Context;

pub(crate) struct WorkspaceRouter {
    active: usize,
    contexts: Vec<Context>,
}

impl WorkspaceRouter {
    pub(crate) fn new(contexts: Vec<Context>, active: usize) -> Self {
        debug_assert!(contexts.is_empty() || active < contexts.len(), "Active index out of bounds");
        Self { active, contexts }
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

    pub(crate) fn position<F: Fn(&Context) -> bool>(&self, f: F) -> Option<usize> {
        self.contexts.iter().position(f)
    }

    // ── Structural mutation (create / delete) ────────────────────────────────

    pub(crate) fn push(&mut self, ctx: Context) {
        self.contexts.push(ctx);
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
        debug_assert!(idx < self.contexts.len(), "Target active index out of bounds");
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

    /// Remove at `src`, insert at `dst` (after removal), tracking active.
    pub(crate) fn reorder_tracking_active(&mut self, src: usize, dst: usize) {
        let ctx = self.contexts.remove(src);
        self.contexts.insert(dst, ctx);
        if self.active == src {
            self.active = dst;
        } else if src < self.active && dst >= self.active {
            self.active -= 1;
        } else if src > self.active && dst <= self.active {
            self.active += 1;
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
}
