//! Explicit desktop keyboard ownership for the permission banner.
use super::app_trait::KeyDisposition;
use super::input_router::PlexiInput;
use super::{FocusKind, PlexiApp};
use crate::broker::gate::{ApprovalChoice, PermissionMonitor};
use std::collections::HashSet;
use std::time::{Duration, Instant};

pub(super) const ARM_DELAY: Duration = Duration::from_secs(1);

#[derive(Default)]
pub(super) struct ApprovalKeyboard {
    pub id: Option<String>,
    since: Option<Instant>,
    pub confirm_always: bool,
    /// Decision keys remain owned after the banner closes until their key-up.
    /// This stops OS auto-repeat from being delivered to the previously focused pane.
    held_decision_keys: HashSet<egui::Key>,
}

impl ApprovalKeyboard {
    fn focus(&mut self, id: String, now: Instant) {
        self.id = Some(id);
        self.since = Some(now);
        self.confirm_always = false;
    }

    fn leave_focus(&mut self) {
        self.id = None;
        self.since = None;
        self.confirm_always = false;
    }
    pub fn remaining(&self, now: Instant) -> Duration {
        self.since
            .map(|since| ARM_DELAY.saturating_sub(now.saturating_duration_since(since)))
            .unwrap_or_default()
    }

    pub fn armed(&self, now: Instant) -> bool {
        self.id.is_some()
            && self
                .since
                .is_some_and(|since| now.saturating_duration_since(since) >= ARM_DELAY)
    }
}

impl PlexiApp {
    pub(crate) fn focus_approval(&mut self) {
        if self.synthetic_input_frame {
            return;
        }
        let monitor = PermissionMonitor::for_profile(&crate::config::config_dir());
        if let Some(request) = monitor.list_pending().first() {
            self.approval_keyboard
                .focus(request.pending_request_id.clone(), Instant::now());
            self.focus_stack
                .retain(|kind| *kind != FocusKind::ApprovalBanner);
            self.push_focus_layer(FocusKind::ApprovalBanner);
            log::info!("approval: focused id={}", request.pending_request_id);
            self.ctx.request_repaint_after(ARM_DELAY);
        } else if let Some(done_item) = crate::overlays::attention::open_items()
            .into_iter()
            .find(|item| item.kind == crate::broker::gate::NeedsYouKind::AgentDone)
        {
            self.approval_keyboard
                .focus(done_item.id.clone(), Instant::now());
            self.focus_stack
                .retain(|kind| *kind != FocusKind::ApprovalBanner);
            self.push_focus_layer(FocusKind::ApprovalBanner);
            log::info!("approval: focused agent_done id={}", done_item.id);
            self.ctx.request_repaint_after(ARM_DELAY);
        }
    }

    pub(super) fn leave_approval(&mut self) {
        self.approval_keyboard.leave_focus();
        self.focus_stack
            .retain(|kind| *kind != FocusKind::ApprovalBanner);
    }

    /// Called before host bindings and pane dispatch on every input frame.
    pub(crate) fn suppress_approval_decision_repeats(&mut self, input: &mut PlexiInput) {
        input.suppress_held_keys(&mut self.approval_keyboard.held_decision_keys);
    }

    pub(crate) fn approval_handle_key(&mut self, input: &mut PlexiInput) -> KeyDisposition {
        self.approval_key_at(input, Instant::now())
    }

    fn approval_key_at(&mut self, input: &mut PlexiInput, now: Instant) -> KeyDisposition {
        let Some(id) = self.approval_keyboard.id.clone() else {
            return KeyDisposition::Passthrough;
        };
        // Own text and key edges even on the frame that returns focus to the pane.
        let globals = input.events().iter().filter(|event| matches!(event,
            egui::Event::Key { modifiers, .. } if modifiers.command || modifiers.ctrl || modifiers.alt
        )).cloned().collect();
        let owned = std::mem::replace(input, PlexiInput::synthetic(globals, input.modifiers()));
        if self.synthetic_input_frame {
            return KeyDisposition::Consumed;
        }
        let monitor = PermissionMonitor::for_profile(&crate::config::config_dir());
        if monitor.show_pending(&id).is_none() {
            if let Some(done_record) = monitor.get_needs_you(&id).filter(|r| {
                r.resolution.is_none() && r.kind == crate::broker::gate::NeedsYouKind::AgentDone
            }) {
                let key = owned.events().iter().find_map(|event| match event {
                    egui::Event::Key {
                        key,
                        pressed: true,
                        repeat: false,
                        modifiers,
                        ..
                    } if *modifiers == egui::Modifiers::NONE => Some(*key),
                    _ => None,
                });
                match key {
                    Some(egui::Key::Escape) => {
                        self.leave_approval();
                    }
                    Some(egui::Key::Enter) => {
                        log::info!("approval: opening agent_done result id={id} via=keyboard");
                        self.open_agent_done_result(&done_record);
                        self.approval_keyboard
                            .held_decision_keys
                            .insert(egui::Key::Enter);
                        self.leave_approval();
                    }
                    Some(egui::Key::D) => {
                        let _ = monitor.resolve_needs_you_from(
                            &id,
                            false,
                            crate::broker::gate::ResolveSurface::Desktop,
                        );
                        self.approval_keyboard
                            .held_decision_keys
                            .insert(egui::Key::D);
                        self.leave_approval();
                    }
                    _ => {}
                }
                return KeyDisposition::Consumed;
            }
            self.leave_approval();
            return KeyDisposition::Consumed;
        }
        if !self.approval_keyboard.armed(now) {
            return KeyDisposition::Consumed;
        }
        // Exact modifiers and fresh presses only: held keys cannot become consent.
        let key = owned.events().iter().find_map(|event| match event {
            egui::Event::Key {
                key,
                pressed: true,
                repeat: false,
                modifiers,
                ..
            } if *modifiers == egui::Modifiers::NONE || *modifiers == egui::Modifiers::SHIFT => {
                Some((*key, *modifiers))
            }
            _ => None,
        });
        let choice = match key {
            Some((egui::Key::Escape, egui::Modifiers::NONE)) => {
                self.leave_approval();
                return KeyDisposition::Consumed;
            }
            Some((egui::Key::A, egui::Modifiers::SHIFT)) => {
                self.approval_keyboard.confirm_always = true;
                // Require a second deliberate press after another arm interval.
                self.approval_keyboard.since = Some(now);
                self.ctx.request_repaint_after(ARM_DELAY);
                None
            }
            Some((egui::Key::Enter, egui::Modifiers::NONE))
                if self.approval_keyboard.confirm_always =>
            {
                Some(ApprovalChoice::DenyAlways)
            }
            Some((egui::Key::A, egui::Modifiers::NONE))
                if !self.approval_keyboard.confirm_always =>
            {
                Some(ApprovalChoice::Once)
            }
            Some((egui::Key::D, egui::Modifiers::NONE))
                if !self.approval_keyboard.confirm_always =>
            {
                Some(ApprovalChoice::Deny)
            }
            _ => None,
        };
        if let Some(choice) = choice {
            match monitor.approve_pending(&id, choice) {
                Ok(()) => log::info!("approval: decided id={id} decision={choice:?} via=keyboard"),
                Err(error) => log::warn!("approval: decision failed id={id} via=keyboard: {error}"),
            }
            if matches!(
                choice,
                ApprovalChoice::Once | ApprovalChoice::Deny | ApprovalChoice::DenyAlways
            ) {
                if let Some((key, _)) = key {
                    self.approval_keyboard.held_decision_keys.insert(key);
                }
            }
            self.leave_approval();
        }
        KeyDisposition::Consumed
    }

    // The nonmodal banner is painted once, alongside the other desktop banners.
    pub(crate) fn approval_focus_draw(&mut self, _ctx: &egui::Context) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::HostHarness;

    fn pending(monitor: &PermissionMonitor, name: &str, root: &std::path::Path) -> String {
        let result = crate::workspace::secrets::read_pane_secret(
            monitor,
            &crate::workspace::secrets::InMemoryKeychain::new(),
            &crate::workspace::secrets::SecretActor {
                kind: crate::broker::ActorType::Agent,
                id: "pane:4".into(),
            },
            name,
            &crate::workspace::secrets::SecretScope::Workspace {
                id: "ws".into(),
                root: root.to_owned(),
            },
            root,
        )
        .unwrap();
        match result {
            crate::workspace::secrets::ReadResult::PermissionRequired { pending_id, .. } => {
                pending_id
            }
            _ => panic!("expected permission request"),
        }
    }
    fn input(key: egui::Key, modifiers: egui::Modifiers) -> PlexiInput {
        let ctx = egui::Context::default();
        ctx.input_mut(|i| {
            i.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            })
        });
        PlexiInput::take_from(&ctx)
    }
    #[test]
    fn approval_keyboard_arm_oldest_escape_and_pane_typing() {
        let mut h = HostHarness::new();
        let root = tempfile::tempdir().unwrap();
        let monitor = PermissionMonitor::for_profile(&crate::config::config_dir());
        let first = pending(&monitor, "FIRST", root.path());
        let second = pending(&monitor, "SECOND", root.path());
        let now = Instant::now();
        assert_eq!(
            h.app
                .approval_key_at(&mut input(egui::Key::A, egui::Modifiers::NONE), now),
            KeyDisposition::Passthrough
        );
        assert_eq!(monitor.list_pending().len(), 2);
        h.app.focus_approval();
        assert_eq!(h.app.approval_keyboard.id.as_ref(), Some(&first));
        h.app.approval_keyboard.since = Some(now);
        h.app.approval_key_at(
            &mut input(egui::Key::A, egui::Modifiers::NONE),
            now + ARM_DELAY - Duration::from_millis(1),
        );
        assert_eq!(monitor.list_pending().len(), 2);
        h.app.approval_key_at(
            &mut input(egui::Key::Escape, egui::Modifiers::NONE),
            now + ARM_DELAY,
        );
        assert!(h.app.approval_keyboard.id.is_none());
        assert!(monitor.open_needs_you().iter().any(|row| row.id == first));
        h.app.focus_approval();
        h.app.approval_keyboard.since = Some(now);
        h.app.approval_key_at(
            &mut input(egui::Key::A, egui::Modifiers::NONE),
            now + ARM_DELAY,
        );
        assert!(monitor.show_pending(&first).is_none());
        h.app.focus_approval();
        assert_eq!(h.app.approval_keyboard.id.as_ref(), Some(&second));
        h.app.approval_keyboard.since = Some(now);
        h.app.approval_key_at(
            &mut input(egui::Key::D, egui::Modifiers::NONE),
            now + ARM_DELAY,
        );
        assert!(monitor.list_pending().is_empty());
    }
    #[test]
    fn approval_keyboard_rejects_synthetic_and_repeat_input() {
        let mut h = HostHarness::new();
        let root = tempfile::tempdir().unwrap();
        let monitor = PermissionMonitor::for_profile(&crate::config::config_dir());
        let id = pending(&monitor, "SECRET", root.path());
        h.app.focus_approval();
        let now = Instant::now();
        h.app.approval_keyboard.since = Some(now - ARM_DELAY);
        h.app.synthetic_input_frame = true;
        h.app
            .approval_key_at(&mut input(egui::Key::A, egui::Modifiers::NONE), now);
        assert!(monitor.show_pending(&id).is_some());
        h.app.synthetic_input_frame = false;
        let mut repeated = PlexiInput::synthetic(
            vec![egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: true,
                modifiers: egui::Modifiers::NONE,
            }],
            egui::Modifiers::NONE,
        );
        h.app.approval_key_at(&mut repeated, now);
        assert!(monitor.show_pending(&id).is_some());
        assert!(repeated.is_empty());
    }

    #[test]
    fn approval_keyboard_always_requires_confirmation_and_rearms() {
        let mut h = HostHarness::new();
        let root = tempfile::tempdir().unwrap();
        let monitor = PermissionMonitor::for_profile(&crate::config::config_dir());
        let id = pending(&monitor, "SECRET", root.path());
        h.app.focus_approval();
        let now = Instant::now();
        h.app.approval_keyboard.since = Some(now - ARM_DELAY);
        h.app
            .approval_key_at(&mut input(egui::Key::A, egui::Modifiers::SHIFT), now);
        assert!(h.app.approval_keyboard.confirm_always);
        h.app
            .approval_key_at(&mut input(egui::Key::Enter, egui::Modifiers::NONE), now);
        assert!(monitor.show_pending(&id).is_some());
        h.app.approval_key_at(
            &mut input(egui::Key::Enter, egui::Modifiers::NONE),
            now + ARM_DELAY,
        );
        assert!(monitor.show_pending(&id).is_none());
    }

    #[test]
    fn approval_decision_key_stays_owned_until_release() {
        let mut h = HostHarness::new();
        let root = tempfile::tempdir().unwrap();
        let monitor = PermissionMonitor::for_profile(&crate::config::config_dir());
        let id = pending(&monitor, "SECRET", root.path());
        h.app.focus_approval();
        let now = Instant::now();
        h.app.approval_keyboard.since = Some(now - ARM_DELAY);
        h.app
            .approval_key_at(&mut input(egui::Key::A, egui::Modifiers::NONE), now);
        assert!(monitor.show_pending(&id).is_none());

        let mut held_repeat = PlexiInput::synthetic(
            vec![
                egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::Text("a".to_string()),
            ],
            egui::Modifiers::NONE,
        );
        h.app.suppress_approval_decision_repeats(&mut held_repeat);
        assert!(
            held_repeat.is_empty(),
            "held approval key must not reach the pane"
        );

        let mut release = PlexiInput::synthetic(
            vec![egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            egui::Modifiers::NONE,
        );
        h.app.suppress_approval_decision_repeats(&mut release);
        assert!(release.is_empty(), "approval key-up remains host-owned");

        let mut next_key = input(egui::Key::A, egui::Modifiers::NONE);
        h.app.suppress_approval_decision_repeats(&mut next_key);
        assert!(
            !next_key.is_empty(),
            "input resumes after the decision key is released"
        );
    }

    #[test]
    fn human_started_agent_done_surfaces_persists_opens_result_and_survives_close() {
        let mut h = HostHarness::new();
        let root = h.workspace_root();
        h.app.set_context_root(root.clone(), None);
        let (pane_id, _) = h
            .app
            .spawn_agent_worker_pane(
                "researcher",
                "pi",
                "sleep 30",
                "Audit auth",
                None,
                false,
                None,
                None,
                None,
                None,
                false,
                None,
            )
            .unwrap();
        if let Some((wi, _)) = h.app.find_pane_in_any_window(pane_id) {
            if let Some(crate::host::pane::Pane::Terminal(t)) = h.app.windows[wi].panes.get_mut(&pane_id) {
                t.spawned_by = Some("human".to_string());
                t.agent_session_id = Some("sess-0962".to_string());
                t.initial_cwd = Some(root.clone());
            }
        }
        h.app.agent_attention.pending_os.clear();

        let done_reply = root.join("done.json");
        h.inject_ipc(crate::protocol::AppRequest::AgentDone { peer_ancestry: None,
            pane_id,
            result: Some("{\"answer\":\"42\"}".into()),
            summary: Some("Audit complete".into()),
            response_file: done_reply.to_string_lossy().into_owned(),
        });
        h.hidden_frame();

        let reply: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&done_reply).unwrap()).unwrap();
        assert_eq!(reply["ok"], true);
        let md_path = std::path::PathBuf::from(reply["result_path"].as_str().unwrap());
        let json_path = std::path::PathBuf::from(reply["json_path"].as_str().unwrap());
        assert!(md_path.exists(), "RESULT.md must be persisted on disk");
        assert!(json_path.exists(), "result.json must be persisted on disk");
        assert!(
            std::fs::read_to_string(&md_path)
                .unwrap()
                .contains("Audit complete")
        );

        // OS notification queued for human-started completed agent.
        h.app.agent_attention.reports.remove(&pane_id);
        h.app.report_agent_attention(
            pane_id,
            &crate::protocol::AgentState::Completed,
            Some("Audit complete"),
        );
        assert!(
            h.app.agent_attention.pending_os.iter().any(|(id, state, _)| {
                *id == pane_id && *state == crate::protocol::AgentState::Completed
            }),
            "human-started agent done must queue an OS notification"
        );

        // AgentDone NeedsYou item is open with "<agent> done: <summary>".
        let monitor = PermissionMonitor::for_profile(&crate::config::config_dir());
        let open = monitor.open_needs_you();
        let done_item = open
            .iter()
            .find(|r| r.kind == crate::broker::gate::NeedsYouKind::AgentDone)
            .expect("human-started agent done must file a NeedsYouKind::AgentDone item");
        assert_eq!(done_item.summary, "researcher done: Audit complete");

        // Agent state remains Completed (not overridden to NeedsYou).
        let agent_input = h
            .app
            .agent_inputs()
            .into_iter()
            .find(|a| a.pane_id == pane_id)
            .unwrap();
        assert_eq!(agent_input.state, crate::protocol::AgentState::Completed);

        // Keyboard-only flow: focus_approval (Cmd+Alt+P) -> Enter opens RESULT.md in text-editor split and settles item.
        let now = Instant::now();
        h.app.focus_approval();
        assert_eq!(h.app.approval_keyboard.id.as_ref(), Some(&done_item.id));
        h.app.approval_keyboard.since = Some(now - ARM_DELAY);
        h.app
            .approval_key_at(&mut input(egui::Key::Enter, egui::Modifiers::NONE), now);
        assert!(
            monitor
                .open_needs_you()
                .iter()
                .all(|r| r.id != done_item.id),
            "Enter must settle the AgentDone attention item"
        );
        assert!(
            h.app
                .windows
                .iter()
                .flat_map(|w| w.panes.values())
                .any(|p| matches!(p, crate::host::pane::Pane::App(app) if app.manifest_id == "text-editor")),
            "Enter must open RESULT.md in a text-editor viewer pane"
        );

        // Close the pane and verify `read_persisted_agent_result_in` still resolves by pane_id, session_id, role, and latest.
        h.app.close_pane_by_id(pane_id);
        for selector in [
            None,
            Some(pane_id.to_string()),
            Some("sess-0962".to_string()),
            Some("researcher".to_string()),
        ] {
            let (_, _, persisted) = crate::cli::agent::read_persisted_agent_result_in(
                selector.as_deref(),
                Some(&root),
                &crate::config::config_dir(),
            )
            .expect("persisted result must remain readable after pane close");
            assert_eq!(persisted["summary"], "Audit complete");
            assert_eq!(persisted["result"]["answer"], "42");
        }

        // Verify a child agent with a parent agent does NOT file a human AgentDone item.
        let (parent, _) = h
            .app
            .spawn_agent_worker_pane(
                "lead",
                "pi",
                "sleep 30",
                "Lead task",
                None,
                false,
                None,
                None,
                None,
                None,
                false,
                None,
            )
            .unwrap();
        let (child, _) = h
            .app
            .spawn_agent_worker_pane(
                "worker",
                "pi",
                "sleep 30",
                "Subtask",
                None,
                true,
                Some(parent),
                None,
                None,
                None,
                false,
                None,
            )
            .unwrap();
        let child_reply = root.join("child_done.json");
        h.inject_ipc(crate::protocol::AppRequest::AgentDone { peer_ancestry: None,
            pane_id: child,
            result: Some("{\"sub\":1}".into()),
            summary: Some("Child done".into()),
            response_file: child_reply.to_string_lossy().into_owned(),
        });
        h.hidden_frame();
        assert!(
            monitor
                .open_needs_you()
                .iter()
                .all(|r| r.kind != crate::broker::gate::NeedsYouKind::AgentDone),
            "child agent with parent agent must not file a human AgentDone attention item"
        );
    }
}
