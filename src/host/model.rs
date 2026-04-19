use std::collections::{HashMap, HashSet};

use crate::host::command::{
    Capability, Direction, HostCommand, OpenPaneRequest, Placement, PaneRuntimeKind,
};
use crate::host::effect::{HostEffect, HostEvent};
use crate::host::services::HostServices;
use crate::tiling::PaneId;

#[derive(Debug, Clone)]
pub struct HostPane {
    pub id: PaneId,
    pub kind: PaneRuntimeKind,
    pub declared_capabilities: Vec<Capability>,
    pub group: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct HostContext {
    pub panes: Vec<HostPane>,
    pub focused_pane: Option<PaneId>,
    /// group name → member pane IDs
    pub groups: HashMap<String, Vec<PaneId>>,
    /// pane ID → set of granted capabilities
    pub permissions: HashMap<PaneId, HashSet<Capability>>,
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
            kind: PaneRuntimeKind::Terminal,
            declared_capabilities: Vec::new(),
            group: None,
        };
        model.context_mut().panes.push(initial);
        model.context_mut().focused_pane = Some(0);
        model
    }

    pub fn active_context_index(&self) -> usize {
        self.active_context
    }

    pub fn context_count(&self) -> usize {
        self.contexts.len()
    }

    pub fn pane_count(&self) -> usize {
        self.context().panes.len()
    }

    pub fn focused_pane(&self) -> Option<&HostPane> {
        let focused = self.context().focused_pane?;
        self.context().panes.iter().find(|p| p.id == focused)
    }

    pub fn pane_by_id(&self, id: PaneId) -> Option<&HostPane> {
        self.context().panes.iter().find(|p| p.id == id)
    }

    pub fn handle_command(
        &mut self,
        command: HostCommand,
        services: &mut HostServices,
    ) -> Vec<HostEffect> {
        let effects = match command {
            HostCommand::OpenPane(req) => self.open_pane(req),
            HostCommand::CloseFocusedPane => self.close_focused_pane(),
            HostCommand::FocusPane(id) => self.focus_pane(id),
            HostCommand::Navigate(dir) => self.navigate(dir),
            HostCommand::SplitHorizontal => self.split(Placement::Below),
            HostCommand::SplitVertical => self.split(Placement::Right),
            HostCommand::NewContext => self.new_context(),
            HostCommand::SwitchContext(idx) => self.switch_context(idx),
            HostCommand::SendKeyToFocusedApp { key } => self.send_key(key),
            HostCommand::SimulatePathChanged { pane_id, cwd } => {
                self.simulate_path_changed(pane_id, cwd)
            }
            HostCommand::CheckCapability { pane_id, capability } => {
                self.check_capability(pane_id, capability)
            }
            HostCommand::GrantCapability { pane_id, capability } => {
                self.grant_capability(pane_id, capability)
            }
            HostCommand::DenyCapability { pane_id, capability } => {
                self.deny_capability(pane_id, capability)
            }
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
            kind: req.runtime.clone(),
            declared_capabilities: req.declared_capabilities,
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

    fn focus_pane(&mut self, pane_id: PaneId) -> Vec<HostEffect> {
        if self.context().panes.iter().any(|p| p.id == pane_id) {
            self.context_mut().focused_pane = Some(pane_id);
            vec![HostEffect::FocusChanged { pane_id: Some(pane_id) }]
        } else {
            Vec::new()
        }
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
            kind: PaneRuntimeKind::Terminal,
            declared_capabilities: Vec::new(),
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

    fn new_context(&mut self) -> Vec<HostEffect> {
        let mut ctx = HostContext::default();
        let pane_id = self.alloc_pane_id();
        ctx.panes.push(HostPane {
            id: pane_id,
            kind: PaneRuntimeKind::Terminal,
            declared_capabilities: Vec::new(),
            group: None,
        });
        ctx.focused_pane = Some(pane_id);
        self.contexts.push(ctx);
        let new_index = self.contexts.len() - 1;
        vec![ContextCreated { index: new_index }]
    }

    fn switch_context(&mut self, idx: usize) -> Vec<HostEffect> {
        if idx < self.contexts.len() {
            self.active_context = idx;
            vec![HostEffect::ContextSwitched { index: idx }]
        } else {
            Vec::new()
        }
    }

    fn send_key(&self, key: String) -> Vec<HostEffect> {
        let Some(pane) = self.focused_pane() else { return Vec::new() };
        if matches!(pane.kind, PaneRuntimeKind::App { .. }) {
            vec![HostEffect::AppKeyDispatched { pane_id: pane.id, key }]
        } else {
            Vec::new()
        }
    }

    fn simulate_path_changed(&self, pane_id: PaneId, cwd: String) -> Vec<HostEffect> {
        let Some(pane) = self.pane_by_id(pane_id) else { return Vec::new() };
        let Some(ref group) = pane.group.clone() else { return Vec::new() };
        let Some(members) = self.context().groups.get(group) else { return Vec::new() };
        let recipients: Vec<PaneId> =
            members.iter().copied().filter(|&id| id != pane_id).collect();
        if recipients.is_empty() {
            return Vec::new();
        }
        vec![HostEffect::PathBroadcasted {
            group: group.clone(),
            cwd,
            recipient_pane_ids: recipients,
        }]
    }

    fn check_capability(&self, pane_id: PaneId, capability: Capability) -> Vec<HostEffect> {
        let Some(pane) = self.pane_by_id(pane_id) else { return Vec::new() };
        if !pane.declared_capabilities.contains(&capability) {
            return vec![HostEffect::CapabilityDenied { pane_id, capability }];
        }
        let granted = self
            .context()
            .permissions
            .get(&pane_id)
            .map(|set| set.contains(&capability))
            .unwrap_or(false);
        if granted {
            vec![HostEffect::CapabilityGranted { pane_id, capability }]
        } else {
            vec![HostEffect::CapabilityPromptRequired { pane_id, capability }]
        }
    }

    fn grant_capability(&mut self, pane_id: PaneId, capability: Capability) -> Vec<HostEffect> {
        self.context_mut()
            .permissions
            .entry(pane_id)
            .or_default()
            .insert(capability.clone());
        vec![
            HostEffect::CapabilityGranted { pane_id, capability: capability.clone() },
            HostEffect::EventEmitted(HostEvent::CapabilityDecided {
                pane_id,
                capability,
                granted: true,
            }),
        ]
    }

    fn deny_capability(&mut self, pane_id: PaneId, capability: Capability) -> Vec<HostEffect> {
        vec![
            HostEffect::CapabilityDenied { pane_id, capability: capability.clone() },
            HostEffect::EventEmitted(HostEvent::CapabilityDecided {
                pane_id,
                capability,
                granted: false,
            }),
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
use HostEffect::ContextCreated;

#[cfg(test)]
mod tests {
    use crate::host::command::{HostCommand, PaneRuntimeKind};
    use crate::host::model::HostModel;
    use crate::host::services::HostServices;

    #[test]
    fn initial_state_has_one_terminal_pane() {
        let model = HostModel::new();
        assert_eq!(model.pane_count(), 1);
        assert_eq!(
            model.focused_pane().map(|p| p.kind.clone()),
            Some(PaneRuntimeKind::Terminal)
        );
        assert_eq!(model.context_count(), 1);
    }

    #[test]
    fn split_horizontal_emits_split_opened_below() {
        let mut model = HostModel::new();
        let mut svc = HostServices::new();
        let fx = model.handle_command(HostCommand::SplitHorizontal, &mut svc);
        assert_eq!(model.pane_count(), 2);
        assert!(fx.iter().any(|e| matches!(
            e,
            crate::host::effect::HostEffect::SplitOpened {
                placement: crate::host::command::Placement::Below,
                ..
            }
        )));
    }

    #[test]
    fn split_vertical_emits_split_opened_right() {
        let mut model = HostModel::new();
        let mut svc = HostServices::new();
        let fx = model.handle_command(HostCommand::SplitVertical, &mut svc);
        assert_eq!(model.pane_count(), 2);
        assert!(fx.iter().any(|e| matches!(
            e,
            crate::host::effect::HostEffect::SplitOpened {
                placement: crate::host::command::Placement::Right,
                ..
            }
        )));
    }

    #[test]
    fn new_context_creates_isolated_pane_list() {
        let mut model = HostModel::new();
        let mut svc = HostServices::new();
        model.handle_command(HostCommand::NewContext, &mut svc);
        assert_eq!(model.context_count(), 2);
        // New context starts with its own terminal pane.
        // (We haven't switched to it yet — active_context is still 0.)
        assert_eq!(model.active_context_index(), 0);
    }

    #[test]
    fn switch_context_changes_active() {
        let mut model = HostModel::new();
        let mut svc = HostServices::new();
        model.handle_command(HostCommand::NewContext, &mut svc);
        let fx = model.handle_command(HostCommand::SwitchContext(1), &mut svc);
        assert_eq!(model.active_context_index(), 1);
        assert!(fx
            .iter()
            .any(|e| matches!(e, crate::host::effect::HostEffect::ContextSwitched { index: 1 })));
    }
}
