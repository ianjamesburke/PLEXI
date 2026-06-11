use std::collections::HashMap;

use crate::host::command::{Direction, HostAction, OpenPaneRequest, PaneRuntimeKind, Placement};
use crate::host::effect::{HostEffect, HostEvent};
use crate::host::services::HostServices;
use crate::spatial::tiling::PaneId;

#[derive(Debug, Clone)]
pub struct HostPane {
    pub id: PaneId,
    pub group: Option<String>,
}

#[derive(Debug, Clone)]
pub struct HostContext {
    pub panes: Vec<HostPane>,
    pub focused_pane: Option<PaneId>,
    /// group name → member pane IDs
    pub groups: HashMap<String, Vec<PaneId>>,
    /// Stable unique ID matching `Context::context_id`.
    pub context_id: u64,
    /// Parent context_id for sub-contexts. None = top-level.
    pub parent_id: Option<u64>,
}

impl Default for HostContext {
    fn default() -> Self {
        Self {
            panes: Vec::new(),
            focused_pane: None,
            groups: HashMap::new(),
            context_id: 0,
            parent_id: None,
        }
    }
}

#[derive(Debug, Default)]
pub struct HostModel {
    contexts: Vec<HostContext>,
    active_context: usize,
    next_pane_id: PaneId,
}

impl HostModel {
    pub fn new() -> Self {
        let mut model = Self {
            contexts: vec![HostContext::default()],
            active_context: 0,
            next_pane_id: 1,
        };
        let initial = HostPane { id: 0, group: None };
        model.context_mut().panes.push(initial);
        model.context_mut().focused_pane = Some(0);
        model
    }

    pub fn handle_command(
        &mut self,
        command: HostAction,
        services: &mut HostServices,
    ) -> Vec<HostEffect> {
        let effects = match command {
            HostAction::OpenPane(req) => self.open_pane(req),
            HostAction::CloseFocusedPane => self.close_focused_pane(),
            HostAction::Navigate(dir) => self.navigate(dir),
            HostAction::SplitHorizontal => self.split(Placement::Below),
            HostAction::SplitVertical => self.split(Placement::Right),
        };
        for e in &effects {
            services.event_sink.emit(e);
        }
        effects
    }

    // ── commands ────────────────────────────────────────────────────────────

    fn open_pane(&mut self, req: OpenPaneRequest) -> Vec<HostEffect> {
        let new_id = self.alloc_pane_id();
        let pane = HostPane {
            id: new_id,
            group: req.group.clone(),
        };
        if let Some(ref group) = req.group {
            self.context_mut()
                .groups
                .entry(group.clone())
                .or_default()
                .push(new_id);
        }
        self.context_mut().panes.push(pane);
        self.context_mut().focused_pane = Some(new_id);
        vec![
            HostEffect::PaneOpened {
                pane_id: new_id,
                kind: req.runtime,
                share: req.share,
                placement: req.placement,
            },
            HostEffect::FocusChanged {
                pane_id: Some(new_id),
            },
            HostEffect::EventEmitted(HostEvent::PaneOpened { pane_id: new_id }),
        ]
    }

    fn close_focused_pane(&mut self) -> Vec<HostEffect> {
        let Some(focused) = self.context().focused_pane else {
            return Vec::new();
        };
        let idx = self.context().panes.iter().position(|p| p.id == focused);
        let Some(idx) = idx else { return Vec::new() };

        // Remove from group membership before removing the pane.
        let group = self.context().panes[idx].group.clone();
        self.context_mut().panes.remove(idx);
        if let Some(ref g) = group {
            if let Some(members) = self.context_mut().groups.get_mut(g) {
                members.retain(|&id| id != focused);
            }
        }

        let new_focus = self
            .context()
            .panes
            .get(idx.saturating_sub(1))
            .map(|p| p.id);
        self.context_mut().focused_pane = new_focus;

        vec![
            HostEffect::PaneClosed { pane_id: focused },
            HostEffect::FocusChanged { pane_id: new_focus },
            HostEffect::EventEmitted(HostEvent::PaneClosed { pane_id: focused }),
        ]
    }

    fn navigate(&mut self, dir: Direction) -> Vec<HostEffect> {
        let panes = &self.context().panes;
        if panes.is_empty() {
            return Vec::new();
        }
        let cur = self.context().focused_pane.unwrap_or(panes[0].id);
        let idx = panes.iter().position(|p| p.id == cur).unwrap_or(0);
        let next_idx = match dir {
            Direction::Left | Direction::Up => (idx + panes.len() - 1) % panes.len(),
            Direction::Right | Direction::Down => (idx + 1) % panes.len(),
        };
        let next_id = panes[next_idx].id;
        self.context_mut().focused_pane = Some(next_id);
        vec![HostEffect::FocusChanged {
            pane_id: Some(next_id),
        }]
    }

    fn split(&mut self, placement: Placement) -> Vec<HostEffect> {
        let new_id = self.alloc_pane_id();
        let pane = HostPane {
            id: new_id,
            group: None,
        };
        self.context_mut().panes.push(pane);
        self.context_mut().focused_pane = Some(new_id);
        vec![
            HostEffect::SplitOpened {
                pane_id: new_id,
                kind: PaneRuntimeKind::Terminal,
                placement,
            },
            HostEffect::FocusChanged {
                pane_id: Some(new_id),
            },
        ]
    }

    // ── helpers ─────────────────────────────────────────────────────────────

    /// Allocate a new pane ID. `HostModel` is the sole ID allocator — every
    /// pane ID in `PlexiApp.windows.panes` must be obtained here, either
    /// directly or via `handle_command` (which calls this internally).
    ///
    /// The full pane *registry* (the `Pane` structs) lives in
    /// `PlexiApp.windows.panes`, not in `HostModel`. `HostModel.context().panes`
    /// tracks only panes opened through `handle_command`; callers that use
    /// `alloc_pane_id()` directly (e.g. `create_single_pane_tree`,
    /// `spawn_terminal_pane_at`) skip that registration intentionally.
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

    /// IDs of all panes registered in the active context via `handle_command`.
    /// Does not include panes created with `alloc_pane_id()` directly.
    #[cfg(test)]
    pub fn test_pane_ids(&self) -> Vec<PaneId> {
        self.context().panes.iter().map(|p| p.id).collect()
    }

    /// Focused pane as tracked by `HostModel` (updated by `handle_command`).
    #[cfg(test)]
    pub fn test_focused_pane(&self) -> Option<PaneId> {
        self.context().focused_pane
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
            ..Default::default()
        });
    }

    fn context(&self) -> &HostContext {
        &self.contexts[self.active_context]
    }

    fn context_mut(&mut self) -> &mut HostContext {
        &mut self.contexts[self.active_context]
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
}

// Shorthand to avoid repeating the long path in new_context().

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::services::{EventSink, HostServices};

    /// Test sink that drops every effect — no audit trail required for unit tests.
    struct NullSink;
    impl EventSink for NullSink {
        fn emit(&mut self, _: &HostEffect) {}
    }

    fn services() -> HostServices {
        HostServices {
            event_sink: Box::new(NullSink),
        }
    }

    /// `SplitHorizontal` emits `Placement::Below` (stacked). In `split_focused`, vertical=false
    /// maps to SplitHorizontal, and the recalculation `!matches(Below, Below)=false` preserves
    /// vertical=false → Horizontal tile → side-by-side. The Placement here is internal state.
    #[test]
    fn split_horizontal_emits_below_placement() {
        let mut model = HostModel::new();
        let mut svc = services();
        let effects = model.handle_command(HostAction::SplitHorizontal, &mut svc);
        let placement = effects
            .iter()
            .find_map(|e| match e {
                HostEffect::SplitOpened { placement, .. } => Some(*placement),
                _ => None,
            })
            .expect("SplitOpened effect must be emitted");
        assert_eq!(placement, Placement::Below);
    }

    /// `SplitVertical` emits `Placement::Right` (side-by-side). In `split_focused`, vertical=true
    /// maps to SplitVertical, and the recalculation `!matches(Right, Below)=true` preserves
    /// vertical=true → Vertical tile → stacked. The Placement here is internal state.
    #[test]
    fn split_vertical_emits_right_placement() {
        let mut model = HostModel::new();
        let mut svc = services();
        let effects = model.handle_command(HostAction::SplitVertical, &mut svc);
        let placement = effects
            .iter()
            .find_map(|e| match e {
                HostEffect::SplitOpened { placement, .. } => Some(*placement),
                _ => None,
            })
            .expect("SplitOpened effect must be emitted");
        assert_eq!(placement, Placement::Right);
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

    // ── HostModel pane invariant tests ────────────────────────────────────────

    #[test]
    fn new_model_has_one_bootstrap_pane_registered() {
        let model = HostModel::new();
        let ids = model.test_pane_ids();
        assert_eq!(
            ids,
            vec![0],
            "HostModel::new() registers bootstrap pane id=0"
        );
        assert_eq!(model.test_focused_pane(), Some(0));
        assert_eq!(model.test_next_pane_id(), 1);
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
    fn open_pane_registers_id_in_model_and_sets_focus() {
        let mut model = HostModel::new();
        let mut svc = services();
        let req = crate::host::command::OpenPaneRequest {
            runtime: PaneRuntimeKind::Terminal,
            placement: Placement::Right,
            share: crate::host::command::ShareRatio::new(1.0, 1.0).unwrap(),
            group: None,
            declared_capabilities: vec![],
        };
        let pre_next = model.test_next_pane_id();
        let effects = model.handle_command(HostAction::OpenPane(req), &mut svc);

        let opened_id = effects
            .iter()
            .find_map(|e| match e {
                HostEffect::PaneOpened { pane_id, .. } => Some(*pane_id),
                _ => None,
            })
            .expect("OpenPane must emit PaneOpened");

        assert_eq!(
            opened_id, pre_next,
            "PaneOpened id must equal pre-call next_pane_id"
        );
        assert!(
            model.test_pane_ids().contains(&opened_id),
            "opened pane must be registered in model"
        );
        assert_eq!(
            model.test_focused_pane(),
            Some(opened_id),
            "model focus must update to the newly opened pane"
        );
        assert_eq!(model.test_next_pane_id(), pre_next + 1);
    }

    #[test]
    fn close_focused_pane_removes_from_model_registry() {
        let mut model = HostModel::new();
        let mut svc = services();
        let req = crate::host::command::OpenPaneRequest {
            runtime: PaneRuntimeKind::Terminal,
            placement: Placement::Right,
            share: crate::host::command::ShareRatio::new(1.0, 1.0).unwrap(),
            group: None,
            declared_capabilities: vec![],
        };
        let effects = model.handle_command(HostAction::OpenPane(req), &mut svc);
        let opened_id = effects
            .iter()
            .find_map(|e| match e {
                HostEffect::PaneOpened { pane_id, .. } => Some(*pane_id),
                _ => None,
            })
            .unwrap();

        let close_effects = model.handle_command(HostAction::CloseFocusedPane, &mut svc);
        let closed_id = close_effects
            .iter()
            .find_map(|e| match e {
                HostEffect::PaneClosed { pane_id } => Some(*pane_id),
                _ => None,
            })
            .expect("CloseFocusedPane must emit PaneClosed");

        assert_eq!(closed_id, opened_id);
        assert!(
            !model.test_pane_ids().contains(&opened_id),
            "closed pane must be removed from model registry"
        );
    }

    #[test]
    fn split_allocates_next_id_and_registers_in_model() {
        let mut model = HostModel::new();
        let mut svc = services();
        let pre_next = model.test_next_pane_id();
        let effects = model.handle_command(HostAction::SplitVertical, &mut svc);

        let split_id = effects
            .iter()
            .find_map(|e| match e {
                HostEffect::SplitOpened { pane_id, .. } => Some(*pane_id),
                _ => None,
            })
            .expect("SplitVertical must emit SplitOpened");

        assert_eq!(
            split_id, pre_next,
            "split pane id must equal pre-call next_pane_id"
        );
        assert!(
            model.test_pane_ids().contains(&split_id),
            "split pane must be registered in model"
        );
        assert_eq!(
            model.test_focused_pane(),
            Some(split_id),
            "model focus must move to split pane"
        );
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

    #[test]
    fn direct_alloc_pane_id_does_not_register_in_model_panes() {
        // Documents the intentional design split: alloc_pane_id() advances the
        // counter (so IDs are unique) but does NOT add an entry to context().panes.
        // Callers like create_single_pane_tree use alloc_pane_id() directly and
        // manage pane registration themselves in PlexiApp.windows.panes.
        let mut model = HostModel::new();
        let pre_count = model.test_pane_ids().len();
        let _id = model.alloc_pane_id();
        assert_eq!(
            model.test_pane_ids().len(),
            pre_count,
            "alloc_pane_id() must not add to model panes list"
        );
    }
}
