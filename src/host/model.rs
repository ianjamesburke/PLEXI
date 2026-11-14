use std::collections::{HashMap, VecDeque};

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
    /// The pane the last selection step focused. Focus anywhere else means a
    /// non-selection focus move (split, new pane, palette or CLI jump,
    /// history), which ends the selection.
    pub cursor: PaneId,
}

/// Where a [`PaneTargets`] set came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PaneTargetSource {
    Selection,
    Focused,
}

impl PaneTargetSource {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Selection => "selection",
            Self::Focused => "focused",
        }
    }
}

/// The panes a pane action applies to: a stable snapshot of pane ids, in
/// selection order, that survives closes made while the action runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PaneTargets {
    pub anchor: PaneId,
    pub panes: Vec<PaneId>,
    pub source: PaneTargetSource,
}

/// What a paused close batch waits for: the normal dialog of one pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloseBatchWait {
    /// A view vetoed the close and shows its unsaved-changes dialog.
    Unsaved(PaneId),
    /// `confirm_close` is on; `confirmed` is set by the dialog's Close button.
    ConfirmClose { pane: PaneId, confirmed: bool },
    /// A portal whose subcontext is not empty shows the context-close dialog.
    ContextClose(PaneId),
}

impl CloseBatchWait {
    pub(crate) fn pane(self) -> PaneId {
        match self {
            Self::Unsaved(pane) | Self::ContextClose(pane) => pane,
            Self::ConfirmClose { pane, .. } => pane,
        }
    }
}

/// Close several panes one after another, each through its own close veto
/// and dialog. Owns its snapshot, so closes that clear the window's selection
/// do not end it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PaneCloseBatch {
    pub window: u64,
    pub anchor: PaneId,
    pub queue: VecDeque<PaneId>,
    pub waiting: Option<CloseBatchWait>,
    /// Pane chosen once, before any close, by the spatial close-focus rule.
    pub focus_after: Option<PaneId>,
}

impl PaneCloseBatch {
    /// The panes not closed yet, the paused one first.
    pub(crate) fn remaining(&self) -> Vec<PaneId> {
        self.waiting
            .map(CloseBatchWait::pane)
            .into_iter()
            .chain(self.queue.iter().copied())
            .collect()
    }
}

#[derive(Debug, Default)]
pub struct HostModel {
    contexts: Vec<HostContext>,
    pane_selections: HashMap<u64, PaneSelection>,
    pane_close_batch: Option<PaneCloseBatch>,
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
                cursor: start,
            });
        if !selection.panes.contains(&next) {
            selection.panes.push(next);
        }
        selection.cursor = next;
        log::info!(
            "pane_selection: window={window} anchor={} panes={:?}",
            selection.anchor,
            selection.panes
        );
    }

    /// Cmd+Shift+click: add `pane`, or take it out when it is already
    /// selected. A selection left empty ends.
    pub(crate) fn toggle_pane_selection(&mut self, window: u64, start: PaneId, pane: PaneId) {
        let selected = self
            .pane_selections
            .get(&window)
            .is_some_and(|s| s.panes.contains(&pane));
        if !selected {
            self.extend_pane_selection(window, start, pane);
            return;
        }
        let Some(selection) = self.pane_selections.get_mut(&window) else {
            return;
        };
        selection.panes.retain(|id| *id != pane);
        selection.cursor = pane;
        log::info!(
            "pane_selection: window={window} removed={pane} panes={:?}",
            selection.panes
        );
        if selection.panes.is_empty() {
            self.clear_pane_selection(window);
        } else if selection.anchor == pane {
            selection.anchor = selection.panes[0];
        }
    }

    /// A focus move that keeps the selection (a file drag hovering a pane)
    /// moves its cursor along.
    pub(crate) fn set_pane_selection_cursor(&mut self, window: u64, pane: PaneId) {
        if let Some(selection) = self.pane_selections.get_mut(&window) {
            selection.cursor = pane;
        }
    }

    /// Windows that currently hold a selection.
    pub(crate) fn pane_selection_windows(&self) -> Vec<u64> {
        self.pane_selections.keys().copied().collect()
    }

    pub(crate) fn clear_pane_selection(&mut self, window: u64) -> Option<PaneSelection> {
        let selection = self.pane_selections.remove(&window);
        if selection.is_some() {
            log::info!("pane_selection: cleared window={window}");
        }
        selection
    }

    /// Replace a window's selection, e.g. with the panes a cancelled close
    /// batch left open. An empty set clears it.
    pub(crate) fn set_pane_selection(
        &mut self,
        window: u64,
        anchor: PaneId,
        panes: Vec<PaneId>,
        cursor: PaneId,
    ) {
        if panes.is_empty() {
            self.clear_pane_selection(window);
            return;
        }
        let anchor = if panes.contains(&anchor) {
            anchor
        } else {
            panes[0]
        };
        log::info!("pane_selection: set window={window} anchor={anchor} panes={panes:?}");
        self.pane_selections.insert(
            window,
            PaneSelection {
                anchor,
                panes,
                cursor,
            },
        );
    }

    /// Which panes `action` applies to in `window`: the selection when it
    /// still holds a live pane, else the focused pane. Ids `is_live` rejects
    /// (closed, moved to another window) are dropped and logged. Minimum set
    /// sizes are the caller's rule, not the resolver's.
    pub(crate) fn resolve_pane_targets(
        &self,
        action: &str,
        window: u64,
        focused: Option<PaneId>,
        is_live: impl Fn(PaneId) -> bool,
    ) -> Option<PaneTargets> {
        let from_selection = self.pane_selections.get(&window).and_then(|selection| {
            let (panes, stale): (Vec<PaneId>, Vec<PaneId>) =
                selection.panes.iter().copied().partition(|id| is_live(*id));
            if !stale.is_empty() {
                log::info!(
                    "pane_targets: action={action} window={window} dropped stale ids={stale:?}"
                );
            }
            let anchor = if panes.contains(&selection.anchor) {
                selection.anchor
            } else {
                *panes.first()?
            };
            Some(PaneTargets {
                anchor,
                panes,
                source: PaneTargetSource::Selection,
            })
        });
        let targets = from_selection.or_else(|| {
            focused.filter(|id| is_live(*id)).map(|id| PaneTargets {
                anchor: id,
                panes: vec![id],
                source: PaneTargetSource::Focused,
            })
        });
        match &targets {
            Some(t) => log::info!(
                "pane_targets: action={action} window={window} source={} anchor={} panes={:?}",
                t.source.as_str(),
                t.anchor,
                t.panes
            ),
            None => log::info!("pane_targets: action={action} window={window} none"),
        }
        targets
    }

    pub(crate) fn pane_close_batch(&self) -> Option<&PaneCloseBatch> {
        self.pane_close_batch.as_ref()
    }

    pub(crate) fn pane_close_batch_mut(&mut self) -> Option<&mut PaneCloseBatch> {
        self.pane_close_batch.as_mut()
    }

    pub(crate) fn start_pane_close_batch(&mut self, batch: PaneCloseBatch) {
        self.pane_close_batch = Some(batch);
    }

    pub(crate) fn take_pane_close_batch(&mut self) -> Option<PaneCloseBatch> {
        self.pane_close_batch.take()
    }

    pub fn new() -> Self {
        Self {
            contexts: vec![HostContext::default()],
            pane_selections: HashMap::new(),
            pane_close_batch: None,
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

    /// Stint 1458: the selection wins over focus, keeps selection order and
    /// its anchor, and drops ids the caller says are no longer live.
    #[test]
    fn pane_targets_prefer_selection_and_drop_stale_ids() {
        let mut model = HostModel::new();
        model.extend_pane_selection(7, 1, 2);
        model.extend_pane_selection(7, 2, 3);
        let live = |id: PaneId| id != 2;
        let targets = model
            .resolve_pane_targets("test", 7, Some(9), live)
            .unwrap();
        assert_eq!(targets.source, PaneTargetSource::Selection);
        assert_eq!(targets.anchor, 1);
        assert_eq!(
            targets.panes,
            vec![1, 3],
            "stale id 2 is dropped, order kept"
        );

        let anchor_gone = model
            .resolve_pane_targets("test", 7, None, |id| id == 3)
            .unwrap();
        assert_eq!(
            anchor_gone.anchor, 3,
            "a stale anchor yields to the first live pane"
        );
    }

    #[test]
    fn pane_targets_fall_back_to_the_focused_pane() {
        let mut model = HostModel::new();
        let focused = model
            .resolve_pane_targets("test", 7, Some(4), |_| true)
            .unwrap();
        assert_eq!(focused.source, PaneTargetSource::Focused);
        assert_eq!(focused.panes, vec![4]);
        assert!(model
            .resolve_pane_targets("test", 7, None, |_| true)
            .is_none());

        model.extend_pane_selection(7, 1, 2);
        let all_stale = model
            .resolve_pane_targets("test", 7, Some(4), |id| id == 4)
            .unwrap();
        assert_eq!(
            all_stale.source,
            PaneTargetSource::Focused,
            "a selection with no live pane falls through to focus"
        );
    }

    /// A selection of one is still the selection; whether one pane is enough
    /// is the action's rule (close acts on it like a focused close, wrap
    /// needs two).
    #[test]
    fn pane_targets_return_a_selection_of_one_unchanged() {
        let mut model = HostModel::new();
        model.extend_pane_selection(7, 5, 5);
        let one = model
            .resolve_pane_targets("test", 7, Some(5), |_| true)
            .unwrap();
        assert_eq!(one.source, PaneTargetSource::Selection);
        assert_eq!(one.panes, vec![5]);
    }

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
