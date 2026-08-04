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
        dismiss_owner_pane_id: 0,
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
        origin_in_view: false,
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
        dismiss_owner_pane_id: 0,
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
        origin_in_view: false,
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
        dismiss_owner_pane_id: 0,
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
        origin_in_view: false,
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
        dismiss_owner_pane_id: 0,
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
        origin_in_view: false,
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
        dismiss_owner_pane_id: 0,
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
        origin_in_view: false,
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
        dismiss_owner_pane_id: 0,
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
        origin_in_view: false,
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
        dismiss_owner_pane_id: 0,
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
        origin_in_view: false,
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
        dismiss_owner_pane_id: 0,
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
        origin_in_view: false,
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

    h.inject_ipc(cli_notify_request(None, None, None, None));
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
    focused.inject_ipc(cli_notify_request(None, None, None, None));
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
/// `peer_pid` stands in for the host-established socket-peer identity
/// (`None` reproduces an unresolvable/outside-pane caller). Internally
/// expanded to the single-hop ancestry `resolve_socket_peer_pane` expects —
/// callers pass the pid they want matched directly (typically a live pane's
/// own `child_pid`), same as the pre-0636-fix single-pid API.
fn cli_notify_request(
    scope: Option<crate::app_protocol::NotifyScope>,
    source_context_id: Option<u64>,
    source_pane_id: Option<u64>,
    peer_pid: Option<u32>,
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
        peer_pid: peer_pid.map(|pid| vec![pid]),
    }
}

/// Create a real PTY-backed terminal pane at a fresh grid slot so its
/// shell's `child_pid` can stand in for a genuine `plexi notify` socket
/// peer. Returns `None` when the environment has no PTY (headless CI) —
/// every caller must treat that as "skip this test", matching the
/// `pty_available` pattern used elsewhere in this test suite.
fn spawn_real_terminal_pane(
    h: &mut HostHarness,
    grid_x: u32,
    grid_y: u32,
    context_id: u64,
) -> Option<(crate::spatial::tiling::PaneId, u32, u64, u64)> {
    let before = h.app.windows.len();
    h.app.create_page_at(grid_x, grid_y, context_id, None, false, None);
    if h.app.windows.len() == before {
        return None; // no PTY in this environment
    }
    let win_idx = h.app.windows.len() - 1;
    let pane_id = *h.app.windows[win_idx].panes.keys().next()?;
    let child_pid = h.app.windows[win_idx]
        .panes
        .get(&pane_id)?
        .as_terminal()?
        .backend
        .child_pid();
    let ctx_id = h.app.windows[win_idx].context_id;
    let window_id = h.app.windows[win_idx].window_id;
    Some((pane_id, child_pid, ctx_id, window_id))
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
            dismiss_owner_pane_id: 0,
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
            origin_in_view: false,
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

/// 0610/0611, re-verified after 0636: the CLI's display timeout reaches the
/// host payload, while a sticky CLI notification can be removed only by its
/// posting pane — ownership now decided from the real socket-peer identity
/// (`peer_pid`), not the client-claimed `source_pane_id` this test used to
/// drive directly.
#[test]
fn cli_notification_timeout_and_sender_scoped_dismissal() {
    let mut h = HostHarness::new();
    h.app.notifications_enabled = true;
    let Some((owner_pane_id, owner_peer_pid, owner_ctx, _)) =
        spawn_real_terminal_pane(&mut h, 92, 92, 503)
    else {
        return; // no PTY in this environment
    };
    let Some((foreign_pane_id, foreign_peer_pid, _, _)) =
        spawn_real_terminal_pane(&mut h, 93, 93, 504)
    else {
        return; // no PTY in this environment
    };
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
        source_context_id: Some(owner_ctx),
        source_pane_id: Some(owner_pane_id),
        peer_pid: Some(vec![owner_peer_pid]),
    });
    h.run_frames(2);

    let posted = &h.app.pending_notifications[0];
    assert_eq!(posted.notify_id, format!("cli:{owner_pane_id}:sticky"));
    assert_eq!(posted.timeout_secs, Some(30));
    assert_eq!(
        posted.sender_pane_id, 0,
        "CLI notifications must not auto-dismiss with their focused terminal"
    );
    assert_eq!(
        posted.dismiss_owner_pane_id, owner_pane_id,
        "dismiss ownership must be stamped from the resolved socket peer"
    );

    // A different real pane (its own genuine socket peer) claims to be the
    // owner via the wire fields — those claims must be ignored; only the
    // resolved `peer_pid` decides ownership, and it names a stranger.
    h.inject_ipc(crate::app_protocol::AppRequest::DismissNotification {
        notify_id: format!("cli:{owner_pane_id}:sticky"),
        source_context_id: Some(owner_ctx),
        source_pane_id: Some(owner_pane_id), // forged claim
        response_file: response_file.to_string_lossy().into_owned(),
        peer_pid: Some(vec![foreign_peer_pid]), // true peer disagrees
    });
    h.run_frames(2);
    assert_eq!(
        h.app.pending_notifications.len(),
        1,
        "foreign pane must not dismiss, even while claiming to be the owner"
    );

    h.inject_ipc(crate::app_protocol::AppRequest::DismissNotification {
        notify_id: format!("cli:{owner_pane_id}:sticky"),
        source_context_id: Some(owner_ctx),
        source_pane_id: Some(foreign_pane_id), // forged claim, must be ignored
        response_file: response_file.to_string_lossy().into_owned(),
        peer_pid: Some(vec![owner_peer_pid]), // true peer is the real owner
    });
    h.run_frames(2);
    assert!(
        h.app.pending_notifications.is_empty(),
        "owner dismissal removes sticky notification via the resolved peer"
    );
}

/// 0636: a socket peer that resolves to no live pane (outside-pane caller,
/// or an already-closed pane) must not fall back to trusting the client's
/// claimed identity — Notify escalates to Global and Dismiss is rejected,
/// exactly like an absent identity.
#[test]
fn unresolvable_peer_escalates_notify_and_blocks_dismiss() {
    let mut h = HostHarness::new();
    h.app.notifications_enabled = true;
    let real_context_id = h.app.router.active().context_id;

    // No live terminal pane in this harness has this (or any) pid as an
    // ancestor, so any peer_pid — including our own process — resolves to
    // nothing.
    let unresolvable_peer = std::process::id();

    h.inject_ipc(crate::app_protocol::AppRequest::Notify {
        title: "t".into(),
        body: "b".into(),
        kind: crate::app_protocol::NotifyKind::Message,
        options: vec![],
        input_prompt: None,
        required: false,
        actions: vec![],
        notify_id: Some("unresolvable-peer".into()),
        image_inline: None,
        image_pipe_id: None,
        timeout_secs: None,
        on_dismiss: None,
        response_file: None,
        scope: Some(crate::app_protocol::NotifyScope::Context),
        source_context_id: Some(real_context_id),
        source_pane_id: None,
        peer_pid: Some(vec![unresolvable_peer]),
    });
    h.run_frames(2);

    let posted = &h.app.pending_notifications[0];
    assert_eq!(
        posted.scope,
        crate::app_protocol::NotifyScope::Global,
        "an unresolvable peer must escalate to Global rather than trust the claimed context"
    );
    assert_eq!(
        posted.dismiss_owner_pane_id, 0,
        "an unresolvable peer must not stamp any pane as the dismiss owner"
    );

    let response_dir = tempfile::tempdir().expect("tempdir");
    let response_file = response_dir.path().join("dismiss-response");
    h.inject_ipc(crate::app_protocol::AppRequest::DismissNotification {
        notify_id: "unresolvable-peer".into(),
        source_context_id: Some(real_context_id),
        source_pane_id: Some(424_242), // forged, irrelevant
        response_file: response_file.to_string_lossy().into_owned(),
        peer_pid: Some(vec![unresolvable_peer]),
    });
    h.run_frames(2);
    assert_eq!(
        h.app.pending_notifications.len(),
        1,
        "dismiss with no resolvable sender must not remove the notification"
    );
}

/// 0636: a forged `source_pane_id`/`source_context_id` naming a REAL pane
/// must be ignored whenever it disagrees with the host-resolved socket peer
/// — the notification is attributed to the true peer, never the claim.
#[test]
fn forged_source_pane_is_ignored_in_favor_of_resolved_peer() {
    let mut h = HostHarness::new();
    h.app.notifications_enabled = true;

    let Some((_, real_peer_pid, real_ctx, _)) = spawn_real_terminal_pane(&mut h, 94, 94, 505)
    else {
        return; // no PTY in this environment
    };
    let Some((forged_pane_id, _, forged_ctx, _)) = spawn_real_terminal_pane(&mut h, 95, 95, 506)
    else {
        return; // no PTY in this environment
    };
    assert_ne!(real_ctx, forged_ctx);

    h.inject_ipc(crate::app_protocol::AppRequest::Notify {
        title: "t".into(),
        body: "b".into(),
        kind: crate::app_protocol::NotifyKind::Message,
        options: vec![],
        input_prompt: None,
        required: false,
        actions: vec![],
        notify_id: Some("forged-claim".into()),
        image_inline: None,
        image_pipe_id: None,
        timeout_secs: None,
        on_dismiss: None,
        response_file: None,
        scope: Some(crate::app_protocol::NotifyScope::Context),
        source_context_id: Some(forged_ctx),
        source_pane_id: Some(forged_pane_id),
        peer_pid: Some(vec![real_peer_pid]), // the true sender is the OTHER pane
    });
    h.run_frames(2);

    let posted = &h.app.pending_notifications[0];
    assert_eq!(
        posted.source_context_id, real_ctx,
        "must attribute to the host-resolved peer, not the claimed context"
    );
    assert_ne!(posted.source_context_id, forged_ctx);
}

/// 0569: an unscoped `plexi notify` from inside a pane is context-local, not
/// global. Drives the real CLI surface end to end through pane_ipc, with the
/// caller identity coming from a real socket-peer resolution (0636) rather
/// than a claimed `source_context_id`.
#[test]
fn cli_notify_without_scope_defaults_to_context() {
    let mut h = HostHarness::new();
    h.app.notifications_enabled = true;
    let Some((_, peer_pid, real_ctx, _)) = spawn_real_terminal_pane(&mut h, 90, 90, 501) else {
        return; // no PTY in this environment
    };

    h.inject_ipc(cli_notify_request(None, None, None, Some(peer_pid)));
    h.run_frames(2);

    assert_eq!(h.app.pending_notifications.len(), 1, "notification queued");
    assert_eq!(
        h.app.pending_notifications[0].scope,
        crate::app_protocol::NotifyScope::Context,
        "unscoped CLI notify must stay in its own context, not leak globally"
    );
    assert_eq!(
        h.app.pending_notifications[0].source_context_id, real_ctx,
        "provenance is the host-resolved socket-peer context"
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

/// Fix round 1 blocker 2, re-verified after 0636: the notification attaches
/// to the socket peer's real context, never to whichever context happens to
/// be active at dispatch time. An agent notifying from a background context
/// must not have its notification stamped with — and hidden inside — a
/// stranger's context.
#[test]
fn cli_notify_attaches_to_callers_context_not_active() {
    let mut h = HostHarness::new();
    h.app.notifications_enabled = true;
    let active_before = h.app.router.active().context_id;

    // The real sender pane lives in a fresh window/context; the harness's
    // originally-active context stays active throughout — the caller is
    // never the active context, so any attribution to `active_before` would
    // prove dispatch-time active state leaked in.
    let Some((_, peer_pid, real_ctx, _)) = spawn_real_terminal_pane(&mut h, 91, 91, 502) else {
        return; // no PTY in this environment
    };
    assert_ne!(real_ctx, active_before);

    h.inject_ipc(cli_notify_request(None, None, None, Some(peer_pid)));
    h.run_frames(2);

    assert_eq!(h.app.pending_notifications.len(), 1);
    let n = &h.app.pending_notifications[0];
    assert_eq!(n.scope, crate::app_protocol::NotifyScope::Context);
    assert_eq!(
        n.source_context_id, real_ctx,
        "provenance must be the socket peer's own context, not dispatch-time active state"
    );
}

/// Fix round 1 blocker 2, re-verified after 0636: an identity naming no
/// resolvable socket peer is never reattributed to some other context — the
/// notification escalates to global so it stays reachable everywhere instead
/// of hiding in a fabricated home. `source_context_id`/`source_pane_id` are
/// forged here to prove they are ignored entirely absent a host-resolved
/// peer, not merely absent themselves.
#[test]
fn cli_notify_stale_identity_escalates_to_global() {
    let mut h = HostHarness::new();
    h.app.notifications_enabled = true;

    h.inject_ipc(cli_notify_request(
        None,
        Some(999_999),
        Some(888_888),
        None, // no resolvable socket peer
    ));
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

/// Fix round 1 blocker 2: a caller outside any pane (PLEXI_SOCKET exported by
/// hand, no PLEXI_CONTEXT_ID) has no home context — its unscoped notification
/// is global, matching what such callers always got.
#[test]
fn cli_notify_outside_pane_unscoped_escalates_to_global() {
    let mut h = HostHarness::new();
    h.app.notifications_enabled = true;

    h.inject_ipc(cli_notify_request(None, None, None, None));
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

/// 0636 live regression: a genuine child process spawned as a real
/// descendant of a HostHarness terminal pane's shell, connected to a real
/// `UnixListener` (not `UnixStream::pair`, which only proves in-process
/// wiring), sends a notify line with a DIFFERENT pane forged into
/// `source_pane_id`/`source_context_id`. The host must attribute the
/// notification to the true shell-descendant pane, never the forged claim.
/// macOS-only: relies on `nc -U` (BSD netcat, always present on the
/// project's dev/CI platform — see root `AGENTS.md`); the ancestor walk
/// itself (`resolve_socket_peer_pane`) has an independent Linux path
/// exercised indirectly by every other test in this module.
#[cfg(target_os = "macos")]
#[test]
fn live_socket_peer_resolves_through_real_descendant_process() {
    let mut h = HostHarness::new();
    h.app.notifications_enabled = true;

    let Some((real_pane_id, _real_shell_pid, real_ctx, _)) =
        spawn_real_terminal_pane(&mut h, 96, 96, 507)
    else {
        return; // no PTY in this environment
    };
    let real_win_idx = h.app.windows.len() - 1;

    let Some((forged_pane_id, _, forged_ctx, _)) = spawn_real_terminal_pane(&mut h, 97, 97, 508)
    else {
        return; // no PTY in this environment
    };
    assert_ne!(real_ctx, forged_ctx);

    // A real kernel-backed AF_UNIX listener — not `UnixStream::pair`, which
    // only proves the in-process handler wiring, never a genuine peer
    // credential lookup across a real accept().
    let dir = tempfile::tempdir().expect("tempdir");
    let sock_path = dir.path().join("live-notify.sock");
    let listener = std::os::unix::net::UnixListener::bind(&sock_path).expect("bind");

    let mailbox = h.ipc_tx.clone();
    let wake = std::sync::Arc::new(crate::app::ui_mailbox::EguiWake::new(
        egui::Context::default(),
    ));
    let (subscribe_mailbox, _sub_rx) = crate::app::ui_mailbox::UiMailbox::<
        crate::host::event_subscriptions::HostSubscribeRequest,
    >::channel(wake.clone(), "event_subscribe");
    let (publish_mailbox, _pub_rx) = crate::app::ui_mailbox::UiMailbox::<
        crate::host::event_subscriptions::HostPublishRequest,
    >::channel(wake, "event_publish");

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            // Mirror `spawn_socket_listener`'s production accept loop:
            // resolve the peer identity synchronously, right here, before
            // any further work — never inside a spawned connection thread.
            let peer_pid = resolve_socket_peer_pid(&stream);
            let peer_ancestry = peer_pid.map(capture_peer_ancestry);
            handle_socket_connection(
                stream,
                mailbox.clone(),
                subscribe_mailbox.clone(),
                publish_mailbox.clone(),
                peer_ancestry,
            );
        }
    });

    // JSON payload forging `source_pane_id`/`source_context_id` as the
    // OTHER (forged) pane. Sent by a real `nc` process spawned INSIDE the
    // real pane's own shell — a genuine child of its `child_pid`, not a
    // pid we merely compute.
    let payload = serde_json::json!({
        "type": "notify",
        "notify_id": "live-descendant",
        "title": "t",
        "body": "b",
        "kind": "message",
        "options": [],
        "scope": "context",
        "source_context_id": forged_ctx,
        "source_pane_id": forged_pane_id,
    });
    let payload_path = dir.path().join("payload.json");
    std::fs::write(&payload_path, format!("{payload}\n")).expect("write payload");

    let quote = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
    // Absolute path, not bare `nc`: a dev machine with Homebrew's GNU
    // netcat ahead of /usr/bin on PATH silently runs the wrong `nc` (GNU
    // netcat rejects `-U` with "invalid option"), which would hang this
    // test on the 10s deadline below instead of proving anything.
    let cmd = format!(
        "cat {} | /usr/bin/nc -U {}\n",
        quote(payload_path.to_str().expect("utf8 path")),
        quote(sock_path.to_str().expect("utf8 path")),
    );
    {
        let win = &mut h.app.windows[real_win_idx];
        let pane = win.panes.get_mut(&real_pane_id).expect("real pane exists");
        let terminal = pane.as_terminal_mut().expect("terminal pane");
        terminal
            .backend
            .process_command(egui_term::BackendCommand::Write(cmd.into_bytes()));
    }

    // The shell executes `nc` as a genuine OS process on its own schedule,
    // independent of our frame pump — poll for delivery instead of assuming
    // synchronous completion.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        h.run_frames(1);
        if !h.app.pending_notifications.is_empty() {
            break;
        }
        if std::time::Instant::now() >= deadline {
            panic!(
                "live descendant notify did not arrive within 10s — check `nc -U` availability"
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    let posted = &h.app.pending_notifications[0];
    assert_eq!(
        posted.source_context_id, real_ctx,
        "must attribute to the real shell-descendant pane, not the forged claim"
    );
    assert_ne!(posted.source_context_id, forged_ctx);
}

// ── stint 0727: resurface a notification when its origin pane's window
// comes into view ───────────────────────────────────────────────────────────
//
// "In view" is a layout predicate (pane present in the currently-displayed
// context of the currently-focused window), never a render/focus/paint
// predicate. `origin_in_view` on `PendingNotification` doubles as the policy
// state: a resurface only fires on a rising edge (out-of-view → in-view), so
// an explicit dismissal while the pane stays in view is final, and it only
// re-arms once the pane leaves view and returns.

/// Build a Global-scope notification for resurface tests — scope stays out
/// of the way so only `pane_is_in_view` gates visibility/interruption.
fn resurface_test_notification(id: &str, sender_pane_id: u64) -> PendingNotification {
    PendingNotification {
        notify_id: id.into(),
        sender_pane_id,
        dismiss_owner_pane_id: 0,
        source_context_id: 0,
        source_window_id: 0,
        title: "Resurface me".into(),
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
        origin_in_view: false,
    }
}

/// Push a second context with its own single-pane window, mirroring the
/// pattern in `dispatch_notify_action_pane_focus_navigates`. Returns the new
/// window's index.
fn push_second_context_window(h: &mut HostHarness, ctx_id: u64, window_id: u64) -> usize {
    h.app.windows.push(Window {
        name: "Context B".into(),
        path: std::env::temp_dir(),
        tree: {
            let mut tree = egui_tiles::Tree::empty("resurface_test_tree");
            let tile = tree.tiles.insert_pane(9_000_000 + window_id);
            tree.root = Some(tile);
            tree
        },
        panes: HashMap::new(),
        focused_pane: None,
        zoomed_pane: None,
        grid_x: 0,
        grid_y: 1,
        window_id,
        context_id: ctx_id,
    });
    h.app.router.push(crate::host::context::Context {
        name: "Context B".into(),
        root: std::env::temp_dir(),
        description: None,
        context_id: ctx_id,
        parent_id: None,
        depth: 0,
        parked: false,
    });
    h.app.windows.len() - 1
}

/// #727: a notification queued while its origin pane is out of view does not
/// resurface until the pane's window actually comes into view — then it
/// opens the modal exactly once for that navigation.
#[test]
fn resurfaces_when_origin_pane_window_comes_into_view() {
    let mut h = HostHarness::new();
    h.app.notifications_enabled = true;
    let pane_id = h.add_test_pane();

    // Navigate away to a second context/window so `pane_id` is out of view.
    let win1_idx = push_second_context_window(&mut h, 2, 2);
    h.app.active_window = win1_idx;
    h.app.router.set_active(1);
    assert!(
        !h.app.pane_is_in_view(pane_id),
        "precondition: origin pane must be out of view"
    );

    let notify_id = "resurface-basic";
    h.app.enqueue_notification(
        crate::app::notifications::NotifySource::App,
        resurface_test_notification(notify_id, pane_id),
    );
    assert!(
        !h.app.pending_notifications[0].origin_in_view,
        "queuing while out of view must not itself count as a resurface"
    );
    // Raising while out of view must not force the modal open either.
    h.app.show_notification_modal = false;
    h.app.current_notify_id = None;

    // Navigate back — the pane's window comes into view.
    h.app.active_window = 0;
    h.app.router.set_active(0);
    assert!(
        h.app.pane_is_in_view(pane_id),
        "precondition: origin pane must now be in view"
    );

    h.app.resurface_in_view_notifications();

    assert!(
        h.app.show_notification_modal,
        "modal must reopen on navigation into view"
    );
    assert_eq!(h.app.current_notify_id.as_deref(), Some(notify_id));
    assert!(
        h.app.pending_notifications[0].origin_in_view,
        "origin_in_view must track the rising edge"
    );
}

/// #727: "in view" is a layout predicate, not a focus predicate — a
/// notification from a sibling pane in the same window resurfaces even
/// though a *different* pane in that window holds focus.
#[test]
fn resurfaces_for_unfocused_sibling_pane_in_multi_pane_window() {
    let mut h = HostHarness::new();
    h.app.notifications_enabled = true;
    let pane_a = h.add_test_pane();
    let pane_b = h.add_test_pane(); // orphan tile — not yet joined into the tree

    let notify_id = "resurface-sibling";
    h.app.enqueue_notification(
        crate::app::notifications::NotifySource::App,
        resurface_test_notification(notify_id, pane_b),
    );
    assert!(
        !h.app.pending_notifications[0].origin_in_view,
        "precondition: pane_b's tile is not yet part of the tree"
    );

    // Join pane_b into the same window as a horizontal sibling of pane_a —
    // this is the pane's window "coming into view".
    {
        let win = &mut h.app.windows[0];
        let tile_a = win.tree.tiles.find_pane(&pane_a).expect("tile_a exists");
        let tile_b = win.tree.tiles.find_pane(&pane_b).expect("tile_b exists");
        let root = win.tree.tiles.insert_horizontal_tile(vec![tile_a, tile_b]);
        win.tree.root = Some(root);
    }
    // Focus stays on pane_a, never pane_b.
    h.app.pane_navigate(pane_a);
    let win = &h.app.windows[0];
    let focused = win
        .focused_pane
        .and_then(|tile_id| PlexiApp::find_pane_in_tile(&win.tree, tile_id));
    assert_eq!(
        focused,
        Some(pane_a),
        "precondition: focus must be on pane_a"
    );

    h.app.resurface_in_view_notifications();

    assert!(
        h.app.show_notification_modal,
        "sibling pane's notification must resurface even though it never held focus"
    );
    assert_eq!(h.app.current_notify_id.as_deref(), Some(notify_id));
}

/// #727: once resurfaced, the same notification must not reopen the modal on
/// every subsequent frame while the origin pane stays in view — explicit
/// dismissal (or a manual close) in that state is final until the pane
/// leaves and returns.
#[test]
fn does_not_refire_while_pane_remains_in_view() {
    let mut h = HostHarness::new();
    let pane_id = h.add_test_pane();

    let notify_id = "resurface-no-refire";
    h.app.pending_notifications.push({
        let mut n = resurface_test_notification(notify_id, pane_id);
        n.origin_in_view = true; // already resurfaced once
        n
    });
    h.app.show_notification_modal = false; // user manually closed it
    h.app.current_notify_id = None;

    h.app.resurface_in_view_notifications();

    assert!(
        !h.app.show_notification_modal,
        "no rising edge while still in view — must not reopen the modal"
    );
    assert!(h.app.current_notify_id.is_none());
}

/// #727: the resurface re-arms after the origin pane leaves view and comes
/// back — a second navigation-into-view fires a second resurface.
#[test]
fn rearms_after_pane_leaves_view_and_returns() {
    let mut h = HostHarness::new();
    let pane_id = h.add_test_pane();
    let win1_idx = push_second_context_window(&mut h, 2, 2);

    let notify_id = "resurface-rearm";
    h.app.pending_notifications.push({
        let mut n = resurface_test_notification(notify_id, pane_id);
        n.origin_in_view = true; // already resurfaced once
        n
    });
    h.app.show_notification_modal = false;
    h.app.current_notify_id = None;

    // Leave view.
    h.app.active_window = win1_idx;
    h.app.router.set_active(1);
    h.app.resurface_in_view_notifications();
    assert!(
        !h.app.pending_notifications[0].origin_in_view,
        "leaving view must clear the edge"
    );
    assert!(
        !h.app.show_notification_modal,
        "a falling edge must never open the modal"
    );

    // Return.
    h.app.active_window = 0;
    h.app.router.set_active(0);
    h.app.resurface_in_view_notifications();

    assert!(
        h.app.show_notification_modal,
        "returning to view must re-fire the resurface"
    );
    assert_eq!(h.app.current_notify_id.as_deref(), Some(notify_id));
}

/// #727: resurface evaluation must happen on hidden/occluded frames — it is
/// wired into `update_preamble`, which runs from `App::logic`, never `ui`.
/// Driven purely through `HostHarness::run_hidden_frames`, never a normal
/// visible frame, so a regression that moves the call into `ui` fails this
/// test even though a visible-frame test would still pass.
#[test]
fn resurface_runs_on_hidden_occluded_frames() {
    let mut h = HostHarness::new();
    h.app.notifications_enabled = true;
    let pane_id = h.add_test_pane();
    let win1_idx = push_second_context_window(&mut h, 2, 2);

    h.app.active_window = win1_idx;
    h.app.router.set_active(1);
    h.app.enqueue_notification(
        crate::app::notifications::NotifySource::App,
        resurface_test_notification("resurface-hidden", pane_id),
    );
    h.app.show_notification_modal = false;
    h.app.current_notify_id = None;

    h.app.active_window = 0;
    h.app.router.set_active(0);

    // Only hidden/occluded passes from here — App::ui never runs.
    h.run_hidden_frames(1);

    assert!(
        h.app.show_notification_modal,
        "resurface must fire from App::logic even while the window is hidden/occluded"
    );
    assert_eq!(h.app.current_notify_id.as_deref(), Some("resurface-hidden"));
}

/// #727: a pane sitting in a background (inactive) tab of a Tabs container is
/// not in view — only the active tab's pane counts.
#[test]
fn background_tab_pane_is_not_in_view() {
    use egui_tiles::{Container, Tile};

    let mut h = HostHarness::new();
    h.app.notifications_enabled = true;
    let pane_a = h.add_test_pane();
    let pane_b = h.add_test_pane();

    let (tile_a, tile_b, tab_tile) = {
        let win = &mut h.app.windows[0];
        let tile_a = win.tree.tiles.find_pane(&pane_a).expect("tile_a");
        let tile_b = win.tree.tiles.find_pane(&pane_b).expect("tile_b");
        let tab_tile = win.tree.tiles.insert_tab_tile(vec![tile_a, tile_b]);
        win.tree.root = Some(tab_tile);
        if let Some(Tile::Container(Container::Tabs(tabs))) = win.tree.tiles.get_mut(tab_tile) {
            tabs.set_active(tile_a);
        }
        (tile_a, tile_b, tab_tile)
    };
    let _ = (tile_a, tile_b);

    assert!(
        h.app.pane_is_in_view(pane_a),
        "active tab's pane must be in view"
    );
    assert!(
        !h.app.pane_is_in_view(pane_b),
        "background tab's pane must not be in view"
    );

    // Switching the active tab brings pane_b into view and fires a resurface.
    h.app.enqueue_notification(
        crate::app::notifications::NotifySource::App,
        resurface_test_notification("resurface-tab", pane_b),
    );
    assert!(!h.app.pending_notifications[0].origin_in_view);

    if let Some(Tile::Container(Container::Tabs(tabs))) =
        h.app.windows[0].tree.tiles.get_mut(tab_tile)
    {
        tabs.set_active(tile_b);
    }
    h.app.resurface_in_view_notifications();

    assert!(
        h.app.show_notification_modal,
        "activating pane_b's tab must resurface its notification"
    );
}

/// #727: a zoomed (fullscreen) pane hides its siblings from view even though
/// they remain in the same window/context.
#[test]
fn zoomed_window_hides_sibling_panes_from_view() {
    let mut h = HostHarness::new();
    let pane_a = h.add_test_pane();
    let pane_b = h.add_test_pane();

    let tile_a = {
        let win = &mut h.app.windows[0];
        let tile_a = win.tree.tiles.find_pane(&pane_a).expect("tile_a");
        let tile_b = win.tree.tiles.find_pane(&pane_b).expect("tile_b");
        let root = win.tree.tiles.insert_horizontal_tile(vec![tile_a, tile_b]);
        win.tree.root = Some(root);
        tile_a
    };

    // Both panes are in view before zooming.
    assert!(h.app.pane_is_in_view(pane_a));
    assert!(h.app.pane_is_in_view(pane_b));

    h.app.windows[0].zoomed_pane = Some(tile_a);

    assert!(
        h.app.pane_is_in_view(pane_a),
        "the zoomed pane itself stays in view"
    );
    assert!(
        !h.app.pane_is_in_view(pane_b),
        "zoom hides the sibling pane from view"
    );
}

/// #727: focus mode (the global interruption mute) still suppresses the
/// resurface, mirroring every other interruption surface gated by
/// `notification_may_interrupt`.
#[test]
fn focus_mode_suppresses_resurface() {
    let mut h = HostHarness::new();
    h.app.notifications_enabled = true;
    let pane_id = h.add_test_pane();
    let win1_idx = push_second_context_window(&mut h, 2, 2);

    h.app.active_window = win1_idx;
    h.app.router.set_active(1);
    h.app.enqueue_notification(
        crate::app::notifications::NotifySource::App,
        resurface_test_notification("resurface-focus-mode", pane_id),
    );
    h.app.show_notification_modal = false;
    h.app.current_notify_id = None;

    h.app.notifications_focus_mode = true;
    h.app.active_window = 0;
    h.app.router.set_active(0);

    h.app.resurface_in_view_notifications();

    assert!(
        !h.app.show_notification_modal,
        "focus mode must suppress the resurface interrupt"
    );
    assert!(
        h.app.pending_notifications[0].origin_in_view,
        "the in-view state is still tracked even when suppressed"
    );
}

// ── stint 0724 Phase E: dedup parity — notification_visible /
// notification_counts_toward_context ────────────────────────────────────────
//
// Phase E extracted the scope-match logic duplicated across
// `notification_is_visible`, the `render.rs` per-pane count inline copy, and
// the two context-badge count functions into two free functions taking plain
// scalar ids. These tests enumerate scope × (same/other window) ×
// (same/other context) and assert the extracted functions answer exactly
// like the pre-extraction inline logic — pure dedup, no behavior change.

/// `notification_visible`'s full compatibility matrix: Global is always
/// visible; Window depends only on window id match; Context depends only on
/// context id match — independent of the other id in both cases.
#[test]
fn notification_visible_matches_scope_semantics_matrix() {
    use crate::app::notifications::notification_visible;
    use crate::app_protocol::NotifyScope;

    let active_window_id = 10;
    let active_context_id = 20;
    let other_window_id = 11;
    let other_context_id = 21;

    // Global: always visible, in every window/context combination.
    for (window_id, context_id) in [
        (active_window_id, active_context_id),
        (active_window_id, other_context_id),
        (other_window_id, active_context_id),
        (other_window_id, other_context_id),
    ] {
        assert!(
            notification_visible(
                NotifyScope::Global,
                window_id,
                context_id,
                active_window_id,
                active_context_id
            ),
            "Global must always be visible (window={window_id}, context={context_id})"
        );
    }

    // Window: visible iff source_window_id == active_window_id, regardless of context.
    assert!(notification_visible(
        NotifyScope::Window,
        active_window_id,
        active_context_id,
        active_window_id,
        active_context_id
    ));
    assert!(notification_visible(
        NotifyScope::Window,
        active_window_id,
        other_context_id,
        active_window_id,
        active_context_id
    ));
    assert!(!notification_visible(
        NotifyScope::Window,
        other_window_id,
        active_context_id,
        active_window_id,
        active_context_id
    ));
    assert!(!notification_visible(
        NotifyScope::Window,
        other_window_id,
        other_context_id,
        active_window_id,
        active_context_id
    ));

    // Context: visible iff source_context_id == active_context_id, regardless of window.
    assert!(notification_visible(
        NotifyScope::Context,
        active_window_id,
        active_context_id,
        active_window_id,
        active_context_id
    ));
    assert!(notification_visible(
        NotifyScope::Context,
        other_window_id,
        active_context_id,
        active_window_id,
        active_context_id
    ));
    assert!(!notification_visible(
        NotifyScope::Context,
        active_window_id,
        other_context_id,
        active_window_id,
        active_context_id
    ));
    assert!(!notification_visible(
        NotifyScope::Context,
        other_window_id,
        other_context_id,
        active_window_id,
        active_context_id
    ));
}

/// `PlexiApp::notification_is_visible` (the real call site, including the
/// snooze/`deliver_after` gate) must agree with the extracted
/// `notification_visible` for every non-snoozed notification across the same
/// scope × window × context matrix — proving the extraction changed where
/// the scope-match logic lives, not what it answers.
#[test]
fn notification_is_visible_agrees_with_extracted_scope_match_when_not_snoozed() {
    let h = HostHarness::new();
    let active_window_id = h.app.windows[h.app.active_window].window_id;
    let active_context_id = h.app.router.active().context_id;

    for scope in [
        crate::app_protocol::NotifyScope::Global,
        crate::app_protocol::NotifyScope::Window,
        crate::app_protocol::NotifyScope::Context,
    ] {
        for (source_window_id, source_context_id) in [
            (active_window_id, active_context_id),
            (active_window_id, active_context_id + 1),
            (active_window_id + 1, active_context_id),
            (active_window_id + 1, active_context_id + 1),
        ] {
            let mut n = cue_test_notification("parity");
            n.scope = scope;
            n.source_window_id = source_window_id;
            n.source_context_id = source_context_id;
            let expected = crate::app::notifications::notification_visible(
                scope,
                source_window_id,
                source_context_id,
                active_window_id,
                active_context_id,
            );
            assert_eq!(
                h.app.notification_is_visible(&n),
                expected,
                "scope={scope:?} source_window={source_window_id} source_context={source_context_id}"
            );
        }
    }
}

/// `notification_counts_toward_context`: Global never counts toward any
/// context's badge (it already renders everywhere via `notification_visible`
/// — counting it again would double-report it); Window/Context count toward
/// their originating context's badge based purely on context id match.
#[test]
fn notification_counts_toward_context_matrix() {
    use crate::app::notifications::notification_counts_toward_context;
    use crate::app_protocol::NotifyScope;

    assert!(!notification_counts_toward_context(
        NotifyScope::Global,
        5,
        5
    ));
    assert!(!notification_counts_toward_context(
        NotifyScope::Global,
        5,
        6
    ));

    assert!(notification_counts_toward_context(
        NotifyScope::Window,
        5,
        5
    ));
    assert!(!notification_counts_toward_context(
        NotifyScope::Window,
        5,
        6
    ));

    assert!(notification_counts_toward_context(
        NotifyScope::Context,
        5,
        5
    ));
    assert!(!notification_counts_toward_context(
        NotifyScope::Context,
        5,
        6
    ));
}
