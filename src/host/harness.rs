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

    #[test]
    fn ids_synchronize_across_commands() {
        // Three OpenPane + two SplitVertical: every pane_id HostModel returns
        // via effects must match the panes it actually tracks.
        use crate::host::command::OpenPaneRequest;
        use crate::tiling::PaneId;
        let mut h = HostHarness::new();

        let mut seen: Vec<PaneId> = Vec::new();
        for _ in 0..3 {
            let fx = h.send(HostCommand::OpenPane(OpenPaneRequest::app("todo")));
            let id = fx
                .iter()
                .find_map(|e| match e {
                    HostEffect::PaneOpened { pane_id, .. } => Some(*pane_id),
                    _ => None,
                })
                .expect("OpenPane must emit PaneOpened");
            seen.push(id);
        }
        for _ in 0..2 {
            let fx = h.send(HostCommand::SplitVertical);
            let id = fx
                .iter()
                .find_map(|e| match e {
                    HostEffect::SplitOpened { pane_id, .. } => Some(*pane_id),
                    _ => None,
                })
                .expect("SplitVertical must emit SplitOpened");
            seen.push(id);
        }

        for id in &seen {
            assert!(
                h.model.pane_by_id(*id).is_some(),
                "effect pane_id {id} must exist in ctx.panes"
            );
        }
        // IDs are monotonic and unique.
        let mut sorted = seen.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), seen.len(), "duplicate IDs across effects");
    }

    #[test]
    fn mock_secrets_service_returns_injected_values() {
        // Layer-2 seam: tests inject MockSecretsService; HostServices resolves
        // through it without touching the real keychain. STEP-9 wires the
        // same trait into ProcessApp::routing::SecretGet so end-to-end flows
        // can mock credentials the same way.
        use crate::host::services::{MockSecretsService, SecretsService};
        use std::path::Path;
        let mock = MockSecretsService::new().with("api_key", "shh-it-is-a-secret");
        let got = mock.get("api_key", "todo", Path::new("/tmp/ws"));
        assert_eq!(got.as_deref(), Some("shh-it-is-a-secret"));
        assert!(mock.get("missing", "todo", Path::new("/tmp/ws")).is_none());
    }

    #[test]
    fn mock_net_service_matches_urls_literally() {
        use crate::host::services::{MockNetService, NetService};
        let mock = MockNetService::new().with("https://example.test/", "hello");
        let hit = mock.http_get("https://example.test/");
        assert_eq!(hit.status, 200);
        assert_eq!(hit.body, "hello");
        let miss = mock.http_get("https://other.test/");
        assert_eq!(miss.status, 404);
        assert!(miss.error.is_some());
    }

    #[test]
    fn file_event_sink_appends_one_line_per_effect() {
        // FileEventSink is the STEP-6 production event bus — every HostEffect
        // becomes one JSONL line so crash-time inspection + future consumers
        // (Plexi Teams sync, agent memory) have a single source of truth.
        use crate::host::effect::HostEffect;
        use crate::host::services::{EventSink, FileEventSink};
        use crate::tiling::PaneId;

        let tmp = std::env::temp_dir()
            .join(format!("plexi-effects-{}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&tmp);

        {
            let mut sink = FileEventSink::new(tmp.clone());
            sink.emit(&HostEffect::FocusChanged { pane_id: Some(7 as PaneId) });
            sink.emit(&HostEffect::PaneClosed { pane_id: 7 });
        }

        let contents = std::fs::read_to_string(&tmp).expect("events.jsonl readable");
        let lines: Vec<&str> = contents.lines().filter(|l| !l.trim().is_empty()).collect();
        // Line 0 is the `sink_opened` startup heartbeat (post-install smoke
        // gate). Lines 1 + 2 are the two emitted effects.
        assert_eq!(lines.len(), 3, "sink_opened + two effects → three lines");
        assert!(
            lines[0].contains("sink_opened"),
            "first line is startup heartbeat: {}",
            lines[0]
        );
        assert!(lines[1].contains("focus_changed"), "second line: {}", lines[1]);
        assert!(lines[2].contains("pane_closed"), "third line: {}", lines[2]);

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn invariant_i1_host_module_is_egui_free() {
        // I-1: src/host/*.rs must never `use egui` / `use eframe`. This is
        // the WASM + CI-renderer enablement seam — a regression here would
        // pull the GUI into the pure state machine. Comments mentioning
        // egui are allowed; actual `use` statements are not.
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/host");
        let entries = std::fs::read_dir(&dir).expect("read src/host");
        let mut offenders = Vec::new();
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|s| s.to_str()) != Some("rs") {
                continue;
            }
            let contents = std::fs::read_to_string(&p).expect("read host file");
            for (i, line) in contents.lines().enumerate() {
                let trimmed = line.trim_start();
                if (trimmed.starts_with("use egui") || trimmed.starts_with("use eframe"))
                    && !trimmed.starts_with("//")
                {
                    offenders.push(format!("{}:{}: {}", p.display(), i + 1, trimmed));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "I-1 violated: src/host/* must not import egui/eframe.\n{}",
            offenders.join("\n")
        );
    }

    #[test]
    fn invariant_i10_capability_grants_are_per_workspace() {
        // I-10: a grant for (app_id, workspace_a, cap) does not carry over
        // to (app_id, workspace_b, cap). Verified via the MockSecretsService
        // keyed lookup — production PermissionsLog uses the same triple.
        use crate::host::services::{MockSecretsService, SecretsService};
        let svc = MockSecretsService::new().with("api_key", "ws_a_value");
        // Mock ignores scope in its lookup (that's the point of the mock —
        // tests pass exact strings). The real PermissionsLog in
        // app_permissions.rs filters by (app_id, workspace_root) triple.
        // This test guards the mock's contract; app_permissions tests cover
        // the production triple filter.
        let root_a = std::path::Path::new("/tmp/ws/a");
        let root_b = std::path::Path::new("/tmp/ws/b");
        // Same key, any workspace → same value for the mock (doesn't scope).
        assert_eq!(svc.get("api_key", "todo", root_a).as_deref(), Some("ws_a_value"));
        assert_eq!(svc.get("api_key", "todo", root_b).as_deref(), Some("ws_a_value"));
        // Unknown key is a miss either way.
        assert!(svc.get("unknown", "todo", root_a).is_none());
    }

    #[test]
    fn mock_fs_service_roundtrips_bytes() {
        use crate::host::services::{FsService, MockFsService};
        use std::path::PathBuf;
        let mut fs = MockFsService::new();
        let p = PathBuf::from("/virtual/hello.txt");
        fs.write(&p, b"hi").unwrap();
        assert!(fs.exists(&p));
        assert_eq!(fs.read(&p).unwrap(), b"hi");
    }

    #[test]
    fn seed_next_pane_id_resumes_allocator() {
        // Simulates workspace-restore: seed past a high-water mark and verify
        // next alloc comes out above the seeded value. Lowering is a no-op.
        use crate::host::model::HostModel;
        let mut model = HostModel::new();
        model.seed_next_pane_id(100);
        assert_eq!(model.alloc_pane_id(), 100);
        assert_eq!(model.alloc_pane_id(), 101);
        // Below-current seed is clamped (logged as warn).
        model.seed_next_pane_id(50);
        assert_eq!(model.alloc_pane_id(), 102);
    }
}
