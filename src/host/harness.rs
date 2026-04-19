use crate::host::command::{
    Direction, HostCommand, OpenPaneRequest, PaneRuntimeKind,
};
use crate::host::effect::HostEffect;
use crate::host::model::HostModel;
use crate::host::services::HostServices;
use crate::tiling::PaneId;

/// Test driver for Layer 2 — drives HostModel directly, collects effects.
pub struct HostHarness {
    pub model: HostModel,
    pub services: HostServices,
    pub effects: Vec<HostEffect>,
}

impl HostHarness {
    pub fn new() -> Self {
        Self {
            model: HostModel::new(),
            services: HostServices::new(),
            effects: Vec::new(),
        }
    }

    pub fn send(&mut self, cmd: HostCommand) -> &[HostEffect] {
        let start = self.effects.len();
        let new = self.model.handle_command(cmd, &mut self.services);
        self.effects.extend(new);
        &self.effects[start..]
    }

    pub fn press(&mut self, chord: &str) {
        let cmd = match chord {
            "cmd+w" => Some(HostCommand::CloseFocusedPane),
            "tab" => Some(HostCommand::Navigate(Direction::Right)),
            "shift+tab" => Some(HostCommand::Navigate(Direction::Left)),
            "cmd+d" => Some(HostCommand::SplitVertical),
            "cmd+shift+d" => Some(HostCommand::SplitHorizontal),
            _ => None,
        };
        if let Some(c) = cmd {
            self.send(c);
        }
    }

    pub fn launch_app(&mut self, app_id: &str) -> PaneId {
        let req = OpenPaneRequest::app(app_id);
        let fx = self.send(HostCommand::OpenPane(req));
        fx.iter()
            .find_map(|e| {
                if let HostEffect::PaneOpened { pane_id, .. } = e {
                    Some(*pane_id)
                } else {
                    None
                }
            })
            .expect("OpenPane must emit PaneOpened")
    }

    pub fn launch_app_in_group(&mut self, app_id: &str, group: &str) -> PaneId {
        let mut req = OpenPaneRequest::app(app_id);
        req.group = Some(group.to_string());
        let fx = self.send(HostCommand::OpenPane(req));
        fx.iter()
            .find_map(|e| {
                if let HostEffect::PaneOpened { pane_id, .. } = e {
                    Some(*pane_id)
                } else {
                    None
                }
            })
            .expect("OpenPane must emit PaneOpened")
    }

    pub fn launch_app_with_capabilities(
        &mut self,
        app_id: &str,
        caps: &[&str],
    ) -> PaneId {
        let mut req = OpenPaneRequest::app(app_id);
        req.declared_capabilities = caps.iter().map(|s| s.to_string()).collect();
        let fx = self.send(HostCommand::OpenPane(req));
        fx.iter()
            .find_map(|e| {
                if let HostEffect::PaneOpened { pane_id, .. } = e {
                    Some(*pane_id)
                } else {
                    None
                }
            })
            .expect("OpenPane must emit PaneOpened")
    }

    // ── assertions ──────────────────────────────────────────────────────────

    pub fn assert_pane_count(&self, expected: usize) {
        assert_eq!(self.model.pane_count(), expected, "pane_count mismatch");
    }

    pub fn assert_focused_kind(&self, expected: PaneRuntimeKind) {
        assert_eq!(
            self.model.focused_pane().map(|p| p.kind.clone()),
            Some(expected),
            "focused pane kind mismatch"
        );
    }

    pub fn assert_focused_id(&self, expected: PaneId) {
        assert_eq!(
            self.model.focused_pane().map(|p| p.id),
            Some(expected),
            "focused pane ID mismatch"
        );
    }

    pub fn assert_last_effect(&self, expected: &HostEffect) {
        let last = self.effects.last().expect("no effects emitted");
        assert_eq!(last, expected, "last effect mismatch");
    }

    pub fn assert_has_effect(&self, expected: &HostEffect) {
        assert!(
            self.effects.contains(expected),
            "expected effect not found: {expected:?}"
        );
    }

    pub fn assert_no_effect_matching(&self, predicate: impl Fn(&HostEffect) -> bool) {
        assert!(
            !self.effects.iter().any(predicate),
            "unexpected effect was emitted"
        );
    }

    pub fn clear_effects(&mut self) {
        self.effects.clear();
    }

    pub fn grant(&mut self, pane_id: PaneId, capability: &str) {
        self.send(HostCommand::GrantCapability {
            pane_id,
            capability: capability.to_string(),
        });
    }
}

// ── Layer 2 test suite ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use crate::host::command::{Direction, HostCommand, Placement, PaneRuntimeKind};
    use crate::host::effect::HostEffect;
    use crate::host::harness::HostHarness;

    #[test]
    fn open_close_cycle() {
        let mut h = HostHarness::new();
        let id = h.launch_app("todo");
        h.assert_pane_count(2);
        h.assert_focused_kind(PaneRuntimeKind::App { app_id: "todo".to_string() });

        h.press("cmd+w");
        h.assert_pane_count(1);
        h.assert_focused_kind(PaneRuntimeKind::Terminal);

        assert!(h.effects.iter().any(|e| matches!(e, HostEffect::PaneClosed { pane_id } if *pane_id == id)));
    }

    #[test]
    fn focus_pane_by_id() {
        let mut h = HostHarness::new();
        let app_id = h.launch_app("wiki");
        // Focus back to the initial terminal (id=0).
        h.send(HostCommand::FocusPane(0));
        h.assert_focused_id(0);
        // Focus the app pane again.
        h.send(HostCommand::FocusPane(app_id));
        h.assert_focused_id(app_id);
    }

    #[test]
    fn focus_unknown_pane_is_noop() {
        let mut h = HostHarness::new();
        let fx = h.send(HostCommand::FocusPane(999));
        assert!(fx.is_empty(), "focus on nonexistent pane must produce no effects");
    }

    #[test]
    fn navigate_cycles_through_panes() {
        let mut h = HostHarness::new();
        h.launch_app("a");
        h.launch_app("b");
        // State: panes [0:term, 1:a, 2:b], focused=2
        h.assert_pane_count(3);
        h.assert_focused_kind(PaneRuntimeKind::App { app_id: "b".to_string() });

        h.press("tab"); // Navigate Right → wraps to pane[0]
        h.assert_focused_kind(PaneRuntimeKind::Terminal);

        h.press("shift+tab"); // Navigate Left → wraps to pane[2]
        h.assert_focused_kind(PaneRuntimeKind::App { app_id: "b".to_string() });
    }

    #[test]
    fn navigate_single_pane_stays_focused() {
        let mut h = HostHarness::new();
        h.assert_pane_count(1);
        h.send(HostCommand::Navigate(Direction::Right));
        h.assert_focused_kind(PaneRuntimeKind::Terminal);
    }

    #[test]
    fn split_horizontal_creates_terminal_below() {
        let mut h = HostHarness::new();
        h.press("cmd+shift+d");
        h.assert_pane_count(2);
        assert!(h.effects.iter().any(|e| matches!(
            e,
            HostEffect::SplitOpened { placement: Placement::Below, .. }
        )));
    }

    #[test]
    fn split_vertical_creates_terminal_right() {
        let mut h = HostHarness::new();
        h.press("cmd+d");
        h.assert_pane_count(2);
        assert!(h.effects.iter().any(|e| matches!(
            e,
            HostEffect::SplitOpened { placement: Placement::Right, .. }
        )));
    }

    #[test]
    fn new_context_then_switch() {
        let mut h = HostHarness::new();
        h.send(HostCommand::NewContext);
        assert_eq!(h.model.context_count(), 2);
        assert_eq!(h.model.active_context_index(), 0);

        h.send(HostCommand::SwitchContext(1));
        assert_eq!(h.model.active_context_index(), 1);
        // New context always has exactly 1 pane (terminal).
        h.assert_pane_count(1);
        h.assert_focused_kind(PaneRuntimeKind::Terminal);
    }

    #[test]
    fn switch_to_out_of_range_context_is_noop() {
        let mut h = HostHarness::new();
        let fx = h.send(HostCommand::SwitchContext(99));
        assert!(fx.is_empty());
        assert_eq!(h.model.active_context_index(), 0);
    }

    #[test]
    fn send_key_dispatches_to_app_pane() {
        let mut h = HostHarness::new();
        let app_pane_id = h.launch_app("snake");
        h.clear_effects();

        let fx = h.send(HostCommand::SendKeyToFocusedApp { key: "j".to_string() });
        assert_eq!(
            fx,
            &[HostEffect::AppKeyDispatched { pane_id: app_pane_id, key: "j".to_string() }]
        );
    }

    #[test]
    fn send_key_ignored_when_terminal_focused() {
        let mut h = HostHarness::new();
        h.assert_focused_kind(PaneRuntimeKind::Terminal);
        let fx = h.send(HostCommand::SendKeyToFocusedApp { key: "j".to_string() });
        assert!(fx.is_empty());
    }

    #[test]
    fn path_changed_broadcasts_to_group_members() {
        let mut h = HostHarness::new();
        let a = h.launch_app_in_group("todo", "dev");
        let b = h.launch_app_in_group("wiki", "dev");
        h.clear_effects();

        let fx = h.send(HostCommand::SimulatePathChanged {
            pane_id: a,
            cwd: "/src".to_string(),
        });
        assert_eq!(fx.len(), 1);
        match &fx[0] {
            HostEffect::PathBroadcasted { group, cwd, recipient_pane_ids } => {
                assert_eq!(group, "dev");
                assert_eq!(cwd, "/src");
                assert_eq!(recipient_pane_ids, &[b]);
            }
            other => panic!("expected PathBroadcasted, got {other:?}"),
        }
    }

    #[test]
    fn path_changed_for_ungrouped_pane_is_noop() {
        let mut h = HostHarness::new();
        // The initial terminal (id=0) has no group.
        let fx = h.send(HostCommand::SimulatePathChanged {
            pane_id: 0,
            cwd: "/home".to_string(),
        });
        assert!(fx.is_empty());
    }

    #[test]
    fn path_changed_sole_group_member_is_noop() {
        let mut h = HostHarness::new();
        let a = h.launch_app_in_group("todo", "solo");
        let fx = h.send(HostCommand::SimulatePathChanged {
            pane_id: a,
            cwd: "/x".to_string(),
        });
        assert!(fx.is_empty(), "no recipients when pane is sole group member");
    }

    #[test]
    fn closing_group_member_removes_from_group() {
        let mut h = HostHarness::new();
        let a = h.launch_app_in_group("todo", "dev");
        let b = h.launch_app_in_group("wiki", "dev");

        // Close b (it's currently focused).
        h.assert_focused_id(b);
        h.press("cmd+w");

        // After closing b, a is now sole member → broadcast is noop.
        h.clear_effects();
        let fx = h.send(HostCommand::SimulatePathChanged {
            pane_id: a,
            cwd: "/y".to_string(),
        });
        assert!(fx.is_empty(), "closed pane must be removed from group");
    }

    #[test]
    fn capability_undeclared_is_denied() {
        let mut h = HostHarness::new();
        let pane = h.launch_app("notes");
        let fx = h.send(HostCommand::CheckCapability {
            pane_id: pane,
            capability: "fs.write".to_string(),
        });
        assert_eq!(
            fx,
            &[HostEffect::CapabilityDenied {
                pane_id: pane,
                capability: "fs.write".to_string()
            }]
        );
    }

    #[test]
    fn capability_declared_unpermitted_requires_prompt() {
        let mut h = HostHarness::new();
        let pane = h.launch_app_with_capabilities("notes", &["fs.write"]);
        let fx = h.send(HostCommand::CheckCapability {
            pane_id: pane,
            capability: "fs.write".to_string(),
        });
        assert_eq!(
            fx,
            &[HostEffect::CapabilityPromptRequired {
                pane_id: pane,
                capability: "fs.write".to_string()
            }]
        );
    }

    #[test]
    fn grant_then_check_capability_is_granted() {
        let mut h = HostHarness::new();
        let pane = h.launch_app_with_capabilities("notes", &["fs.write"]);
        h.grant(pane, "fs.write");
        h.clear_effects();

        let fx = h.send(HostCommand::CheckCapability {
            pane_id: pane,
            capability: "fs.write".to_string(),
        });
        assert_eq!(
            fx,
            &[HostEffect::CapabilityGranted {
                pane_id: pane,
                capability: "fs.write".to_string()
            }]
        );
    }

    #[test]
    fn deny_capability_command_emits_denied() {
        let mut h = HostHarness::new();
        let pane = h.launch_app_with_capabilities("notes", &["fs.write"]);
        h.clear_effects();
        let fx = h.send(HostCommand::DenyCapability {
            pane_id: pane,
            capability: "fs.write".to_string(),
        });
        assert!(fx
            .iter()
            .any(|e| matches!(e, HostEffect::CapabilityDenied { .. })));
    }

    #[test]
    fn open_pane_emits_pane_opened_and_focus_changed() {
        let mut h = HostHarness::new();
        let req = crate::host::command::OpenPaneRequest::terminal();
        let fx = h.send(HostCommand::OpenPane(req));
        assert!(fx.iter().any(|e| matches!(e, HostEffect::PaneOpened { .. })));
        assert!(fx.iter().any(|e| matches!(e, HostEffect::FocusChanged { .. })));
    }

    #[test]
    fn open_pane_carries_share_to_effect() {
        use crate::host::command::{OpenPaneRequest, ShareRatio};
        let mut h = HostHarness::new();
        let mut req = OpenPaneRequest::app("todo");
        req.share = ShareRatio::new(0.4, 0.6).expect("valid fraction");
        let fx = h.send(HostCommand::OpenPane(req));
        let opened = fx
            .iter()
            .find_map(|e| match e {
                HostEffect::PaneOpened { share, .. } => Some(*share),
                _ => None,
            })
            .expect("PaneOpened effect required");
        assert_eq!(opened.numerator, 0.4);
        assert_eq!(opened.denominator, 0.6);
    }
}
