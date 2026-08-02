use super::super::*;
use crate::app::app_trait::AppCommand;
use crate::host::context::Window;
use crate::testing::HostHarness;

fn same_workspace_window_below(window_id: u64, pane_id: u64) -> Window {
    let mut tree = egui_tiles::Tree::empty("test_tree_below");
    let tile = tree.tiles.insert_pane(pane_id);
    tree.root = Some(tile);
    Window {
        name: "Test".into(),
        path: std::env::temp_dir(),
        tree,
        panes: HashMap::new(),
        focused_pane: None,
        zoomed_pane: None,
        grid_x: 0,
        grid_y: 1,
        window_id,
        context_id: 1, // same workspace as window 0
    }
}

/// #840: A snoozed notification must become invisible immediately after
/// deliver_after is set, and visible again once the instant has elapsed.
#[test]
fn snoozed_notification_invisible_then_visible() {
    let mut h = HostHarness::new();

    // Push a notification that is already past its snooze window.
    let wake_past = std::time::Instant::now() - std::time::Duration::from_secs(1);
    h.app.pending_notifications.push(PendingNotification {
        notify_id: "snoozed-woken".into(),
        sender_pane_id: 0,
        source_context_id: h.app.router.active().context_id,
        source_window_id: h.app.windows[h.app.active_window].window_id,
        title: "Snoozed".into(),
        body: "body".into(),
        kind: crate::app_protocol::NotifyKind::Message,
        options: vec![],
        input_prompt: None,
        required: false,
        scope: crate::app_protocol::NotifyScope::Global,
        image_inline: None,
        image_pipe_id: None,
        response_file: None,
        timeout_secs: None,
        on_dismiss: None,
        enqueued_at: std::time::Instant::now(),
        tombstoned: false,
        deliver_after: Some(wake_past), // already elapsed → visible
    });

    assert_eq!(
        h.app.visible_notification_count(),
        1,
        "past-snooze notification must be visible"
    );

    // Now set deliver_after to the future (active snooze).
    let wake_future = std::time::Instant::now() + std::time::Duration::from_secs(300);
    h.app.pending_notifications[0].deliver_after = Some(wake_future);

    assert_eq!(
        h.app.visible_notification_count(),
        0,
        "future-snooze notification must be invisible"
    );
}

/// #840: tick_notification_timeouts must not time out a snoozed notification.
#[test]
fn snoozed_notification_exempt_from_timeout() {
    let mut h = HostHarness::new();

    let sender_id = h.add_test_pane();
    // Enqueued 10 minutes ago with a 60s timeout, but snoozed into the future.
    h.app.pending_notifications.push(PendingNotification {
        notify_id: "snoozed-no-timeout".into(),
        sender_pane_id: sender_id,
        source_context_id: h.app.router.active().context_id,
        source_window_id: h.app.windows[h.app.active_window].window_id,
        title: "ShouldNotTimeout".into(),
        body: "body".into(),
        kind: crate::app_protocol::NotifyKind::Message,
        options: vec![],
        input_prompt: None,
        required: false,
        scope: crate::app_protocol::NotifyScope::Global,
        image_inline: None,
        image_pipe_id: None,
        response_file: None,
        timeout_secs: Some(60),
        on_dismiss: None,
        enqueued_at: std::time::Instant::now() - std::time::Duration::from_secs(600),
        tombstoned: false,
        deliver_after: Some(std::time::Instant::now() + std::time::Duration::from_secs(300)),
    });

    h.app.tick_notification_timeouts();

    assert_eq!(
        h.app.pending_notifications.len(),
        1,
        "snoozed notification must survive tick_notification_timeouts"
    );
}

#[test]
fn persist_roundtrip() {
    let dir = std::env::temp_dir().join(format!("plexi_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("notifications.json");

    let n = PendingNotification {
        notify_id: "test-id".into(),
        sender_pane_id: 0,
        source_context_id: 1,
        source_window_id: 1,
        title: "Test".into(),
        body: "body".into(),
        kind: crate::app_protocol::NotifyKind::Message,
        options: vec![],
        input_prompt: None,
        required: false,
        scope: crate::app_protocol::NotifyScope::Global,
        image_inline: None,
        image_pipe_id: None,
        response_file: Some("/tmp/resp".into()),
        timeout_secs: None,
        on_dismiss: None,
        enqueued_at: std::time::Instant::now(),
        tombstoned: false,
        deliver_after: None,
    };

    save_pending_notifications_to(&[n], &path);
    let restored = load_pending_notifications_from(&path);

    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0].notify_id, "test-id");
    assert_eq!(restored[0].title, "Test");
    assert!(
        restored[0].tombstoned,
        "restored notification must be tombstoned"
    );
    assert!(
        restored[0].response_file.is_none(),
        "response_file must be cleared on restore"
    );
    assert!(
        restored[0].image_pipe_id.is_none(),
        "image_pipe_id must be cleared on restore"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn persist_ttl_drops_old_notifications() {
    let dir = std::env::temp_dir().join(format!("plexi_test_ttl_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("notifications.json");

    let now_sys = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let eight_days_ago = now_sys.saturating_sub(8 * 24 * 3600);
    let json = format!(
        r#"[{{"notify_id":"old","sender_pane_id":0,"source_context_id":1,"title":"Old","body":"","kind":"message","options":[],"required":false,"scope":"global","enqueued_at_secs":{},"tombstoned":false}}]"#,
        eight_days_ago
    );
    std::fs::write(&path, json).unwrap();

    let restored = load_pending_notifications_from(&path);
    assert!(
        restored.is_empty(),
        "notification older than 7 days must be dropped"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Regression guard for #791: dispatch_notify_action_cmds with pane_focus
/// host_action must call pane_navigate synchronously before writing the
/// response file (so navigation is complete before the shell unblocks).
#[test]
fn dispatch_notify_action_pane_focus_navigates() {
    let mut h = HostHarness::new();
    let sender_id = h.add_test_pane();
    h.app.windows.push(crate::host::context::Window {
        name: "Context B".into(),
        path: std::env::temp_dir(),
        tree: {
            let mut tree = egui_tiles::Tree::empty("test_tree_2");
            let tile = tree.tiles.insert_pane(9903);
            tree.root = Some(tile);
            tree
        },
        panes: HashMap::new(),
        focused_pane: None,
        zoomed_pane: None,
        grid_x: 0,
        grid_y: 1,
        window_id: 2,
        context_id: 2,
    });
    h.app.router.push(crate::host::context::Context {
        name: "Context B".into(),
        root: std::env::temp_dir(),
        description: None,
        context_id: 2,
        parent_id: None,
        depth: 0,
        parked: false,
    });

    assert_eq!(h.app.active_window, 0);
    h.app
        .dispatch_notify_action_cmds(vec![AppCommand::DeliverNotifyAction {
            pane_id: sender_id,
            notify_id: "n1".into(),
            action_label: "Go".into(),
            value: None,
            response_file: None,
            host_action: Some("pane_focus:9903".into()),
        }]);
    assert_eq!(
        h.app.active_window, 1,
        "pane_focus host_action must navigate to the target window"
    );
}

/// #1774: A Window-scoped notification from a pane on window 0 must be visible
/// only when window 0 is active; navigating to window 1 hides it; returning
/// makes it visible again.
#[test]
fn window_scoped_notification_visible_only_on_source_window() {
    let mut h = HostHarness::new();
    let _pane_w0 = h.add_test_pane();

    // Add a second window in the same workspace so we can switch active_window.
    let win1_id = 2u64;
    h.app
        .windows
        .push(same_workspace_window_below(win1_id, 9920));

    let win0_id = h.app.windows[0].window_id;
    assert_eq!(h.app.active_window, 0);

    // Push a Window-scoped notification originating from window 0.
    h.app.pending_notifications.push(PendingNotification {
        notify_id: "win-scoped-1774".into(),
        sender_pane_id: 0,
        source_context_id: h.app.router.active().context_id,
        source_window_id: win0_id,
        title: "Window Notification".into(),
        body: "body".into(),
        kind: crate::app_protocol::NotifyKind::Message,
        options: vec![],
        input_prompt: None,
        required: false,
        scope: crate::app_protocol::NotifyScope::Window,
        image_inline: None,
        image_pipe_id: None,
        response_file: None,
        timeout_secs: None,
        on_dismiss: None,
        enqueued_at: std::time::Instant::now(),
        tombstoned: false,
        deliver_after: None,
    });

    // Visible on window 0.
    assert_eq!(h.app.active_window, 0);
    assert_eq!(
        h.app.visible_notification_count(),
        1,
        "notification must be visible on source window"
    );

    // Navigate to window 1 — notification must disappear.
    h.app.active_window = 1;
    assert_eq!(
        h.app.visible_notification_count(),
        0,
        "notification must be invisible on a different window"
    );

    // Return to window 0 — notification reappears.
    h.app.active_window = 0;
    assert_eq!(
        h.app.visible_notification_count(),
        1,
        "notification must reappear when returning to source window"
    );
}

// ── #1635: auto-dismiss when originating pane is focused ─────────────────────

/// #1635: non-required notification from a focused pane must be auto-dismissed.
#[test]
fn auto_dismiss_removes_non_required_notification_when_sender_focused() {
    let mut h = HostHarness::new();
    let pane_id = h.add_test_pane();

    // Focus the pane.
    h.app.pane_navigate(pane_id);

    let ctx_id = h.app.router.active().context_id;
    let win_id = h.app.windows[h.app.active_window].window_id;

    // Push a non-required notification from the focused pane.
    h.app.pending_notifications.push(PendingNotification {
        notify_id: "n-auto-dismiss".into(),
        sender_pane_id: pane_id,
        source_context_id: ctx_id,
        source_window_id: win_id,
        title: "Should go away".into(),
        body: "body".into(),
        kind: crate::app_protocol::NotifyKind::Message,
        options: vec![],
        input_prompt: None,
        required: false,
        scope: crate::app_protocol::NotifyScope::Global,
        image_inline: None,
        image_pipe_id: None,
        response_file: None,
        timeout_secs: None,
        on_dismiss: None,
        enqueued_at: std::time::Instant::now(),
        tombstoned: false,
        deliver_after: None,
    });
    h.app.show_notification_modal = true;
    h.app.current_notify_id = Some("n-auto-dismiss".into());

    assert_eq!(h.app.pending_notifications.len(), 1, "notification queued");

    h.app.auto_dismiss_sender_focused_notifications();

    assert!(
        h.app.pending_notifications.is_empty(),
        "non-required notification from focused pane must be auto-dismissed"
    );
    assert!(
        !h.app.show_notification_modal,
        "modal must close when queue empties via auto-dismiss"
    );
    assert!(
        h.app.current_notify_id.is_none(),
        "current_notify_id must be cleared after dismissal"
    );
}

/// #1635: required notifications must NOT be auto-dismissed even when sender is focused.
#[test]
fn auto_dismiss_spares_required_notifications() {
    let mut h = HostHarness::new();
    let pane_id = h.add_test_pane();

    h.app.pane_navigate(pane_id);

    let ctx_id = h.app.router.active().context_id;
    let win_id = h.app.windows[h.app.active_window].window_id;

    h.app.pending_notifications.push(PendingNotification {
        notify_id: "n-required".into(),
        sender_pane_id: pane_id,
        source_context_id: ctx_id,
        source_window_id: win_id,
        title: "Must stay".into(),
        body: "body".into(),
        kind: crate::app_protocol::NotifyKind::Message,
        options: vec![],
        input_prompt: None,
        required: true, // <-- required
        scope: crate::app_protocol::NotifyScope::Global,
        image_inline: None,
        image_pipe_id: None,
        response_file: None,
        timeout_secs: None,
        on_dismiss: None,
        enqueued_at: std::time::Instant::now(),
        tombstoned: false,
        deliver_after: None,
    });

    h.app.auto_dismiss_sender_focused_notifications();

    assert_eq!(
        h.app.pending_notifications.len(),
        1,
        "required notification must not be auto-dismissed"
    );
}

/// #1635: notification from a DIFFERENT pane (not the focused one) must be left alone.
#[test]
fn auto_dismiss_does_not_touch_other_pane_notifications() {
    let mut h = HostHarness::new();
    let sender_id = h.add_test_pane();
    let focused_id = h.add_test_pane();

    // Focus the second pane, NOT the sender.
    h.app.pane_navigate(focused_id);

    let ctx_id = h.app.router.active().context_id;
    let win_id = h.app.windows[h.app.active_window].window_id;

    h.app.pending_notifications.push(PendingNotification {
        notify_id: "n-other-pane".into(),
        sender_pane_id: sender_id, // different from focused_id
        source_context_id: ctx_id,
        source_window_id: win_id,
        title: "From other pane".into(),
        body: "body".into(),
        kind: crate::app_protocol::NotifyKind::Message,
        options: vec![],
        input_prompt: None,
        required: false,
        scope: crate::app_protocol::NotifyScope::Global,
        image_inline: None,
        image_pipe_id: None,
        response_file: None,
        timeout_secs: None,
        on_dismiss: None,
        enqueued_at: std::time::Instant::now(),
        tombstoned: false,
        deliver_after: None,
    });

    h.app.auto_dismiss_sender_focused_notifications();

    assert_eq!(
        h.app.pending_notifications.len(),
        1,
        "notification from a non-focused pane must not be auto-dismissed"
    );
}

// ── QuickNote preemption tests (#1626) ────────────────────────────────────

/// #1626: Cmd+0 with a NotificationModal on the focus stack must dismiss the
/// modal and push FocusKind::QuickNote to the top.
#[test]
fn quicknote_preempts_notification_modal() {
    let mut h = HostHarness::new();
    h.add_test_pane();

    // Simulate a NotificationModal being open.
    h.app.push_focus_layer(FocusKind::NotificationModal);
    assert_eq!(
        h.app.focus_stack.last(),
        Some(&FocusKind::NotificationModal),
        "NotificationModal must be on top before preemption"
    );
    assert!(
        h.app.is_quick_note_preemptable(),
        "NotificationModal must be preemptable"
    );

    h.app.dismiss_preemptable_modal();
    h.app.open_quick_note_modal();

    assert_eq!(
        h.app.focus_stack.last(),
        Some(&FocusKind::QuickNote),
        "QuickNote must be on top after preemption"
    );
}

/// #1626: Cmd+0 with CommandPalette on the focus stack must dismiss the
/// palette and push FocusKind::QuickNote to the top.
#[test]
fn quicknote_preempts_command_palette() {
    let mut h = HostHarness::new();
    h.add_test_pane();

    h.app.push_focus_layer(FocusKind::CommandPalette);
    assert!(
        h.app.is_quick_note_preemptable(),
        "CommandPalette must be preemptable"
    );

    h.app.dismiss_preemptable_modal();
    h.app.open_quick_note_modal();

    assert_eq!(
        h.app.focus_stack.last(),
        Some(&FocusKind::QuickNote),
        "QuickNote must be on top after preempting CommandPalette"
    );
}

/// #1626: ConfirmClose is critical and must NOT be preemptable by QuickNote.
#[test]
fn quicknote_does_not_preempt_confirm_close() {
    let mut h = HostHarness::new();
    h.add_test_pane();

    h.app.push_focus_layer(FocusKind::ConfirmClose);
    assert!(
        !h.app.is_quick_note_preemptable(),
        "ConfirmClose must NOT be preemptable"
    );
}

/// #1626: CapabilityModal is critical and must NOT be preemptable by QuickNote.
#[test]
fn quicknote_does_not_preempt_capability_modal() {
    let mut h = HostHarness::new();
    h.add_test_pane();

    h.app.push_focus_layer(FocusKind::CapabilityModal);
    assert!(
        !h.app.is_quick_note_preemptable(),
        "CapabilityModal must NOT be preemptable"
    );
}

/// #1626: ContextCloseConfirm is critical and must NOT be preemptable.
#[test]
fn quicknote_does_not_preempt_context_close_confirm() {
    let mut h = HostHarness::new();
    h.add_test_pane();

    h.app.push_focus_layer(FocusKind::ContextCloseConfirm);
    assert!(
        !h.app.is_quick_note_preemptable(),
        "ContextCloseConfirm must NOT be preemptable"
    );
}

// ── stint 0566/0569: the `enqueue_notification` choke point ─────────────────
//
// The audible cue is asserted on *intent* — `notification_cue_request` and the
// held `PlaybackSession` — never by driving an audio device. In `cfg(test)`,
// `crate::media::audio::start_playback` is the silent stub, so these tests are
// mute by construction; whether the cue is actually audible is a human check.

/// Build a minimal notification for choke-point tests.
fn cue_test_notification(id: &str) -> PendingNotification {
    PendingNotification {
        notify_id: id.into(),
        sender_pane_id: 0,
        source_context_id: 0,
        source_window_id: 0,
        title: "Title".into(),
        body: "Body".into(),
        kind: crate::app_protocol::NotifyKind::Message,
        options: vec![],
        input_prompt: None,
        required: false,
        scope: crate::app_protocol::NotifyScope::Global,
        image_inline: None,
        image_pipe_id: None,
        response_file: None,
        timeout_secs: None,
        on_dismiss: None,
        enqueued_at: std::time::Instant::now(),
        tombstoned: false,
        deliver_after: None,
    }
}

#[test]
fn notification_queue_orders_required_then_arrival() {
    let mut h = HostHarness::new();
    h.app.notifications_enabled = true;

    h.app.enqueue_notification(
        crate::app::notifications::NotifySource::Cli,
        cue_test_notification("first"),
    );

    let mut required = cue_test_notification("required");
    required.required = true;
    h.app
        .enqueue_notification(crate::app::notifications::NotifySource::Cli, required);

    h.app.enqueue_notification(
        crate::app::notifications::NotifySource::Cli,
        cue_test_notification("later"),
    );

    assert_eq!(
        h.app.sorted_notification_ids(),
        vec!["required", "first", "later"]
    );
}

/// 0566: the master switch is enforced inside the choke point, so the WASM and
/// host-internal surfaces — which never had a guard of their own — are now
/// gated too.
#[test]
fn enqueue_notification_honours_master_switch() {
    let mut h = HostHarness::new();
    h.app.notifications_enabled = false;

    for source in [
        crate::app::notifications::NotifySource::Cli,
        crate::app::notifications::NotifySource::App,
        crate::app::notifications::NotifySource::Wasm,
        crate::app::notifications::NotifySource::HostInternal,
    ] {
        let queued = h
            .app
            .enqueue_notification(source, cue_test_notification("dropped"));
        assert!(!queued, "{source:?} must be dropped while disabled");
    }

    assert!(
        h.app.pending_notifications.is_empty(),
        "no surface may bypass the master switch"
    );
    assert!(
        !h.app.show_notification_modal,
        "a dropped notification must not open the modal"
    );
}

/// 0566: with the switch on, every surface reaches the queue through the one
/// path that also persists, decides auto-open, and emits `NotificationPosted`.
#[test]
fn enqueue_notification_queues_when_enabled() {
    let mut h = HostHarness::new();
    h.app.notifications_enabled = true;

    let queued = h.app.enqueue_notification(
        crate::app::notifications::NotifySource::Wasm,
        cue_test_notification("wasm-1"),
    );

    assert!(queued, "enabled host must queue the notification");
    assert_eq!(h.app.pending_notifications.len(), 1);
    assert!(
        h.app.show_notification_modal,
        "a visible notification outside focus mode must auto-open"
    );
    assert_eq!(h.app.current_notify_id.as_deref(), Some("wasm-1"));
}

/// Every visible notification uses the same interruption decision.
#[test]
fn enqueue_notification_interrupts_when_enabled() {
    let mut h = HostHarness::new();
    h.app.notifications_enabled = true;

    h.app.enqueue_notification(
        crate::app::notifications::NotifySource::App,
        cue_test_notification("visible"),
    );

    assert_eq!(h.app.pending_notifications.len(), 1, "notification is queued");
    assert!(
        h.app.show_notification_modal,
        "visible notification must auto-open"
    );
    assert_eq!(h.app.current_notify_id.as_deref(), Some("visible"));
}

/// 0566: no `[notifications] sound` configured → no cue requested.
#[test]
fn notification_cue_silent_without_configured_sound() {
    let h = HostHarness::new();
    assert!(h.app.notifications_sound.is_none(), "unset by default");
    assert!(
        h.app
            .notification_cue_request(&cue_test_notification("no-sound"))
            .is_none(),
        "unset sound must request no cue"
    );
}

/// 0566: the cue carries the configured path and a sane volume. Asserting the
/// request — not playback — is what keeps this test silent.
#[test]
fn notification_cue_uses_configured_sound() {
    let mut h = HostHarness::new();
    h.app.notifications_sound = Some("/tmp/cue.wav".to_string());

    let request = h
        .app
        .notification_cue_request(&cue_test_notification("with-sound"))
        .expect("configured sound must request a cue");
    assert_eq!(request.source, "/tmp/cue.wav");
    assert_eq!(request.volume, 1.0);
}

/// 0566: focus mode means nothing interrupts, and a sound is the most
/// interrupting thing there is.
#[test]
fn notification_cue_suppressed_in_focus_mode() {
    let mut h = HostHarness::new();
    h.app.notifications_sound = Some("/tmp/cue.wav".to_string());
    h.app.notifications_focus_mode = true;

    assert!(
        h.app
            .notification_cue_request(&cue_test_notification("focused"))
            .is_none(),
        "focus mode must suppress the cue"
    );
}

/// Fix round 1 blocker 1: a notification the user cannot see must not make a
/// sound. Context-scoped with a source context that is not active → invisible
/// → silent with a sound configured. Driven through
/// the real enqueue path; the cue decision is asserted, never played.
#[test]
fn cue_suppressed_for_invisible_notification() {
    let mut h = HostHarness::new();
    h.app.notifications_enabled = true;
    h.app.notifications_sound = Some("/tmp/cue.wav".to_string());

    let mut n = cue_test_notification("invisible");
    n.scope = crate::app_protocol::NotifyScope::Context;
    n.source_context_id = 999_999; // not the active context
    assert!(
        !h.app.notification_is_visible(&n),
        "precondition: the notification must not be visible"
    );

    let queued = h
        .app
        .enqueue_notification(crate::app::notifications::NotifySource::Cli, n);

    assert!(queued, "still queued — the badge ticks");
    assert!(
        h.app.notification_cue_playback.is_none(),
        "an invisible notification must not start a cue"
    );
    assert!(
        !h.app.show_notification_modal,
        "an invisible notification must not open the modal"
    );
}

/// A plain CLI notification is visible, opens the modal, and starts its cue
/// when sound is configured. Focus mode shares the same gate and mutes both.
#[test]
fn plain_cli_notify_interrupts_and_focus_mode_mutes() {
    let mut h = HostHarness::new();
    h.app.notifications_enabled = true;
    h.app.notifications_sound = Some("/tmp/cue.wav".to_string());

    h.inject_ipc(cli_notify_request(None, None, None));
    h.run_frames(2);

    assert!(h.app.show_notification_modal, "plain CLI notify interrupts");
    assert!(
        h.app.notification_cue_playback.is_some(),
        "plain CLI notify starts the configured cue"
    );

    let mut focused = HostHarness::new();
    focused.app.notifications_enabled = true;
    focused.app.notifications_sound = Some("/tmp/cue.wav".to_string());
    focused.app.notifications_focus_mode = true;
    focused.inject_ipc(cli_notify_request(None, None, None));
    focused.run_frames(2);
    assert!(!focused.app.show_notification_modal, "focus mode prevents interrupt");
    assert!(focused.app.notification_cue_playback.is_none(), "focus mode mutes cue");
}

/// 0566: the session is held on the app — a local binding would drop at end of
/// scope and cut playback off.
#[test]
fn enqueue_notification_holds_cue_session() {
    let mut h = HostHarness::new();
    h.app.notifications_enabled = true;
    h.app.notifications_sound = Some("/tmp/cue.wav".to_string());
    assert!(h.app.notification_cue_playback.is_none(), "silent at start");

    h.app.enqueue_notification(
        crate::app::notifications::NotifySource::Cli,
        cue_test_notification("with-cue"),
    );

    assert!(
        h.app.notification_cue_playback.is_some(),
        "the cue session must outlive enqueue_notification"
    );
}

/// 0566: a notification dropped by the master switch never reaches the cue.
#[test]
fn disabled_notifications_never_play_a_cue() {
    let mut h = HostHarness::new();
    h.app.notifications_enabled = false;
    h.app.notifications_sound = Some("/tmp/cue.wav".to_string());

    h.app.enqueue_notification(
        crate::app::notifications::NotifySource::Cli,
        cue_test_notification("dropped"),
    );

    assert!(
        h.app.notification_cue_playback.is_none(),
        "the enabled gate must run before the cue"
    );
}

/// Build the `AppRequest::Notify` a `plexi notify` invocation produces.
fn cli_notify_request(
    scope: Option<crate::app_protocol::NotifyScope>,
    source_context_id: Option<u64>,
    source_pane_id: Option<u64>,
) -> crate::app_protocol::AppRequest {
    crate::app_protocol::AppRequest::Notify {
        title: "CLI notify".into(),
        body: "body".into(),
        kind: crate::app_protocol::NotifyKind::Message,
        options: vec![],
        input_prompt: None,
        required: false,
        actions: vec![],
        notify_id: None,
        image_inline: None,
        image_pipe_id: None,
        timeout_secs: None,
        on_dismiss: None,
        response_file: None,
        scope,
        source_context_id,
        source_pane_id,
    }
}

/// 0608: a parked app has no sender window. Window-scoped delivery must stay
/// in its stamped context rather than inheriting whatever window is active at
/// dispatch.
#[test]
fn parked_app_window_notification_narrows_without_active_window_provenance() {
    let mut h = HostHarness::new();
    h.app.notifications_enabled = true;
    let context_id = h.app.router.active().context_id;
    let (scope, source_window_id) = h
        .app
        .resolve_app_notification_provenance(0, crate::app_protocol::NotifyScope::Window);
    h.app.enqueue_notification(
        crate::app::notifications::NotifySource::App,
        PendingNotification {
            notify_id: "parked-window-scope".into(),
            sender_pane_id: 0,
            source_context_id: context_id,
            source_window_id,
            title: "Parked".into(),
            body: "body".into(),
            kind: crate::app_protocol::NotifyKind::Message,
            options: vec![],
            input_prompt: None,
            required: false,
            scope,
            image_inline: None,
            image_pipe_id: None,
            response_file: None,
            timeout_secs: None,
            on_dismiss: None,
            enqueued_at: std::time::Instant::now(),
            tombstoned: false,
            deliver_after: None,
        },
    );

    let notification = &h.app.pending_notifications[0];
    assert_eq!(
        notification.scope,
        crate::app_protocol::NotifyScope::Context
    );
    assert_eq!(notification.source_window_id, 0);
    assert_eq!(notification.source_context_id, context_id);
}

/// 0610/0611: the CLI's display timeout reaches the host payload, while a
/// sticky CLI notification can be removed only by its posting pane.
#[test]
fn cli_notification_timeout_and_sender_scoped_dismissal() {
    let mut h = HostHarness::new();
    h.app.notifications_enabled = true;
    let owner_pane_id = h.add_test_pane();
    let foreign_pane_id = h.add_test_pane();
    let context_id = h.app.router.active().context_id;
    let response_dir = tempfile::tempdir().expect("tempdir");
    let response_file = response_dir.path().join("dismiss-response");

    h.inject_ipc(crate::app_protocol::AppRequest::Notify {
        title: "sticky".into(),
        body: "body".into(),
        kind: crate::app_protocol::NotifyKind::Message,
        options: vec![],
        input_prompt: None,
        required: false,
        actions: vec![],
        notify_id: Some(format!("cli:{owner_pane_id}:sticky")),
        image_inline: None,
        image_pipe_id: None,
        timeout_secs: Some(30),
        on_dismiss: None,
        response_file: None,
        scope: None,
        source_context_id: Some(context_id),
        source_pane_id: Some(owner_pane_id),
    });
    h.run_frames(2);

    let posted = &h.app.pending_notifications[0];
    assert_eq!(posted.notify_id, format!("cli:{owner_pane_id}:sticky"));
    assert_eq!(posted.timeout_secs, Some(30));
    assert_eq!(
        posted.sender_pane_id, 0,
        "CLI notifications must not auto-dismiss with their focused terminal"
    );

    h.inject_ipc(crate::app_protocol::AppRequest::DismissNotification {
        notify_id: format!("cli:{owner_pane_id}:sticky"),
        source_context_id: Some(context_id),
        source_pane_id: Some(foreign_pane_id),
        response_file: response_file.to_string_lossy().into_owned(),
    });
    h.run_frames(2);
    assert_eq!(
        h.app.pending_notifications.len(),
        1,
        "foreign pane must not dismiss"
    );

    h.inject_ipc(crate::app_protocol::AppRequest::DismissNotification {
        notify_id: format!("cli:{owner_pane_id}:sticky"),
        source_context_id: Some(context_id),
        source_pane_id: Some(owner_pane_id),
        response_file: response_file.to_string_lossy().into_owned(),
    });
    h.run_frames(2);
    assert!(
        h.app.pending_notifications.is_empty(),
        "owner dismissal removes sticky notification"
    );
}

/// 0569: an unscoped `plexi notify` from inside a pane is context-local, not
/// global. Drives the real CLI surface end to end through pane_ipc, carrying
/// the caller identity the CLI stamps from `PLEXI_CONTEXT_ID`.
#[test]
fn cli_notify_without_scope_defaults_to_context() {
    let mut h = HostHarness::new();
    h.app.notifications_enabled = true;
    let caller_ctx = h.app.router.active().context_id;

    h.inject_ipc(cli_notify_request(None, Some(caller_ctx), None));
    h.run_frames(2);

    assert_eq!(h.app.pending_notifications.len(), 1, "notification queued");
    assert_eq!(
        h.app.pending_notifications[0].scope,
        crate::app_protocol::NotifyScope::Context,
        "unscoped CLI notify must stay in its own context, not leak globally"
    );
    assert_eq!(
        h.app.pending_notifications[0].source_context_id, caller_ctx,
        "provenance is the caller's claimed context"
    );
}

/// 0569: `plexi notify --scope global` is unchanged by the new default.
#[test]
fn cli_notify_explicit_global_scope_survives_the_new_default() {
    let mut h = HostHarness::new();
    h.app.notifications_enabled = true;

    h.inject_ipc(cli_notify_request(
        Some(crate::app_protocol::NotifyScope::Global),
        None,
        None,
    ));
    h.run_frames(2);

    assert_eq!(h.app.pending_notifications.len(), 1);
    assert_eq!(
        h.app.pending_notifications[0].scope,
        crate::app_protocol::NotifyScope::Global,
        "--scope global must still be global"
    );
}

/// Fix round 1 blocker 2: the notification attaches to the context the caller
/// named, never to whichever context happens to be active at dispatch time.
/// An agent notifying from a background context must not have its notification
/// stamped with — and hidden inside — a stranger's context.
#[test]
fn cli_notify_attaches_to_callers_context_not_active() {
    let mut h = HostHarness::new();
    h.app.notifications_enabled = true;
    let parent_id = h.app.router.active().context_id;
    let parent_name = h.app.router.active().name.clone();

    h.app
        .new_child_context(crate::pane_ops::ChildContextSpec::single_terminal(
            Some(parent_id),
            parent_name,
            std::env::temp_dir(),
            true,
            false,
            None,
        ))
        .expect("child context");
    let child_id = h
        .app
        .router
        .iter()
        .map(|c| c.context_id)
        .find(|id| *id != parent_id)
        .expect("child context id");

    // Whichever of the two contexts is NOT active plays the background caller.
    let active_id = h.app.router.active().context_id;
    let caller_id = if active_id == parent_id {
        child_id
    } else {
        parent_id
    };
    assert_ne!(
        caller_id, active_id,
        "caller must not be the active context"
    );

    h.inject_ipc(cli_notify_request(None, Some(caller_id), None));
    h.run_frames(2);

    assert_eq!(h.app.pending_notifications.len(), 1);
    let n = &h.app.pending_notifications[0];
    assert_eq!(n.scope, crate::app_protocol::NotifyScope::Context);
    assert_eq!(
        n.source_context_id, caller_id,
        "provenance must be the caller's own context, not dispatch-time active state"
    );
}

/// Fix round 1 blocker 2: an identity naming no live context is never
/// reattributed to some other context — the notification escalates to global
/// so it stays reachable everywhere instead of hiding in a fabricated home.
#[test]
fn cli_notify_stale_identity_escalates_to_global() {
    let mut h = HostHarness::new();
    h.app.notifications_enabled = true;

    h.inject_ipc(cli_notify_request(None, Some(999_999), Some(888_888)));
    h.run_frames(2);

    assert_eq!(h.app.pending_notifications.len(), 1);
    let n = &h.app.pending_notifications[0];
    assert_eq!(
        n.scope,
        crate::app_protocol::NotifyScope::Global,
        "a stale identity must escalate to global, not attach elsewhere"
    );
    assert_eq!(n.source_context_id, 0, "0 = no context, the host sentinel");
}

/// Fix round 1 blocker 2 (Codex finding): `--scope window` from a context
/// that resolved but has no live window narrows to context scope — the least
/// widening that keeps the notification reachable — never all the way to
/// global.
#[test]
fn cli_notify_window_scope_without_live_window_narrows_to_context() {
    let mut h = HostHarness::new();
    h.app.notifications_enabled = true;
    // A live context with no window (parked): in the router, not in windows.
    h.app.router.push(crate::host::context::Context {
        name: "Parked".into(),
        root: std::env::temp_dir(),
        description: None,
        context_id: 777,
        parent_id: None,
        depth: 0,
        parked: true,
    });

    h.inject_ipc(cli_notify_request(
        Some(crate::app_protocol::NotifyScope::Window),
        Some(777),
        None,
    ));
    h.run_frames(2);

    assert_eq!(h.app.pending_notifications.len(), 1);
    let n = &h.app.pending_notifications[0];
    assert_eq!(
        n.scope,
        crate::app_protocol::NotifyScope::Context,
        "window scope with no live window must narrow to context, not widen to global"
    );
    assert_eq!(n.source_context_id, 777);
}

/// Fix round 1 blocker 2: a caller outside any pane (PLEXI_SOCKET exported by
/// hand, no PLEXI_CONTEXT_ID) has no home context — its unscoped notification
/// is global, matching what such callers always got.
#[test]
fn cli_notify_outside_pane_unscoped_escalates_to_global() {
    let mut h = HostHarness::new();
    h.app.notifications_enabled = true;

    h.inject_ipc(cli_notify_request(None, None, None));
    h.run_frames(2);

    assert_eq!(h.app.pending_notifications.len(), 1);
    assert_eq!(
        h.app.pending_notifications[0].scope,
        crate::app_protocol::NotifyScope::Global,
        "no identity → no context to attach to → global"
    );
}

/// 0569: the shared default every unscoped surface resolves to.
#[test]
fn notify_scope_default_is_context() {
    assert_eq!(
        crate::app_protocol::NotifyScope::default(),
        crate::app_protocol::NotifyScope::Context
    );
    let unset: Option<crate::app_protocol::NotifyScope> = None;
    assert_eq!(
        unset.unwrap_or_default(),
        crate::app_protocol::NotifyScope::Context
    );
}

/// 0569: `--scope global` must map to an explicit `Some(Global)` on the wire.
/// Sending `None` and leaning on the host fallback silently reinterpreted the
/// flag the moment the default changed — that was the actual regression.
#[test]
fn parse_notify_scope_maps_every_named_scope_explicitly() {
    use crate::app_protocol::NotifyScope;
    assert_eq!(crate::cli::parse_notify_scope(None), Ok(None));
    assert_eq!(
        crate::cli::parse_notify_scope(Some("global")),
        Ok(Some(NotifyScope::Global))
    );
    assert_eq!(
        crate::cli::parse_notify_scope(Some("context")),
        Ok(Some(NotifyScope::Context))
    );
    assert_eq!(
        crate::cli::parse_notify_scope(Some("window")),
        Ok(Some(NotifyScope::Window))
    );
    assert!(crate::cli::parse_notify_scope(Some("nope")).is_err());
}
