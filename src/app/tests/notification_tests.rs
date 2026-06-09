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
        level: "info".into(),
        title: "Snoozed".into(),
        body: "body".into(),
        kind: crate::app_protocol::NotifyKind::Message,
        options: vec![],
        input_prompt: None,
        required: false,
        priority: 100,
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
        level: "info".into(),
        title: "ShouldNotTimeout".into(),
        body: "body".into(),
        kind: crate::app_protocol::NotifyKind::Message,
        options: vec![],
        input_prompt: None,
        required: false,
        priority: 100,
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
        level: "info".into(),
        title: "Test".into(),
        body: "body".into(),
        kind: crate::app_protocol::NotifyKind::Message,
        options: vec![],
        input_prompt: None,
        required: false,
        priority: 50,
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
        r#"[{{"notify_id":"old","sender_pane_id":0,"source_context_id":1,"level":"info","title":"Old","body":"","kind":"message","options":[],"required":false,"priority":0,"scope":"global","enqueued_at_secs":{},"tombstoned":false}}]"#,
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
        path: std::env::temp_dir(),
        root: None,
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
        level: "info".into(),
        title: "Window Notification".into(),
        body: "body".into(),
        kind: crate::app_protocol::NotifyKind::Message,
        options: vec![],
        input_prompt: None,
        required: false,
        priority: 100,
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
        level: "info".into(),
        title: "Should go away".into(),
        body: "body".into(),
        kind: crate::app_protocol::NotifyKind::Message,
        options: vec![],
        input_prompt: None,
        required: false,
        priority: 100,
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
        level: "info".into(),
        title: "Must stay".into(),
        body: "body".into(),
        kind: crate::app_protocol::NotifyKind::Message,
        options: vec![],
        input_prompt: None,
        required: true, // <-- required
        priority: 100,
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
        level: "info".into(),
        title: "From other pane".into(),
        body: "body".into(),
        kind: crate::app_protocol::NotifyKind::Message,
        options: vec![],
        input_prompt: None,
        required: false,
        priority: 100,
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
/// modal and push FocusLayer::QuickNote to the top.
#[test]
fn quicknote_preempts_notification_modal() {
    let mut h = HostHarness::new();
    h.add_test_pane();

    // Simulate a NotificationModal being open.
    h.app.push_focus_layer(FocusLayer::NotificationModal);
    assert_eq!(
        h.app.focus_stack.last(),
        Some(&FocusLayer::NotificationModal),
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
        Some(&FocusLayer::QuickNote),
        "QuickNote must be on top after preemption"
    );
}

/// #1626: Cmd+0 with CommandPalette on the focus stack must dismiss the
/// palette and push FocusLayer::QuickNote to the top.
#[test]
fn quicknote_preempts_command_palette() {
    let mut h = HostHarness::new();
    h.add_test_pane();

    h.app.push_focus_layer(FocusLayer::CommandPalette);
    assert!(
        h.app.is_quick_note_preemptable(),
        "CommandPalette must be preemptable"
    );

    h.app.dismiss_preemptable_modal();
    h.app.open_quick_note_modal();

    assert_eq!(
        h.app.focus_stack.last(),
        Some(&FocusLayer::QuickNote),
        "QuickNote must be on top after preempting CommandPalette"
    );
}

/// #1626: ConfirmClose is critical and must NOT be preemptable by QuickNote.
#[test]
fn quicknote_does_not_preempt_confirm_close() {
    let mut h = HostHarness::new();
    h.add_test_pane();

    h.app.push_focus_layer(FocusLayer::ConfirmClose);
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

    h.app.push_focus_layer(FocusLayer::CapabilityModal);
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

    h.app.push_focus_layer(FocusLayer::ContextCloseConfirm);
    assert!(
        !h.app.is_quick_note_preemptable(),
        "ContextCloseConfirm must NOT be preemptable"
    );
}
