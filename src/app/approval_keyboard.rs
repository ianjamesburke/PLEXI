//! Explicit desktop keyboard ownership for the permission banner.
use super::app_trait::KeyDisposition;
use super::input_router::PlexiInput;
use super::{FocusKind, PlexiApp};
use crate::broker::gate::{ApprovalChoice, PermissionMonitor};
use std::time::{Duration, Instant};

pub(super) const ARM_DELAY: Duration = Duration::from_secs(1);

#[derive(Default)]
pub(super) struct ApprovalKeyboard {
    pub id: Option<String>,
    since: Option<Instant>,
    pub confirm_always: bool,
}

impl ApprovalKeyboard {
    fn focus(&mut self, id: String, now: Instant) {
        self.id = Some(id);
        self.since = Some(now);
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
        }
    }

    pub(super) fn leave_approval(&mut self) {
        self.approval_keyboard = Default::default();
        self.focus_stack
            .retain(|kind| *kind != FocusKind::ApprovalBanner);
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
}
