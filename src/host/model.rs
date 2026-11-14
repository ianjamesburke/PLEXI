use std::collections::HashMap;

use crate::spatial::tiling::PaneId;

#[derive(Debug, Clone, Default)]
pub struct HostContext {
    /// Stable unique ID matching `Context::context_id`.
    pub context_id: u64,
    /// Parent context_id for sub-contexts. None = top-level.
    pub parent_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PaneSelection {
    pub anchor: PaneId,
    pub panes: Vec<PaneId>,
}

#[derive(Debug, Default)]
pub struct HostModel {
    contexts: Vec<HostContext>,
    pane_selections: HashMap<u64, PaneSelection>,
    next_pane_id: PaneId,
    pub(crate) pane_lifecycle: HashMap<PaneId, crate::host::pane_lifecycle::PaneLifecycleState>,
    /// Authoritative continuous-strip state per context (`context_id -> Strip`).
    pub(crate) strips: HashMap<u64, crate::strip::Strip>,
    /// Latest resolved strip presentation snapshot per context (`context_id -> StripLayout`).
    pub(crate) strip_layouts: HashMap<u64, PublishedStripSnapshot>,
}

#[derive(Debug)]
pub(crate) struct PublishedStripSnapshot {
    pub generation: u64,
    pub strip: crate::strip::Strip,
    pub layout: crate::strip::StripLayout,
}

impl HostModel {
    pub(crate) fn pane_selection(&self, window: u64) -> Option<&PaneSelection> {
        self.pane_selections.get(&window)
    }

    pub(crate) fn extend_pane_selection(&mut self, window: u64, start: PaneId, next: PaneId) {
        let selection = self
            .pane_selections
            .entry(window)
            .or_insert_with(|| PaneSelection {
                anchor: start,
                panes: vec![start],
            });
        if !selection.panes.contains(&next) {
            selection.panes.push(next);
        }
        log::info!(
            "pane_selection: window={window} anchor={} panes={:?}",
            selection.anchor,
            selection.panes
        );
    }

    pub(crate) fn clear_pane_selection(&mut self, window: u64) -> Option<PaneSelection> {
        let selection = self.pane_selections.remove(&window);
        if selection.is_some() {
            log::info!("pane_selection: cleared window={window}");
        }
        selection
    }

    pub fn new() -> Self {
        Self {
            contexts: vec![HostContext::default()],
            pane_selections: HashMap::new(),
            // Pane id 0 is the bootstrap pane.
            next_pane_id: 1,
            pane_lifecycle: HashMap::new(),
            strips: HashMap::new(),
            strip_layouts: HashMap::new(),
        }
    }

    /// Allocate a new pane ID. `HostModel` is the sole ID allocator: every
    /// pane ID in `PlexiApp.windows.panes` must be obtained here. The pane
    /// registry itself (the `Pane` structs) lives in `PlexiApp.windows.panes`.
    pub fn alloc_pane_id(&mut self) -> PaneId {
        let id = self.next_pane_id;
        self.next_pane_id += 1;
        id
    }

    /// Next pane ID that `alloc_pane_id` will return. Used by workspace restore
    /// to persist and resume the allocation counter without double-allocating.
    pub fn next_pane_id(&self) -> PaneId {
        self.next_pane_id
    }

    /// Seed the counter from a persisted workspace. Only safe to call before
    /// any panes have been opened, because lowering the counter would collide
    /// with existing IDs. A warning is logged if the caller asks for a value
    /// below the current high-water mark.
    pub fn seed_next_pane_id(&mut self, id: PaneId) {
        if id < self.next_pane_id {
            log::warn!(
                "HostModel::seed_next_pane_id({id}) below current {}; clamping",
                self.next_pane_id
            );
            return;
        }
        self.next_pane_id = id;
    }

    /// Next ID that `alloc_pane_id()` will return (peek, does not advance counter).
    #[cfg(test)]
    pub fn test_next_pane_id(&self) -> PaneId {
        self.next_pane_id
    }

    #[cfg(test)]
    pub fn add_context(&mut self, context_id: u64, parent_id: Option<u64>) {
        self.contexts.push(HostContext {
            context_id,
            parent_id,
        });
    }

    #[cfg(test)]
    pub fn children_of(&self, context_id: u64) -> Vec<u64> {
        self.contexts
            .iter()
            .filter(|c| c.parent_id == Some(context_id))
            .map(|c| c.context_id)
            .collect()
    }

    /// Return the ancestor chain for `context_id`, from immediate parent to root.
    /// Returns an empty vec if `context_id` is top-level or not found.
    pub fn ancestors_of(&self, context_id: u64) -> Vec<u64> {
        let mut result = Vec::new();
        let mut current = context_id;
        // Guard against cycles with a max depth.
        for _ in 0..16 {
            let parent = self
                .contexts
                .iter()
                .find(|c| c.context_id == current)
                .and_then(|c| c.parent_id);
            match parent {
                Some(pid) => {
                    result.push(pid);
                    current = pid;
                }
                None => break,
            }
        }
        result
    }

    /// Return the authoritative strip for `context_id` if present.
    pub fn strip(&self, context_id: u64) -> Option<&crate::strip::Strip> {
        self.strips.get(&context_id)
    }

    /// Return a mutable reference to the authoritative strip for `context_id` if present.
    pub fn strip_mut(&mut self, context_id: u64) -> Option<&mut crate::strip::Strip> {
        self.strips.get_mut(&context_id)
    }

    /// Insert or replace the authoritative strip for `context_id`.
    pub fn insert_strip(&mut self, context_id: u64, strip: crate::strip::Strip) {
        self.strips.insert(context_id, strip);
    }

    /// Return the latest presentation snapshot for `context_id`.
    pub fn strip_layout(&self, context_id: u64) -> Option<&crate::strip::StripLayout> {
        self.strip_layouts
            .get(&context_id)
            .map(|snapshot| &snapshot.layout)
    }

    pub(crate) fn published_strip_snapshot(
        &self,
        context_id: u64,
    ) -> Option<&PublishedStripSnapshot> {
        self.strip_layouts.get(&context_id)
    }

    /// Store the latest presentation snapshot for `context_id`.
    pub fn set_strip_layout(&mut self, context_id: u64, layout: crate::strip::StripLayout) {
        let Some(strip) = self.strips.get(&context_id) else {
            return;
        };
        let generation = self.strip_layouts.get(&context_id).map_or(1, |previous| {
            if previous.layout == layout && previous.strip == *strip {
                previous.generation
            } else {
                previous.generation.saturating_add(1)
            }
        });
        self.strip_layouts.insert(
            context_id,
            PublishedStripSnapshot {
                generation,
                strip: strip.clone(),
                layout,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stint 1318: the one checkable clause of the HostModel rule. `HostModel`
    /// has no UI toolkit dependency; the needle is split so this test does not match
    /// itself.
    #[test]
    fn host_model_source_has_no_ui_toolkit_reference() {
        let needle = ["eg", "ui"].concat();
        let source = include_str!("model.rs");
        let hits: Vec<&str> = source
            .lines()
            .filter(|line| line.contains(needle.as_str()))
            .collect();
        assert!(
            hits.is_empty(),
            "src/host/model.rs must not reference {needle}; move UI work to PlexiApp: {hits:?}"
        );
    }

    #[test]
    fn add_context_stores_hierarchy_fields() {
        let mut model = HostModel::new();
        model.add_context(10, None);
        model.add_context(20, Some(10));

        assert_eq!(model.contexts.len(), 3); // default + 2 added
        assert_eq!(model.contexts[1].context_id, 10);
        assert_eq!(model.contexts[1].parent_id, None);
        assert_eq!(model.contexts[2].context_id, 20);
        assert_eq!(model.contexts[2].parent_id, Some(10));
    }

    #[test]
    fn children_of_returns_direct_children() {
        let mut model = HostModel::new();
        model.add_context(1, None);
        model.add_context(2, Some(1));
        model.add_context(3, Some(1));
        model.add_context(4, Some(2));

        let children = model.children_of(1);
        assert_eq!(children.len(), 2);
        assert!(children.contains(&2));
        assert!(children.contains(&3));

        let grandchildren = model.children_of(2);
        assert_eq!(grandchildren, vec![4]);

        assert!(model.children_of(99).is_empty());
    }

    #[test]
    fn ancestors_of_returns_parent_chain() {
        let mut model = HostModel::new();
        model.add_context(1, None);
        model.add_context(2, Some(1));
        model.add_context(3, Some(2));

        let ancestors = model.ancestors_of(3);
        assert_eq!(ancestors, vec![2, 1]);

        let ancestors_of_root = model.ancestors_of(1);
        assert!(ancestors_of_root.is_empty());

        assert!(model.ancestors_of(99).is_empty());
    }

    // ── HostModel pane id invariants ──────────────────────────────────────────

    #[test]
    fn new_model_reserves_bootstrap_pane_id() {
        let model = HostModel::new();
        assert_eq!(
            model.test_next_pane_id(),
            1,
            "HostModel::new() reserves pane id 0 for the bootstrap pane"
        );
    }

    #[test]
    fn alloc_pane_id_is_monotone_and_advances_counter() {
        let mut model = HostModel::new();
        let a = model.alloc_pane_id();
        let b = model.alloc_pane_id();
        let c = model.alloc_pane_id();
        assert!(
            a < b && b < c,
            "alloc_pane_id must return strictly increasing IDs"
        );
        assert_eq!(model.test_next_pane_id(), c + 1);
    }

    #[test]
    fn seed_next_pane_id_positions_allocations_above_restored_ids() {
        let mut model = HostModel::new();
        let max_restored = 99u64;
        model.seed_next_pane_id(max_restored + 1);
        assert_eq!(model.test_next_pane_id(), max_restored + 1);
        for _ in 0..5 {
            let id = model.alloc_pane_id();
            assert!(
                id > max_restored,
                "post-seed allocation id={id} must exceed max restored id={max_restored}"
            );
        }
    }
}
