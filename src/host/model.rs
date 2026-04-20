use std::collections::HashMap;

use crate::host::command::{
    Direction, HostCommand, OpenPaneRequest, Placement, PaneRuntimeKind,
};
use crate::host::effect::{HostEffect, HostEvent};
use crate::host::services::HostServices;
use crate::tiling::PaneId;

#[derive(Debug, Clone)]
pub struct HostPane {
    pub id: PaneId,
    pub group: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct HostContext {
    pub panes: Vec<HostPane>,
    pub focused_pane: Option<PaneId>,
    /// group name → member pane IDs
    pub groups: HashMap<String, Vec<PaneId>>,
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
        command: HostCommand,
        services: &mut HostServices,
    ) -> Vec<HostEffect> {
        let effects = match command {
            HostCommand::OpenPane(req) => self.open_pane(req),
            HostCommand::CloseFocusedPane => self.close_focused_pane(),
            HostCommand::Navigate(dir) => self.navigate(dir),
            HostCommand::SplitHorizontal => self.split(Placement::Below),
            HostCommand::SplitVertical => self.split(Placement::Right),
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

    fn context(&self) -> &HostContext {
        &self.contexts[self.active_context]
    }

    fn context_mut(&mut self) -> &mut HostContext {
        &mut self.contexts[self.active_context]
    }
}

// Shorthand to avoid repeating the long path in new_context().
