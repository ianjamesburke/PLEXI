use std::collections::HashMap;

use crate::host::command::{
    Direction, HostAction, OpenPaneRequest, Placement, PaneRuntimeKind,
};
use crate::host::effect::{HostEffect, HostEvent};
use crate::host::services::HostServices;
use crate::tiling::PaneId;

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
        let initial = HostPane {
            id: 0,
            group: None,
        };
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
            HostEffect::FocusChanged { pane_id: Some(new_id) },
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
        vec![HostEffect::FocusChanged { pane_id: Some(next_id) }]
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
            HostEffect::FocusChanged { pane_id: Some(new_id) },
        ]
    }

    // ── helpers ─────────────────────────────────────────────────────────────

    /// Allocate a new pane ID. `HostModel` is the single source of truth —
    /// `PlexiApp` and `pane_ops` call this directly for any pane creation that
    /// does not already flow through `handle_command` (e.g. the egui-tiles
    /// bookkeeping in `new_tab` / `create_single_pane_tree`).
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

    /// Add a new context with the given ID and optional parent.
    /// Used by the test harness to set up sub-context hierarchies.
    pub fn add_context(&mut self, context_id: u64, parent_id: Option<u64>) {
        log::info!(
            "HostModel::add_context(context_id={context_id}, parent_id={parent_id:?})"
        );
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

    /// Return context IDs whose `parent_id` matches the given `context_id`.
    pub fn children_of(&self, context_id: u64) -> Vec<u64> {
        log::info!("HostModel::children_of(context_id={context_id})");
        self.contexts
            .iter()
            .filter(|c| c.parent_id == Some(context_id))
            .map(|c| c.context_id)
            .collect()
    }

    /// Return the ancestor chain for `context_id`, from immediate parent to root.
    /// Returns an empty vec if `context_id` is top-level or not found.
    pub fn ancestors_of(&self, context_id: u64) -> Vec<u64> {
        log::info!("HostModel::ancestors_of(context_id={context_id})");
        let mut result = Vec::new();
        let mut current = context_id;
        // Guard against cycles with a max depth.
        for _ in 0..16 {
            let parent = self.contexts.iter().find(|c| c.context_id == current)
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
}
