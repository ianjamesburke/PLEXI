use super::super::*;

#[test]
fn parse_key_str_to_event_uses_sdk_key_names_for_view_panes() {
    let (key, modifiers) = parse_key_str_to_event("plus");
    assert_eq!(key, "plus");
    assert!(!modifiers.ctrl);

    let (key, _) = parse_key_str_to_event("equals");
    assert_eq!(key, "equals");

    let (key, _) = parse_key_str_to_event("ArrowDown");
    assert_eq!(key, "down");

    let (key, _) = parse_key_str_to_event("A");
    assert_eq!(key, "a");
}

#[test]
fn parse_key_str_to_event_preserves_modifiers() {
    let (key, modifiers) = parse_key_str_to_event("ctrl+shift+plus");

    assert_eq!(key, "plus");
    assert!(modifiers.ctrl);
    assert!(modifiers.shift);
    assert!(!modifiers.alt);
    assert!(!modifiers.cmd);
}

#[test]
fn key_str_to_egui_raw_input_maps_named_keys_without_text() {
    let raw = key_str_to_egui_raw_input("enter").expect("enter must map");
    assert_eq!(raw.events.len(), 2, "named keys emit press and release");
    assert!(matches!(
        raw.events[0],
        egui::Event::Key {
            key: egui::Key::Enter,
            pressed: true,
            repeat: false,
            ..
        }
    ));
    assert!(matches!(
        raw.events[1],
        egui::Event::Key { pressed: false, .. }
    ));

    let raw = key_str_to_egui_raw_input("ArrowDown").expect("ArrowDown must map");
    assert!(matches!(
        raw.events[0],
        egui::Event::Key {
            key: egui::Key::ArrowDown,
            ..
        }
    ));
}

#[test]
fn key_str_to_egui_raw_input_emits_text_for_printable_chars() {
    let raw = key_str_to_egui_raw_input("a").expect("a must map");
    assert!(matches!(
        raw.events[0],
        egui::Event::Key {
            key: egui::Key::A,
            ..
        }
    ));
    assert!(
        matches!(&raw.events[1], egui::Event::Text(t) if t == "a"),
        "printable chars must also emit Text for search fields; got {:?}",
        raw.events
    );
    assert!(matches!(
        raw.events[2],
        egui::Event::Key { pressed: false, .. }
    ));

    let raw = key_str_to_egui_raw_input("space").expect("space must map");
    assert!(matches!(&raw.events[1], egui::Event::Text(t) if t == " "));
}

#[test]
fn clipboard_chord_matches_egui_winit_vocabulary() {
    let cmd = egui::Modifiers::COMMAND;
    assert_eq!(
        clipboard_chord(cmd, egui::Key::X),
        Some(ClipboardChord::Cut)
    );
    assert_eq!(
        clipboard_chord(cmd, egui::Key::C),
        Some(ClipboardChord::Copy)
    );
    assert_eq!(
        clipboard_chord(cmd, egui::Key::V),
        Some(ClipboardChord::Paste)
    );
    assert_eq!(clipboard_chord(cmd, egui::Key::A), None);
    assert_eq!(
        clipboard_chord(egui::Modifiers::default(), egui::Key::V),
        None,
        "bare v is ordinary typing, never a paste chord"
    );
}

#[test]
fn key_str_clipboard_chords_translate_like_physical_input() {
    // Physical cmd+c/cmd+x reach widgets as Copy/Cut events (egui-winit
    // translates before egui sees them); the synthetic path must match or
    // every text surface silently drops the replayed raw chord.
    let raw = key_str_to_egui_raw_input("cmd+c").expect("cmd+c must map");
    assert_eq!(raw.events, vec![egui::Event::Copy]);

    let raw = key_str_to_egui_raw_input("cmd+x").expect("cmd+x must map");
    assert_eq!(raw.events, vec![egui::Event::Cut]);

    // cmd+v reads the live system clipboard, so only assert the shape: never
    // raw Key/Text events, at most one Paste carrying the clipboard text.
    let raw = key_str_to_egui_raw_input("cmd+v").expect("cmd+v must map");
    assert!(raw.events.len() <= 1, "got {:?}", raw.events);
    assert!(
        raw.events
            .iter()
            .all(|e| matches!(e, egui::Event::Paste(_))),
        "cmd+v must translate to Paste, never raw Key events; got {:?}",
        raw.events
    );
}

#[test]
fn key_str_to_egui_raw_input_chords_set_modifiers_and_suppress_text() {
    // ctrl+b is a chord on every platform and a clipboard chord on none, so
    // it isolates the property under test: press + release, no Text.
    let raw = key_str_to_egui_raw_input("ctrl+b").expect("ctrl+b must map");
    assert!(raw.modifiers.ctrl);
    assert_eq!(
        raw.events.len(),
        2,
        "chord keys must not emit Text; got {:?}",
        raw.events
    );

    let raw = key_str_to_egui_raw_input("cmd+enter").expect("cmd+enter must map");
    assert!(raw.modifiers.command);
}

#[test]
fn ctrl_c_is_a_copy_chord_only_where_ctrl_is_the_command_modifier() {
    // egui's `command` modifier is cmd on macOS and ctrl everywhere else, and
    // egui-winit translates a physical command+c into `Event::Copy` before
    // egui sees it. The synthetic path mirrors that, so the same key string
    // legitimately produces different events per platform — assert the
    // divergence rather than letting one platform's shape look like a bug.
    let raw = key_str_to_egui_raw_input("ctrl+c").expect("ctrl+c must map");
    assert!(raw.modifiers.ctrl);
    if cfg!(target_os = "macos") {
        assert_eq!(
            raw.events.len(),
            2,
            "on macOS ctrl is not the command modifier, so ctrl+c stays a raw chord; got {:?}",
            raw.events
        );
    } else {
        assert_eq!(
            raw.events,
            vec![egui::Event::Copy],
            "off macOS ctrl IS the command modifier, so ctrl+c must translate like physical input"
        );
    }
}

#[test]
fn key_str_to_egui_raw_input_rejects_unmappable_strings() {
    assert!(key_str_to_egui_raw_input("notakey").is_none());
}

/// Stint 1096: the terminal vocabulary of `plexi pane key`, pinned at the
/// parser so each supported spelling and each rejection reason is explicit.
#[test]
fn key_str_to_pty_bytes_encodes_the_terminal_vocabulary() {
    for (key, expected) in [
        ("alt+m", b"\x1bm".as_slice()),
        ("option+m", b"\x1bm".as_slice()),
        ("alt+M", b"\x1bM".as_slice()),
        ("alt++", b"\x1b+".as_slice()),
        ("alt+enter", b"\x1b\r".as_slice()),
        ("alt+escape", b"\x1b\x1b".as_slice()),
        ("ctrl+alt+c", b"\x1b\x03".as_slice()),
        ("control+c", b"\x03".as_slice()),
        ("ctrl+C", b"\x03".as_slice()),
        ("ctrl+space", b"\x00".as_slice()),
        ("ctrl+@", b"\x00".as_slice()),
        ("ctrl+\\", b"\x1c".as_slice()),
        ("ctrl+]", b"\x1d".as_slice()),
        ("ctrl+minus", b"\x1f".as_slice()),
        ("shift+tab", b"\x1b[Z".as_slice()),
        ("Return", b"\r".as_slice()),
        ("plus", b"+".as_slice()),
        ("minus", b"-".as_slice()),
        ("equals", b"=".as_slice()),
        ("+", b"+".as_slice()),
        ("Q", b"Q".as_slice()),
    ] {
        assert_eq!(
            key_str_to_pty_bytes(key).as_deref(),
            Ok(expected),
            "{key:?} must encode as {expected:?}"
        );
    }
}

#[test]
fn key_str_to_pty_bytes_rejects_with_a_reason_and_never_falls_back_to_text() {
    for (key, reason) in [
        ("alt+m+x", "unknown modifier \"m\""),
        ("ctrl+ctrl+c", "repeated modifier"),
        ("meta+c", "Cmd or Meta"),
        ("ctrl+shift+c", "ctrl+shift"),
        ("ctrl+tab", "ctrl applies only"),
        ("shift+enter", "shift+tab"),
        ("alt+left", "alt applies only"),
        ("alt+shift+tab", "alt applies only"),
        ("alt+", "no key after the modifiers"),
        ("++", "empty modifier"),
        ("pagedown", "unknown key name"),
    ] {
        let error = key_str_to_pty_bytes(key).expect_err(key);
        assert!(
            error.contains(reason) && error.contains("nothing was sent"),
            "{key:?} must be rejected for {reason:?}, got {error:?}"
        );
    }
}

#[test]
fn test_spawn_pane_targets_correct_window_with_from_pane_id() {
    let ctx = egui::Context::default();
    let ft = crate::platform::logging::new_frame_tick();
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, ft);

    // Window 0 is created by new_for_test. Add a pane to it.
    let (tile_in_w0, pane_in_w0) = app.add_test_pane();
    app.windows[0].focused_pane = Some(tile_in_w0);

    // Add a second window and focus it.
    let ctx_id = app.router.active().context_id;
    app.windows.push(crate::host::context::Window {
        name: "Window 1".into(),
        path: crate::testing::scratch_context_root("context-2"),
        tree: egui_tiles::Tree::empty("w1"),
        panes: Default::default(),
        focused_pane: None,
        zoomed_pane: None,
        grid_x: 1,
        grid_y: 0,
        window_id: 2,
        context_id: ctx_id,
        tree_history: Default::default(),
    });
    app.active_window = 1; // focus second window

    // Verify pane is in window 0's tree, not window 1's.
    assert!(app.windows[0].tree.tiles.find_pane(&pane_in_w0).is_some());
    assert!(app.windows[1].tree.tiles.find_pane(&pane_in_w0).is_none());

    // find_pane_in_any_window should locate it in window 0.
    let loc = app.find_pane_in_any_window(pane_in_w0);
    assert!(loc.is_some(), "pane should be found in any window");
    let (found_win_idx, found_tile) = loc.unwrap();
    assert_eq!(found_win_idx, 0, "pane should be in window 0");
    assert_eq!(found_tile, tile_in_w0);
}

/// A valid socket line must queue the request AND leave a repaint pending —
/// with zero-frame idle, the repaint is what gets the queued request drained.
#[test]
fn socket_line_queues_request_and_requests_repaint() {
    let ctx = egui::Context::default();
    let (mailbox, rx) = crate::app::ui_mailbox::UiMailbox::<crate::app::socket_caller::PaneIpcMessage>::channel(
        std::sync::Arc::new(crate::app::ui_mailbox::EguiWake::new(ctx.clone())),
        "pane_ipc",
    );
    handle_socket_line(r#"{"type":"wake"}"#, &mailbox, None, &mut std::io::sink());
    assert!(
        matches!(rx.try_recv(), Ok(crate::app::socket_caller::PaneIpcMessage::Request { request, .. }) if matches!(*request, crate::protocol::HostRequest::Wake)),
        "request must be queued on the pane-IPC channel"
    );
    assert!(
        ctx.has_requested_repaint(),
        "socket line must request a repaint so the idle UI thread wakes"
    );
}

/// A malformed socket line must queue nothing (and therefore wake nothing).
#[test]
fn socket_line_parse_error_queues_nothing() {
    let ctx = egui::Context::default();
    let (mailbox, rx) = crate::app::ui_mailbox::UiMailbox::<crate::app::socket_caller::PaneIpcMessage>::channel(
        std::sync::Arc::new(crate::app::ui_mailbox::EguiWake::new(ctx.clone())),
        "pane_ipc",
    );
    handle_socket_line("definitely not json", &mailbox, None, &mut std::io::sink());
    assert!(
        rx.try_recv().is_err(),
        "parse failures must not queue anything"
    );
}

fn run_socket_connection(
    payload: &[u8],
) -> crate::app::ui_mailbox::MailboxReceiver<crate::app::socket_caller::PaneIpcMessage> {
    use std::io::Write as _;

    let ctx = egui::Context::default();
    let wake = std::sync::Arc::new(crate::app::ui_mailbox::EguiWake::new(ctx));
    let (mailbox, rx) = crate::app::ui_mailbox::UiMailbox::<crate::app::socket_caller::PaneIpcMessage>::channel(
        wake.clone(),
        "pane_ipc",
    );
    let (subscribe_mailbox, _subscribe_rx) = crate::app::ui_mailbox::UiMailbox::<
        crate::host::event_subscriptions::HostSubscribeRequest,
    >::channel(wake.clone(), "event_subscribe");
    let (publish_mailbox, _publish_rx) = crate::app::ui_mailbox::UiMailbox::<
        crate::host::event_subscriptions::HostPublishRequest,
    >::channel(wake, "event_publish");
    let (server, mut client) = std::os::unix::net::UnixStream::pair().expect("socket pair");
    client.write_all(payload).expect("write payload");
    client
        .shutdown(std::net::Shutdown::Write)
        .expect("close write half");
    let profile = tempfile::tempdir().unwrap();
    handle_socket_connection(
        profile.path(),
        server,
        mailbox,
        subscribe_mailbox,
        publish_mailbox,
        None,
    );
    rx
}

#[cfg(unix)]
#[test]
fn pane_send_disconnected_socket_does_not_dispatch_after_approval() {
    pane_send_socket_approval(true, false);
}

#[test]
fn pane_send_rejects_missing_reply_stream_before_dispatch() {
    #[derive(Default)]
    struct MissingReply(Vec<u8>);
    impl std::io::Write for MissingReply {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.write(bytes)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl SocketAckWriter for MissingReply {
        fn requires_reply_stream(&self) -> bool {
            true
        }
    }
    let wake = std::sync::Arc::new(crate::app::ui_mailbox::EguiWake::new(
        egui::Context::default(),
    ));
    let (mailbox, rx) = crate::app::ui_mailbox::UiMailbox::channel(wake, "test");
    let profile = tempfile::tempdir().unwrap();
    let mut reply = MissingReply::default();
    let request = serde_json::json!({"type":"send_to_pane", "pane_id":1, "text":"hello", "response_file":profile.path().join("response")});
    handle_socket_line_in(
        profile.path(),
        &request.to_string(),
        &mailbox,
        None,
        &mut reply,
    );
    assert!(rx.try_recv().is_err());
    let response: serde_json::Value = serde_json::from_slice(&reply.0).unwrap();
    assert_eq!(response["error_code"], "reply_connection_unavailable");
}

#[cfg(unix)]
#[test]
fn pane_send_connected_socket_receives_approval_and_dispatches_once() {
    pane_send_socket_approval(false, false);
}

#[cfg(unix)]
#[test]
fn pane_send_pane_lifetime_grant_deduplicates_and_unknown_claim_never_replays() {
    pane_send_socket_approval(false, true);
}

/// An explicit operation id is a promise only the broker can keep: a sender
/// outside it is refused, and a pane's send with one waits for a decision.
#[test]
fn pane_send_explicit_operation_id_needs_a_brokered_caller() {
    let mut h = crate::testing::HostHarness::new();
    let caller = h.add_focused_terminal();
    let target = h.add_focused_terminal();
    h.terminal_backend(target).enable_input_tap();
    let profile = tempfile::tempdir().unwrap();
    let send = |reply: &std::path::Path| crate::protocol::HostRequest::SendToPane {
        pane_id: target,
        text: "hello".into(),
        submit: false,
        paste: false,
        response_file: Some(reply.to_string_lossy().into_owned()),
        operation_id: Some("explicit-op".into()),
        generated_operation_id: false,
    };
    let host_reply = profile.path().join("host");
    h.inject_in_process(send(&host_reply));
    let body: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&host_reply).unwrap()).unwrap();
    assert_eq!(body["error_code"], "idempotency_unavailable", "{body}");
    assert!(h.terminal_backend(target).take_input_tap().is_empty());

    let pane_reply = profile.path().join("pane");
    h.inject_from_pane(caller, send(&pane_reply));
    h.hidden_frame();
    assert!(!pane_reply.exists(), "the pane's send waits for a decision");
    assert_eq!(h.app.pending_send_approvals.len(), 1);
    assert!(h.terminal_backend(target).take_input_tap().is_empty());
}

#[cfg(unix)]
fn pane_send_socket_approval(disconnect: bool, standing: bool) {
    use std::io::Read as _;
    use std::io::Write as _;
    use std::time::Duration;
    let mut h = crate::testing::HostHarness::new();
    let target = h.add_focused_terminal();
    h.terminal_backend(target).enable_input_tap();
    let ws = h.workspace_root();
    let agent = ws.join(".plexi/agents/socket-test");
    std::fs::create_dir_all(&agent).unwrap();
    std::fs::write(agent.join("AGENT.md"), "Test agent").unwrap();
    std::fs::write(agent.join("settings.toml"), "[agent]\nid = \"socket-test\"\ndisplay_name = \"Socket test\"\ndefault_tier = \"medium\"\n[harness]\ncli = \"claude\"\n[permissions]\ndefault_posture = \"allow\"\nask = [\"pane.send\"]\n").unwrap();
    let (caller, _) = h
        .app
        .spawn_agent_worker_pane(
            "socket-test",
            "claude",
            "sleep 30",
            "test",
            None,
            false,
            None,
            Some(ws),
            None,
            None,
            true,
            None,
        )
        .unwrap();
    let caller_pid = h.app.windows[0].panes[&caller]
        .as_terminal()
        .unwrap()
        .backend
        .child_pid();
    let mailbox = h.ipc_tx.clone();
    let wake = std::sync::Arc::new(crate::app::ui_mailbox::EguiWake::new(
        egui::Context::default(),
    ));
    let (subscribe, _sub_rx) = crate::app::ui_mailbox::UiMailbox::<
        crate::host::event_subscriptions::HostSubscribeRequest,
    >::channel(wake.clone(), "subscribe");
    let (publish, _pub_rx) = crate::app::ui_mailbox::UiMailbox::<
        crate::host::event_subscriptions::HostPublishRequest,
    >::channel(wake, "publish");
    let (server, mut client) = std::os::unix::net::UnixStream::pair().unwrap();
    let profile = tempfile::tempdir().unwrap();
    let profile_path = profile.path().to_path_buf();
    let response = profile.path().join("send-response");
    let payload = serde_json::json!({"type":"send_to_pane", "pane_id": target,
        "text":"hello", "operation_id":"socket-liveness-test", "response_file": response});
    let worker = std::thread::spawn(move || {
        handle_socket_connection(
            &profile_path,
            server,
            mailbox,
            subscribe,
            publish,
            Some(crate::broker::caller::PeerWitness::for_live_pids(&[std::process::id(), caller_pid])),
        );
    });
    writeln!(client, "{payload}").unwrap();
    worker.join().unwrap();
    let deadline =
        std::time::Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(3));
    while std::time::Instant::now() < deadline {
        h.hidden_frame();
        if !h.app.pending_send_approvals.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let pending = h
        .app
        .pending_send_approvals
        .first()
        .expect("request parked")
        .pending_request_id
        .clone();
    let mut client = Some(client);
    // The peer process remains alive while only its socket is closed.
    if disconnect {
        drop(client.take());
    }
    let monitor =
        crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir()).unwrap();
    monitor
        .approve_pending(
            &pending,
            if standing {
                crate::broker::gate::ApprovalChoice::Session
            } else {
                crate::broker::gate::ApprovalChoice::Once
            },
        )
        .unwrap();
    h.hidden_frame();
    if disconnect {
        assert!(
            h.terminal_backend(target).take_input_tap().is_empty(),
            "a disconnected caller must not trigger a late write"
        );
        assert_eq!(
            monitor
                .host_records()
                .run("socket-liveness-test")
                .unwrap()
                .state,
            crate::broker::gate::ContinuationState::Paused
        );
    } else {
        let mut client = client.unwrap();
        use std::os::fd::AsRawFd as _;
        let mut poll = libc::pollfd {
            fd: client.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: poll points to one valid pollfd.
        assert!(
            unsafe { libc::poll(&mut poll, 1, 3_000) } > 0,
            "reply must arrive within deadline"
        );
        let mut reply = String::new();
        client.read_to_string(&mut reply).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&reply).unwrap()["ok"],
            true
        );
        assert_eq!(
            h.terminal_backend(target).take_input_tap(),
            b"hello",
            "one approval writes exactly once"
        );
        assert_eq!(
            monitor
                .host_records()
                .run("socket-liveness-test")
                .unwrap()
                .state,
            crate::broker::gate::ContinuationState::Completed
        );
        if standing {
            let dispatch = |h: &mut crate::testing::HostHarness, operation: &str, name: &str| {
                let reply = profile.path().join(name);
                h.inject_socket(
                    serde_json::from_value(serde_json::json!({
                        "type": "send_to_pane", "pane_id": target, "text": "hello",
                        "operation_id": operation, "response_file": reply,
                    }))
                    .unwrap(),
                    Some(crate::broker::caller::PeerWitness::for_live_pids(&[
                        std::process::id(),
                        caller_pid,
                    ])),
                );
                h.hidden_frame();
                serde_json::from_slice::<serde_json::Value>(&std::fs::read(reply).unwrap()).unwrap()
            };
            let duplicate = dispatch(&mut h, "socket-liveness-test", "duplicate");
            assert_eq!(duplicate["duplicate"], true, "{duplicate}");
            assert!(h.terminal_backend(target).take_input_tap().is_empty());

            let fresh = dispatch(&mut h, "persistent-fresh", "fresh");
            assert_eq!(
                fresh["ok"], true,
                "the standing grant permits a fresh operation: {fresh}"
            );
            assert_eq!(h.terminal_backend(target).take_input_tap(), b"hello");
            assert!(monitor.host_records().pending().is_empty());

            // Seed the durable pre-effect claim left by a crash. The boundary
            // must return unknown and never execute it under the still-live grant.
            let text_hash = crate::broker::gate::fingerprint_args(
                &serde_json::json!({"text":"hello"}).to_string(),
            )
            .unwrap();
            let fingerprint = crate::broker::gate::fingerprint_args(
                &serde_json::json!({
                    "pane_id":target, "submit":false, "paste":false,
                    "text_fingerprint":text_hash, "operation_id":"persistent-unknown"
                })
                .to_string(),
            )
            .unwrap();
            let context = h
                .app
                .windows
                .iter()
                .find(|window| window.panes.contains_key(&caller))
                .unwrap()
                .context_id;
            assert!(matches!(
                monitor
                    .claim_pane_send(
                        "persistent-unknown",
                        &h.app.pane_principal(caller).unwrap(),
                        context,
                        &fingerprint
                    )
                    .unwrap(),
                crate::broker::gate::PaneSendClaim::Claimed(_)
            ));
            let unknown = dispatch(&mut h, "persistent-unknown", "unknown");
            assert_eq!(unknown["error_code"], "outcome_unknown", "{unknown}");
            assert!(
                h.terminal_backend(target).take_input_tap().is_empty(),
                "unknown claimed operation must not replay"
            );
        }
    }
}

#[test]
fn socket_connection_discards_valid_json_without_newline_frame() {
    let rx = run_socket_connection(br#"{"type":"wake"}"#);
    assert!(
        rx.try_recv().is_err(),
        "EOF-terminated JSON without a newline is an incomplete frame"
    );
}

#[test]
fn socket_connection_dispatches_newline_framed_json_once() {
    let rx = run_socket_connection(b"{\"type\":\"wake\"}\n");
    assert!(matches!(
        rx.try_recv(),
        Ok(crate::app::socket_caller::PaneIpcMessage::Request { request, .. }) if matches!(*request, crate::protocol::HostRequest::Wake)
    ));
    assert!(
        rx.try_recv().is_err(),
        "one complete frame must dispatch exactly once"
    );
}

/// `HostRequest::Wake` through the host handler is a pure no-op: no panic,
/// no window/pane state change.
#[test]
fn wake_request_is_noop_on_host() {
    let mut h = crate::testing::HostHarness::new();
    let windows_before = h.app.windows.len();
    let panes_before: usize = h.app.windows.iter().map(|w| w.panes.len()).sum();

    h.inject_ipc(crate::protocol::HostRequest::Wake);
    h.app.drain_pane_cmd_channel();

    assert_eq!(
        h.app.windows.len(),
        windows_before,
        "wake must not touch windows"
    );
    let panes_after: usize = h.app.windows.iter().map(|w| w.panes.len()).sum();
    assert_eq!(panes_after, panes_before, "wake must not touch panes");
}

/// #2283: a `[file_handlers]` entry routes a matching extension into the named
/// Plexi plugin via the host open resolver (OpenInPane), rather than falling
/// through to the OS opener. Proves resolution tier (a) + the launch path.
#[test]
fn file_handler_config_routes_extension_to_plugin() {
    let ctx = egui::Context::default();
    let ft = crate::platform::logging::new_frame_tick();
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, ft);

    // Map .md -> the builtin text-editor.
    let mut handlers = std::collections::HashMap::new();
    handlers.insert("md".to_string(), "app:text-editor".to_string());
    app.config.file_handlers = Some(handlers);

    let panes_before: usize = app.windows.iter().map(|w| w.panes.len()).sum();

    // sender_pane_id = 0 -> no pane -> workspace path check is bypassed
    // (trusted host path), so a temp .md path opens cleanly.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("note.md");
    std::fs::write(&path, "# note").expect("write note");
    let path = path.to_string_lossy().to_string();
    app.dispatch_open_artifact(0, path, crate::protocol::ArtifactOpenMode::OpenInPane);

    let panes_after: usize = app.windows.iter().map(|w| w.panes.len()).sum();
    assert_eq!(
        panes_after,
        panes_before + 1,
        "the md handler should open exactly one new in-Plexi pane"
    );

    let opened_text_editor = app.windows.iter().any(|w| {
        w.panes.values().any(|p| {
            p.as_view()
                .map(|a| a.runtime.type_id() == "text-editor")
                .unwrap_or(false)
        })
    });
    assert!(
        opened_text_editor,
        "the file_handler-routed pane should be the text-editor plugin"
    );
}

#[test]
fn stable_routes_only_images_to_native_viewer_apps() {
    let ctx = egui::Context::default();
    let ft = crate::platform::logging::new_frame_tick();
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, ft);
    let (_tile, _pane) = app.add_test_pane();
    // Gated players fall back to the OS opener; never launch a real app.
    crate::app::canvas_bindings::set_test_opener(Some("/usr/bin/true"));
    let dir = tempfile::tempdir().expect("tempdir");

    for (filename, expected_id, opens_pane) in [
        ("photo.png", "image-viewer", true),
        ("clip.mp4", "video-player", false),
        ("song.mp3", "audio-player", false),
    ] {
        let panes_before: usize = app.windows.iter().map(|w| w.panes.len()).sum();
        let path = dir.path().join(filename);
        std::fs::write(&path, b"media bytes").expect("write media");
        let path = path.to_string_lossy().to_string();

        app.dispatch_open_artifact(0, path, crate::protocol::ArtifactOpenMode::OpenInPane);

        let panes_after: usize = app.windows.iter().map(|w| w.panes.len()).sum();
        if opens_pane {
            assert_eq!(
                panes_after,
                panes_before + 1,
                "{filename} should open exactly one in-Plexi pane"
            );
            assert!(
                app.windows.iter().any(|w| {
                    w.panes.values().any(|p| {
                        p.as_view()
                            .map(|a| a.runtime.type_id() == expected_id)
                            .unwrap_or(false)
                    })
                }),
                "{filename} should route to {expected_id}"
            );
        } else {
            assert_eq!(
                panes_after, panes_before,
                "stable must not open the beta-only {expected_id} surface for {filename}"
            );
        }
    }
}

#[test]
fn explorer_media_viewer_close_returns_focus_and_preserves_selection() {
    let ctx = egui::Context::default();
    let ft = crate::platform::logging::new_frame_tick();
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, ft);
    let dir = tempfile::tempdir().expect("tempdir");
    let selected_path = dir.path().join("photo.png");
    std::fs::write(&selected_path, b"not decoded in this test").expect("write image");

    let browser: Box<dyn crate::app::view_trait::View> = Box::new(
        crate::file_browser::FileBrowserView::new(dir.path().to_path_buf()),
    );
    app.open_builtin_view_pane(
        browser,
        crate::app::permissions::PluginPermissions::builtin(),
        dir.path().to_path_buf(),
        Some("cwd".to_string()),
        Some("split_v"),
        Some(0.5),
    );

    let browser_tile = app.windows[0].focused_pane.expect("browser focused");
    let browser_pane_id = match app.windows[0].tree.tiles.get(browser_tile) {
        Some(egui_tiles::Tile::Pane(id)) => *id,
        other => panic!("expected browser pane tile, got {other:?}"),
    };
    let browser_state_before = app.windows[0]
        .panes
        .get(&browser_pane_id)
        .and_then(|p| p.as_view())
        .and_then(|a| a.runtime.serialize_state())
        .expect("browser state before");
    assert_eq!(browser_state_before["selected"], 0);

    app.dispatch_open_artifact(
        browser_pane_id,
        selected_path.to_string_lossy().to_string(),
        crate::protocol::ArtifactOpenMode::OpenInPane,
    );

    let viewer_tile = app.windows[0].focused_pane.expect("viewer focused");
    assert_ne!(
        viewer_tile, browser_tile,
        "viewer should open as split sibling"
    );
    let viewer_pane_id = match app.windows[0].tree.tiles.get(viewer_tile) {
        Some(egui_tiles::Tile::Pane(id)) => *id,
        other => panic!("expected viewer pane tile, got {other:?}"),
    };
    assert!(
        app.windows[0]
            .panes
            .get(&viewer_pane_id)
            .and_then(|p| p.as_view())
            .map(|a| a.runtime.type_id() == "image-viewer")
            .unwrap_or(false),
        "selected image should open in native image viewer"
    );

    app.close_focused();

    // Compared by pane identity: simplify re-wraps panes in Tabs and re-ids tiles.
    let focus = app.windows[0].focused_pane.expect("explorer focused");
    assert!(matches!(
        app.windows[0].tree.tiles.get(focus),
        Some(egui_tiles::Tile::Pane(_))
    ));
    assert_eq!(
        PlexiApp::find_pane_in_tile(&app.windows[0].tree, focus),
        Some(browser_pane_id),
        "closing viewer should return focus to explorer sibling"
    );
    let browser_state_after = app.windows[0]
        .panes
        .get(&browser_pane_id)
        .and_then(|p| p.as_view())
        .and_then(|a| a.runtime.serialize_state())
        .expect("browser state after");
    assert_eq!(
        browser_state_after["selected"], browser_state_before["selected"],
        "browser selection should survive viewer open and close"
    );
}

/// Without an invoking terminal, Explorer falls back to the context root.
#[test]
fn file_browser_without_terminal_opens_at_context_root() {
    let ctx = egui::Context::default();
    let ft = crate::platform::logging::new_frame_tick();
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, ft);

    let root = std::env::temp_dir().join("plexi-fb-root-test");
    std::fs::create_dir_all(&root).expect("mkdir root");
    app.set_context_root(root.clone(), None);

    app.open_file_browser();

    let fb_root = app.windows.iter().find_map(|w| {
        w.panes.values().find_map(|p| {
            p.as_view()
                .filter(|a| a.runtime.type_id() == "file_browser")
                .map(|a| a.workspace_root.clone())
        })
    });

    assert_eq!(
        fb_root.as_deref(),
        Some(root.as_path()),
        "file browser without a terminal should use the context root"
    );
}

/// When `plexi pane new --window` is called with a `from_pane_id` in a non-active
/// context, the new window must land in that pane's context — not the active one.
/// Regression for: `new_window` derived ws_id and grid_y from active_window instead
/// of from the calling pane's window.
#[test]
fn spawn_pane_new_window_uses_caller_context_not_active() {
    let ctx = egui::Context::default();
    let ft = crate::platform::logging::new_frame_tick();
    let (mut app, ipc_tx) = PlexiApp::new_for_test(ctx, ft);

    // Context 1 (id=1) is created by new_for_test at grid_y=0.
    // Add a pane to context 1's window so we have a valid from_pane_id.
    let (tile_ctx1, pane_id_ctx1) = app.add_test_pane();
    app.windows[0].focused_pane = Some(tile_ctx1);

    // Add context 2 at grid_y=1 and make it the active context.
    let ctx2_id: u64 = 2;
    app.router.push(crate::host::context::Context {
        name: "Context 2".into(),
        root: crate::testing::scratch_context_root("context-2"),
        description: None,
        context_id: ctx2_id,
        parent_id: None,
        depth: 0,
        parked: false,
    });
    app.windows.push(crate::host::context::Window {
        name: "Context 2".into(),
        path: crate::testing::scratch_context_root("context-2"),
        tree: egui_tiles::Tree::empty("ctx2_tree"),
        panes: Default::default(),
        focused_pane: None,
        zoomed_pane: None,
        grid_x: 0,
        grid_y: 1,
        window_id: 10,
        context_id: ctx2_id,
        tree_history: Default::default(),
    });
    app.active_window = 1;
    app.router.set_active(1);

    let windows_before = app.windows.len();

    // Inject a spawn-pane IPC from context 1's pane (pane_id_ctx1) with layout=new_window.
    let _ = ipc_tx.send(crate::app::socket_caller::PaneIpcMessage::host(crate::protocol::HostRequest::SpawnPane {
        type_id: "terminal".to_string(),
        layout: Some("new_window".to_string()),
        args: vec![],
        from_pane_id: Some(pane_id_ctx1),
        request_id: None,
        response_file: None,
        ephemeral: false,
        cwd: None,
        no_focus: false,
        path: None,
        workspace_root: None,
        target_context: None,
        context_name: None,
        name: None,
        agent_cmd: None,
        boot_timeout_secs: None,
        worktree: None,
        result_schema: None,
    }));
    app.drain_pane_cmd_channel();

    // PTY creation may fail in some CI environments; guard before asserting.
    if app.windows.len() == windows_before {
        return; // terminal spawn failed; skip remainder
    }

    let new_win = app.windows.last().expect("new window must exist");
    assert_eq!(
        new_win.context_id, 1,
        "new window must be in context 1 (caller's context), not context 2 (active)"
    );
    assert_eq!(
        new_win.grid_y, 0,
        "new window must be in row 0 (caller's grid row), not row 1 (active context's row)"
    );
}

/// When `plexi pane new --tab` is called with a `from_pane_id` in a non-active
/// window, the new tab must land in that pane's window — not the active window.
/// Regression for stint 0337: the `tab` IPC branch called `new_tab()` against
/// `self.active_window` unconditionally, ignoring `from_pane_id` entirely.
#[test]
fn spawn_pane_tab_anchors_to_from_pane_window_not_active() {
    let ctx = egui::Context::default();
    let ft = crate::platform::logging::new_frame_tick();
    let (mut app, ipc_tx) = PlexiApp::new_for_test(ctx, ft);

    // Window 0 is created by new_for_test; give it a caller pane.
    let (tile_in_w0, pane_id_ctx1) = app.add_test_pane();
    app.windows[0].focused_pane = Some(tile_in_w0);

    // Add a second window and make it the active one.
    let ctx2_id: u64 = 2;
    app.router.push(crate::host::context::Context {
        name: "Context 2".into(),
        root: crate::testing::scratch_context_root("context-2"),
        description: None,
        context_id: ctx2_id,
        parent_id: None,
        depth: 0,
        parked: false,
    });
    app.windows.push(crate::host::context::Window {
        name: "Context 2".into(),
        path: crate::testing::scratch_context_root("context-2"),
        tree: egui_tiles::Tree::empty("ctx2_tree"),
        panes: Default::default(),
        focused_pane: None,
        zoomed_pane: None,
        grid_x: 0,
        grid_y: 1,
        window_id: 10,
        context_id: ctx2_id,
        tree_history: Default::default(),
    });
    app.active_window = 1;
    app.router.set_active(1);

    let panes_in_w0_before = app.windows[0].panes.len();
    let panes_in_w1_before = app.windows[1].panes.len();

    let requested_cwd = std::env::temp_dir();

    // Inject a spawn-pane IPC from window 0's pane (pane_id_ctx1) with layout=tab,
    // while window 1 is active.
    let _ = ipc_tx.send(crate::app::socket_caller::PaneIpcMessage::host(crate::protocol::HostRequest::SpawnPane {
        type_id: "terminal".to_string(),
        layout: Some("tab".to_string()),
        args: vec![],
        from_pane_id: Some(pane_id_ctx1),
        request_id: None,
        response_file: None,
        ephemeral: false,
        cwd: Some(requested_cwd.to_string_lossy().to_string()),
        no_focus: false,
        path: None,
        workspace_root: None,
        target_context: None,
        context_name: None,
        name: None,
        agent_cmd: None,
        boot_timeout_secs: None,
        worktree: None,
        result_schema: None,
    }));
    app.drain_pane_cmd_channel();

    // PTY creation may fail in some CI environments; guard on total pane count
    // (not just window 0's) so a regression that lands the tab in window 1
    // instead of window 0 doesn't get mistaken for a skipped spawn failure.
    let total_before = panes_in_w0_before + panes_in_w1_before;
    let total_after = app.windows[0].panes.len() + app.windows[1].panes.len();
    if total_after == total_before {
        return; // terminal spawn failed; skip remainder
    }

    assert_eq!(
        app.windows[0].panes.len(),
        panes_in_w0_before + 1,
        "new tab must be added to window 0 (caller's window), not window 1 (active)"
    );
    assert_eq!(
        app.windows[1].panes.len(),
        panes_in_w1_before,
        "active window 1 must be untouched by a tab anchored to a different caller pane"
    );

    let new_pane_id = *app.windows[0]
        .panes
        .keys()
        .find(|id| **id != pane_id_ctx1)
        .expect("a new pane must have been added to window 0");
    let new_tile = app.windows[0]
        .tree
        .tiles
        .find_pane(&new_pane_id)
        .expect("new pane must be present in window 0's tile tree");
    // lsof-based cwd lookup races the freshly spawned child reporting its own
    // cwd; only assert when it resolved in time.
    if let Some(new_pane_cwd) = app.windows[0].get_focused_pane_cwd(new_tile) {
        assert_eq!(
            new_pane_cwd.canonicalize().unwrap_or(new_pane_cwd),
            requested_cwd.canonicalize().unwrap_or(requested_cwd),
            "new tab must honor the requested --cwd"
        );
    }
}

/// When `plexi host start` boots windowless (active window has no root pane and
/// no focused pane), a `plexi pane new` split request must still materialize a
/// pane. Regression for stint 0348: the empty-window fallback routed through
/// `split_focused`, which returns early with no focused pane to split, so the
/// spawn was silently dropped — `pane list` returned `[]` and the requested
/// name never applied. The fallback must seed a root pane instead, apply the
/// requested name, and write the actually-created pane id to the response file.
#[test]
fn spawn_pane_seeds_root_in_empty_window() {
    let ctx = egui::Context::default();
    let ft = crate::platform::logging::new_frame_tick();
    let (mut app, ipc_tx) = PlexiApp::new_for_test(ctx, ft);

    // new_for_test's sole window boots empty — no root, no focused pane —
    // exactly the windowless-boot state.
    assert!(
        app.windows[0].tree.root.is_none(),
        "precondition: window boots with no root pane"
    );
    assert!(
        app.windows[0].focused_pane.is_none(),
        "precondition: window boots with no focused pane"
    );

    let response_file =
        std::env::temp_dir().join(format!("plexi_test_seed_root_{}.json", std::process::id()));
    let _ = std::fs::remove_file(&response_file);

    // Split spawn (not --window), no from_pane_id: falls into the empty-window
    // fallback branch.
    let _ = ipc_tx.send(crate::app::socket_caller::PaneIpcMessage::host(crate::protocol::HostRequest::SpawnPane {
        type_id: "terminal".to_string(),
        layout: Some("split_h".to_string()),
        args: vec![],
        from_pane_id: None,
        request_id: None,
        response_file: Some(response_file.to_string_lossy().to_string()),
        ephemeral: false,
        cwd: None,
        no_focus: false,
        path: None,
        workspace_root: None,
        target_context: None,
        context_name: None,
        name: Some("seeded".to_string()),
        agent_cmd: None,
        boot_timeout_secs: None,
        worktree: None,
        result_schema: None,
    }));
    app.drain_pane_cmd_channel();

    // Determine PTY availability independently of the code under test:
    // `create_page_at` spawns a terminal via the same `TerminalPane::new` path.
    // If it can't create one here (a headless CI without a PTY), the fallback
    // likewise can't, so skip. When it CAN, the empty-window fallback must
    // produce a pane — so we assert unconditionally below. This split is
    // deliberate: guarding the real assertions on "panes.is_empty()" would let
    // the very bug under test (a silently dropped spawn) masquerade as a
    // skipped PTY failure.
    let pty_available = {
        let ctx = egui::Context::default();
        let ft = crate::platform::logging::new_frame_tick();
        let (mut probe, _tx) = PlexiApp::new_for_test(ctx, ft);
        let before = probe.windows.len();
        let launch = probe.test_terminal_launch(1, None, None);
        probe.create_page_at(9, 9, 1, launch);
        probe.windows.len() > before
    };
    if !pty_available {
        let _ = std::fs::remove_file(&response_file);
        return; // no PTY in this environment; cannot exercise terminal spawn
    }

    assert_eq!(
        app.windows[0].panes.len(),
        1,
        "empty-window spawn must seed exactly one root pane"
    );
    assert!(
        app.windows[0].tree.root.is_some(),
        "seeded window must have a tree root"
    );

    let pane_id = *app.windows[0]
        .panes
        .keys()
        .next()
        .expect("seeded pane must exist");

    // Requested name must have applied to the seeded pane.
    let name = app.windows[0]
        .panes
        .get(&pane_id)
        .and_then(|p| p.as_terminal())
        .and_then(|t| t.name.clone());
    assert_eq!(
        name.as_deref(),
        Some("seeded"),
        "requested pane name must apply to the seeded root pane"
    );

    // Response file must report the id of the pane actually created.
    let raw =
        std::fs::read_to_string(&response_file).expect("spawn_pane must write a response file");
    let _ = std::fs::remove_file(&response_file);
    let json: serde_json::Value =
        serde_json::from_str(&raw).expect("response file must be valid JSON");
    assert_eq!(
        json["pane_id"].as_u64(),
        Some(pane_id),
        "response file pane_id must match the seeded pane's id"
    );
}

/// The notify-socket thread must wake an idle UI thread promptly: one queued
/// request, one repaint callback with the small nonzero IPC wake delay (a
/// zero delay would schedule an extra settling paint; a missing or large
/// delay leaves the request parked until an unrelated frame — stint 0479).
#[test]
fn socket_lines_after_idle_each_request_a_prompt_repaint() {
    use std::sync::{Arc, Mutex};

    let ctx = egui::Context::default();
    // Match a real eframe host after its first pass. Egui advances delayed
    // repaint deadlines by this predicted frame time before invoking the
    // native callback; a wake shorter than `predicted_dt` silently becomes an
    // immediate RepaintNow event instead of the intended one-shot delay.
    for _ in 0..3 {
        let _ = ctx.run_ui(
            egui::RawInput {
                predicted_dt: 1.0 / 60.0,
                ..Default::default()
            },
            |_| {},
        );
    }
    let delays: Arc<Mutex<Vec<std::time::Duration>>> = Arc::new(Mutex::new(Vec::new()));
    let delays_cb = delays.clone();
    ctx.set_request_repaint_callback(move |info| {
        delays_cb.lock().unwrap().push(info.delay);
    });

    let (mailbox, rx) = crate::app::ui_mailbox::UiMailbox::<crate::app::socket_caller::PaneIpcMessage>::channel(
        Arc::new(crate::app::ui_mailbox::EguiWake::new(ctx.clone())),
        "pane_ipc",
    );
    let line = r#"{"type":"log_marker","source":"test","message":"wake"}"#;
    handle_socket_line(line, &mailbox, None, &mut std::io::sink());

    match rx.try_recv() {
        Ok(crate::app::socket_caller::PaneIpcMessage::Request { request, .. }) => {
            let crate::protocol::HostRequest::LogMarker { source, .. } = *request else {
                panic!("expected queued LogMarker request, got {request:?}");
            };
            assert_eq!(source, "test");
        }
        other => panic!("expected queued LogMarker request, got {other:?}"),
    }
    // Simulate the host returning to idle after the ready/startup request,
    // then issue the same response-file style command a later `pane list`
    // sends. Both external arrivals must cross the repaint callback with a
    // nonzero delay; eframe rejects a raw immediate wake during a stale pass.
    let _ = ctx.run_ui(
        egui::RawInput {
            predicted_dt: 1.0 / 60.0,
            ..Default::default()
        },
        |_| {},
    );
    handle_socket_line(
        r#"{"type":"list_panes","response_file":"/tmp/second-pane-list.json"}"#,
        &mailbox,
        None,
        &mut std::io::sink(),
    );
    assert!(
        matches!(
            rx.try_recv(),
            Ok(crate::app::socket_caller::PaneIpcMessage::Request { request, .. }) if matches!(*request, crate::protocol::HostRequest::ListPanes { .. })
        ),
        "post-idle pane list must queue on the pane-IPC channel"
    );

    let delays = delays.lock().unwrap();
    assert_eq!(
        delays.len(),
        2,
        "each socket line, including a post-idle pane list, must trigger one wake callback"
    );
    // The callback delay must remain nonzero after egui subtracts
    // `predicted_dt`. A zero callback is RepaintNow in eframe; its stale-pass
    // rejection can strand the queued IPC request until unrelated input.
    assert!(
        delays
            .iter()
            .all(|delay| *delay > std::time::Duration::ZERO),
        "IPC wake collapsed to RepaintNow after egui predicted_dt adjustment: {delays:?}"
    );
    assert!(
        delays
            .iter()
            .all(|delay| *delay <= crate::app::ui_mailbox::IPC_WAKE_DELAY),
        "wake delay must be prompt (requested {:?}, got {delays:?})",
        crate::app::ui_mailbox::IPC_WAKE_DELAY
    );
}

/// A malformed socket line must neither queue a request nor wake the UI.
#[test]
fn socket_line_parse_error_does_not_wake() {
    use std::sync::{Arc, Mutex};

    let ctx = egui::Context::default();
    let woke: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));
    let woke_cb = woke.clone();
    ctx.set_request_repaint_callback(move |_| *woke_cb.lock().unwrap() += 1);

    let (mailbox, rx) = crate::app::ui_mailbox::UiMailbox::<crate::app::socket_caller::PaneIpcMessage>::channel(
        Arc::new(crate::app::ui_mailbox::EguiWake::new(ctx.clone())),
        "pane_ipc",
    );
    handle_socket_line("not json", &mailbox, None, &mut std::io::sink());

    assert!(rx.try_recv().is_err(), "malformed line must not queue");
    assert_eq!(*woke.lock().unwrap(), 0, "malformed line must not wake");
}

#[test]
fn displayed_version_is_build_info_not_profile_installed_tag() {
    let _h = crate::testing::HostHarness::new();
    std::fs::write(
        crate::config::config_dir().join("installed_tag"),
        "v999.0.0",
    )
    .unwrap();
    let (app, _ipc) = PlexiApp::new_for_test(
        egui::Context::default(),
        std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
    );
    assert_eq!(app.display_version, env!("PLEXI_VERSION_TEXT"));
    assert!(app
        .display_version
        .ends_with(&format!("({})", env!("PLEXI_SHORT_COMMIT"))));
    assert!(!app.display_version.contains("999"));
}

#[test]
fn running_build_identity_is_available_while_hidden_and_ignores_profile_tag() {
    let mut h = crate::testing::HostHarness::new();
    std::fs::write(
        crate::config::config_dir().join("installed_tag"),
        "v999.0.0",
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let reply = dir.path().join("build.json");
    h.inject_ipc(crate::protocol::HostRequest::GetBuildInfo {
        response_file: reply.to_string_lossy().into_owned(),
    });
    h.hidden_frame();
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(reply).unwrap()).unwrap();
    assert_eq!(value["build_id"], env!("PLEXI_BUILD_ID"));
    assert_ne!(value["tag"], "v999.0.0");
}

#[test]
#[ignore = "requires-pty"]
fn file_browser_fresh_open_tracks_terminal_cd() {
    let _shell = crate::host::shell::set_test_shell_override("/bin/sh");
    let mut h = crate::testing::HostHarness::new();
    let root = h.workspace_root();
    h.app.set_context_root(root.clone(), None);
    let terminal = h.add_focused_terminal();
    let _unrelated = h.add_focused_terminal();
    h.focus_pane(terminal);
    for name in ["first", "second"] {
        let directory = root.join(name);
        std::fs::create_dir(&directory).unwrap();
        let marker = directory.join("ready");
        h.terminal_backend(terminal)
            .process_command(egui_term::BackendCommand::Write(
                format!(
                    "cd '{}' && touch '{}'\n",
                    directory.display(),
                    marker.display()
                )
                .into_bytes(),
            ));
        let deadline = std::time::Instant::now()
            + crate::testing::load_aware_timeout(std::time::Duration::from_secs(10));
        while !marker.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "terminal did not execute cd"
            );
            h.run_frames(1);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        h.focus_pane(terminal);
        let pid = h.terminal_backend(terminal).child_pid();
        assert_eq!(
            crate::host::shell::get_pid_cwd_fresh(pid)
                .unwrap()
                .canonicalize()
                .unwrap(),
            directory.canonicalize().unwrap(),
            "shell must remain in the executed cd directory"
        );
        h.app.open_file_browser();
        h.run_frames(3);
        // Overlay replacement retains the invoking terminal slot. The harness
        // seed is also a FileBrowser runtime, so never pick an arbitrary runtime.
        let browser = h.app.windows[0].panes[&terminal].as_view().unwrap();
        assert_eq!(browser.manifest_id, "file_browser");
        assert_eq!(
            browser.workspace_root.canonicalize().unwrap(),
            directory.canonicalize().unwrap()
        );
        h.app.open_file_browser(); // focused Explorer toggles closed, restoring the terminal
        h.focus_pane(terminal);
    }
}

#[test]
fn file_browser_fallback_uses_explicit_context_not_active_router() {
    let mut h = crate::testing::HostHarness::new();
    let root = h.workspace_root();
    let caller_context = h.app.windows[0].context_id;
    let caller = h.add_test_pane();
    let other = tempfile::tempdir().unwrap();
    h.app.router.push(crate::host::context::Context {
        name: "Other".into(),
        root: other.path().to_path_buf(),
        description: None,
        context_id: caller_context + 100000,
        parent_id: None,
        depth: 0,
        parked: false,
    });
    h.app.router.set_active(1);
    for pane in [Some(caller), Some(u64::MAX), None] {
        assert_eq!(
            h.app.file_browser_start_directory(pane, caller_context),
            root
        );
    }
}

#[test]
#[ignore = "requires-pty"]
fn file_browser_unavailable_terminal_directory_falls_back() {
    let _shell = crate::host::shell::set_test_shell_override("/bin/sh");
    let mut h = crate::testing::HostHarness::new();
    let root = h.workspace_root();
    let context = h.app.windows[0].context_id;
    let terminal = h.add_focused_terminal();
    let directory = root.join("removed");
    std::fs::create_dir(&directory).unwrap();
    let marker = root.join("ready");
    h.terminal_backend(terminal)
        .process_command(egui_term::BackendCommand::Write(
            format!(
                "cd '{}' && touch '{}'\n",
                directory.display(),
                marker.display()
            )
            .into_bytes(),
        ));
    let deadline = std::time::Instant::now()
        + crate::testing::load_aware_timeout(std::time::Duration::from_secs(10));
    while !marker.exists() {
        assert!(std::time::Instant::now() < deadline);
        h.run_frames(1);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    std::fs::remove_dir(&directory).unwrap();
    assert_eq!(
        h.app.file_browser_start_directory(Some(terminal), context),
        root
    );
    h.app.windows[0]
        .panes
        .get_mut(&terminal)
        .unwrap()
        .as_terminal_mut()
        .unwrap()
        .exited = true;
    assert_eq!(
        h.app.file_browser_start_directory(Some(terminal), context),
        root
    );
}

#[test]
fn external_spawn_uses_supplied_cwd_for_plugin_workspace() {
    let mut h = crate::testing::HostHarness::new();
    let expected = h.workspace_root().join("external-caller");
    std::fs::create_dir(&expected).unwrap();
    h.inject_ipc(crate::protocol::HostRequest::SpawnPane {
        type_id: "file_browser".into(),
        layout: Some("split_h".into()),
        args: vec![],
        from_pane_id: None,
        request_id: None,
        response_file: None,
        ephemeral: false,
        cwd: Some(expected.to_string_lossy().into_owned()),
        no_focus: false,
        path: None,
        workspace_root: None,
        target_context: None,
        context_name: None,
        name: None,
        agent_cmd: None,
        boot_timeout_secs: None,
        worktree: None,
        result_schema: None,
    });
    h.hidden_frame();
    let browser = h.app.windows[0]
        .panes
        .values()
        .find_map(|pane| {
            pane.as_view()
                .filter(|app| app.manifest_id == "file_browser")
        })
        .expect("external plugin spawned");
    assert_eq!(browser.workspace_root, expected);
}

#[test]
#[ignore = "requires-pty"]
fn file_browser_spawn_binds_caller_before_cross_context_focus_redirect() {
    let _shell = crate::host::shell::set_test_shell_override("/bin/sh");
    let mut h = crate::testing::HostHarness::new();
    let terminal = h.add_focused_terminal();
    let expected = h.workspace_root().join("caller-directory");
    let unrelated_directory = h.workspace_root().join("focused-directory");
    let unrelated = h.add_focused_terminal();
    for (pane, directory) in [(terminal, &expected), (unrelated, &unrelated_directory)] {
        std::fs::create_dir(directory).unwrap();
        let marker = directory.join("ready");
        h.terminal_backend(pane)
            .process_command(egui_term::BackendCommand::Write(
                format!(
                    "cd '{}' && touch '{}'\n",
                    directory.display(),
                    marker.display()
                )
                .into_bytes(),
            ));
        let deadline = std::time::Instant::now()
            + crate::testing::load_aware_timeout(std::time::Duration::from_secs(10));
        while !marker.exists() {
            assert!(std::time::Instant::now() < deadline);
            h.run_frames(1);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    h.focus_pane(unrelated);
    let caller_context = h.app.windows[0].context_id;
    let other_root = tempfile::tempdir().unwrap();
    let other_context = caller_context + 100000;
    h.app.router.push(crate::host::context::Context {
        name: "Other".into(),
        root: other_root.path().to_path_buf(),
        description: None,
        context_id: other_context,
        parent_id: None,
        depth: 0,
        parked: false,
    });
    h.app.windows.push(crate::host::context::Window {
        name: "Other".into(),
        path: other_root.path().to_path_buf(),
        tree: egui_tiles::Tree::empty("explorer_other"),
        panes: Default::default(),
        focused_pane: None,
        zoomed_pane: None,
        grid_x: 0,
        grid_y: 1,
        window_id: 90001,
        context_id: other_context,
        tree_history: Default::default(),
    });
    h.app.active_window = 1;
    h.app.router.set_active(1);
    h.inject_ipc(crate::protocol::HostRequest::SpawnPane {
        type_id: "file_browser".into(),
        layout: Some("split_h".into()),
        args: vec![],
        from_pane_id: Some(terminal),
        request_id: None,
        response_file: None,
        ephemeral: false,
        cwd: None,
        no_focus: true,
        path: None,
        workspace_root: None,
        target_context: None,
        context_name: None,
        name: None,
        agent_cmd: None,
        boot_timeout_secs: None,
        worktree: None,
        result_schema: None,
    });
    h.hidden_frame();
    let browser = h.app.windows[0]
        .panes
        .values()
        .find_map(|pane| {
            pane.as_view()
                .filter(|app| app.manifest_id == "file_browser")
        })
        .unwrap();
    assert_eq!(
        browser.workspace_root.canonicalize().unwrap(),
        expected.canonicalize().unwrap()
    );
    assert_eq!(h.app.active_window, 1);
    assert_eq!(h.app.router.active().context_id, other_context);
    assert!(h.app.windows[1].panes.is_empty());
    let browser_id = browser.id;
    h.app.active_window = 0;
    h.app.router.set_active(0);
    h.focus_pane(unrelated);
    h.run_frames(3);
    let runtime_cwd = |h: &crate::testing::HostHarness| {
        let state = h.app.windows[0].panes[&browser_id]
            .as_view()
            .unwrap()
            .runtime
            .serialize_state()
            .unwrap();
        std::path::PathBuf::from(state["cwd"].as_str().unwrap())
            .canonicalize()
            .unwrap()
    };
    assert_eq!(
        runtime_cwd(&h),
        expected.canonicalize().unwrap(),
        "unrelated focus must not retarget Explorer"
    );
    h.focus_pane(browser_id);
    h.run_frames(3);
    assert_eq!(
        runtime_cwd(&h),
        expected.canonicalize().unwrap(),
        "Explorer focus must preserve its caller's directory"
    );

    let next = h.workspace_root().join("caller-after-open");
    std::fs::create_dir(&next).unwrap();
    h.terminal_backend(terminal)
        .process_command(egui_term::BackendCommand::Write(
            format!("cd '{}'\n", next.display()).into_bytes(),
        ));
    let deadline = std::time::Instant::now()
        + crate::testing::load_aware_timeout(std::time::Duration::from_secs(10));
    while runtime_cwd(&h) != next.canonicalize().unwrap() {
        assert!(
            std::time::Instant::now() < deadline,
            "Explorer must still follow its own linked terminal"
        );
        h.run_frames(1);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    h.app.windows[0]
        .panes
        .get_mut(&terminal)
        .unwrap()
        .as_terminal_mut()
        .unwrap()
        .exited = true;
    h.focus_pane(unrelated);
    h.run_frames(3);
    assert_eq!(
        runtime_cwd(&h),
        next.canonicalize().unwrap(),
        "dead linked owner must not fall back to an unrelated terminal"
    );
}

#[test]
#[ignore = "requires-pty"]
fn file_browser_restore_preserves_saved_navigation_without_caller_link() {
    let _shell = crate::host::shell::set_test_shell_override("/bin/sh");
    let mut h = crate::testing::HostHarness::new();
    let root = h.workspace_root();
    let saved = root.join("saved-navigation");
    std::fs::create_dir(&saved).unwrap();
    std::fs::write(saved.join("one.txt"), "one").unwrap();
    std::fs::write(saved.join("two.txt"), "two").unwrap();
    let browser_id = h.add_test_pane();
    let state = serde_json::json!({"cwd": saved, "selected": 1});
    let restored = crate::pane_ops::create::restore_builtin_view_pane(
        "file_browser",
        browser_id,
        root,
        Some(&state),
    )
    .unwrap();
    h.app.windows[0].replace_pane_for_test(browser_id, restored);
    let _unrelated = h.add_focused_terminal();
    h.run_frames(3);
    let app = h.app.windows[0].panes[&browser_id].as_view().unwrap();
    assert!(app.linked_pane_id.is_none());
    let after = app.runtime.serialize_state().unwrap();
    assert_eq!(after["cwd"], state["cwd"]);
    assert_eq!(after["selected"], 1);
}

#[test]
#[ignore = "requires-pty"]
fn file_browser_explicit_directory_artifact_keeps_requested_root() {
    let _shell = crate::host::shell::set_test_shell_override("/bin/sh");
    for surface in ["artifact", "generic", "ipc", "generic-default"] {
        let mut h = crate::testing::HostHarness::new();
        let sender = h.add_test_pane();
        let terminal = h.add_focused_terminal();
        let directory = h.workspace_root().join("explicit-directory");
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("requested.txt"), "requested").unwrap();
        if surface == "generic-default" {
            let marker = directory.join("ready");
            h.terminal_backend(terminal)
                .process_command(egui_term::BackendCommand::Write(
                    format!(
                        "cd '{}' && touch '{}'\n",
                        directory.display(),
                        marker.display()
                    )
                    .into_bytes(),
                ));
            let deadline = std::time::Instant::now()
                + crate::testing::load_aware_timeout(std::time::Duration::from_secs(10));
            while !marker.exists() {
                assert!(std::time::Instant::now() < deadline);
                h.run_frames(1);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            h.focus_pane(terminal);
        }
        match surface {
            "generic-default" => {
                h.app
                    .launch_view_by_id_with_layout(
                        "file_browser",
                        Some("split_h".into()),
                        &[],
                        None,
                    )
                    .unwrap();
            }
            "artifact" => h.app.dispatch_open_artifact(
                sender,
                directory.display().to_string(),
                crate::protocol::ArtifactOpenMode::OpenInPane,
            ),
            "generic" => {
                h.app
                    .launch_view_by_id_with_layout(
                        "file_browser",
                        Some("split_h".into()),
                        &[],
                        Some(directory.clone()),
                    )
                    .unwrap();
            }
            _ => {
                h.inject_ipc(crate::protocol::HostRequest::SpawnPane {
                    type_id: "file_browser".into(),
                    layout: Some("split_h".into()),
                    args: vec![],
                    from_pane_id: Some(terminal),
                    request_id: None,
                    response_file: None,
                    ephemeral: false,
                    cwd: Some(directory.display().to_string()),
                    no_focus: false,
                    path: None,
                    workspace_root: None,
                    target_context: None,
                    context_name: None,
                    name: None,
                    agent_cmd: None,
                    boot_timeout_secs: None,
                    worktree: None,
                    result_schema: None,
                });
                h.hidden_frame();
            }
        }
        if surface == "generic-default" {
            let browser = h.app.windows[0]
                .panes
                .values()
                .find_map(|pane| {
                    pane.as_view()
                        .filter(|app| app.manifest_id == "file_browser")
                })
                .unwrap();
            assert_eq!(
                browser.workspace_root.canonicalize().unwrap(),
                directory.canonicalize().unwrap(),
                "implicit generic caller must bind before first frame"
            );
        }
        h.run_frames(3);
        let browser = h.app.windows[0]
            .panes
            .values()
            .find_map(|pane| {
                pane.as_view()
                    .filter(|app| app.manifest_id == "file_browser")
            })
            .unwrap();
        let state = browser.runtime.serialize_state().unwrap();
        assert_eq!(
            std::path::PathBuf::from(state["cwd"].as_str().unwrap())
                .canonicalize()
                .unwrap(),
            directory.canonicalize().unwrap(),
            "{surface} explicit directory must survive frame synchronization"
        );
    }
}

fn fire_and_forget_set_agent_state() -> String {
    serde_json::to_string(&crate::protocol::HostRequest::SetAgentState {
        pane_id: 1,
        state: crate::protocol::AgentState::Working,
        agent: "test-agent".into(),
        detail: None,
        session_id: None,
        event: None,
        blocked_reason: None,
    })
    .unwrap()
}

/// Repro for the beta-log `pane_ipc` missing-identity warning: a CLI that
/// writes and disconnects before the host's accept loop runs leaves no peer
/// pid to read, so the request carries no verified sender.
#[cfg(target_os = "macos")]
#[test]
fn peer_pid_is_lost_when_the_client_disconnects_before_accept() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.sock");
    let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    {
        let mut client = std::os::unix::net::UnixStream::connect(&path).unwrap();
        std::io::Write::write_all(&mut client, b"{}\n").unwrap();
    }
    let (server, _) = listener.accept().unwrap();
    assert!(resolve_socket_peer_pid(&server).is_none());
}

#[test]
fn host_acks_fire_and_forget_requests_that_consume_peer_identity() {
    let wake = std::sync::Arc::new(crate::app::ui_mailbox::EguiWake::new(
        egui::Context::default(),
    ));
    let (mailbox, _rx) = crate::app::ui_mailbox::UiMailbox::channel(wake, "test");
    let profile = tempfile::tempdir().unwrap();
    let mut acked = Vec::new();
    handle_socket_line_in(profile.path(), &fire_and_forget_set_agent_state(), &mailbox, None, &mut acked);
    assert_eq!(acked, PEER_IDENTITY_ACK);
    // Every request is admitted by its caller, so every fire-and-forget
    // client waits for the witness (stint 1285).
    let mut wake_ack = Vec::new();
    handle_socket_line_in(profile.path(), r#"{"type":"wake"}"#, &mailbox, None, &mut wake_ack);
    assert_eq!(wake_ack, PEER_IDENTITY_ACK);
}

/// The client holds the connection until the host acks, so the accept loop
/// can read its pid even when it runs late.
#[cfg(unix)]
#[test]
fn client_waits_for_the_identity_ack_so_a_late_accept_still_sees_its_pid() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.sock");
    let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    let server = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(100));
        let (mut stream, _) = listener.accept().unwrap();
        let pid = resolve_socket_peer_pid(&stream);
        std::io::Write::write_all(&mut stream, PEER_IDENTITY_ACK).unwrap();
        pid
    });
    let line = format!("{}\n", fire_and_forget_set_agent_state());
    let mut client = std::os::unix::net::UnixStream::connect(&path).unwrap();
    std::io::Write::write_all(&mut client, line.as_bytes()).unwrap();
    crate::cli::await_peer_identity_ack(&mut client, &line);
    drop(client);
    assert!(server.join().unwrap().is_some());
}

/// Through a real socket: the listener's kernel credential walk reaches the
/// root for this process, travels with the request in its envelope, and a
/// wire `peer` field is not accepted (stint 1285).
#[cfg(unix)]
#[test]
fn file_open_peer_walk_is_stamped_by_the_listener_not_the_wire() {
    use std::io::Write as _;
    let (server, client) = std::os::unix::net::UnixStream::pair().unwrap();
    let pid = super::super::resolve_socket_peer_pid(&server).expect("kernel peer pid");
    assert_eq!(pid, std::process::id());
    let walk = crate::broker::caller::PeerWitness::capture(pid);
    assert_eq!(walk.hops.first().map(|hop| hop.pid), Some(pid));
    assert_eq!(walk.end, crate::broker::caller::AncestryEnd::ReachedRoot, "{walk:?}");

    let wake = std::sync::Arc::new(crate::app::ui_mailbox::EguiWake::new(
        egui::Context::default(),
    ));
    let (mailbox, rx) = crate::app::ui_mailbox::UiMailbox::channel(wake.clone(), "file-open");
    let (subscribe, _s) = crate::app::ui_mailbox::UiMailbox::<
        crate::host::event_subscriptions::HostSubscribeRequest,
    >::channel(wake.clone(), "subscribe");
    let (publish, _p) = crate::app::ui_mailbox::UiMailbox::<
        crate::host::event_subscriptions::HostPublishRequest,
    >::channel(wake, "publish");
    drop(client);
    drop(server);
    let profile = tempfile::tempdir().unwrap();
    let forged = serde_json::json!({
        "type": "file_open", "path": "a.txt", "response_file": "x",
        "peer": {"pids": [1], "end": "ReachedRoot"},
    });
    let honest = serde_json::json!({"type": "file_open", "path": "a.txt", "response_file": "y"});
    // One request per connection, as the CLI sends them.
    for line in [forged, honest] {
        let (server, mut client) = std::os::unix::net::UnixStream::pair().unwrap();
        writeln!(client, "{line}").unwrap();
        drop(client);
        let (profile_path, stamped) = (profile.path().to_path_buf(), walk.clone());
        let (mailbox, subscribe, publish) = (mailbox.clone(), subscribe.clone(), publish.clone());
        std::thread::spawn(move || {
            super::super::handle_socket_connection(
                &profile_path,
                server,
                mailbox,
                subscribe,
                publish,
                Some(stamped),
            )
        })
        .join()
        .unwrap();
    }
    let mut seen = Vec::new();
    while let Ok(message) = rx.try_recv() {
        seen.push(message);
    }
    assert_eq!(seen.len(), 1, "the forged line is refused: {seen:?}");
    match &seen[0] {
        crate::app::socket_caller::PaneIpcMessage::Request {
            ingress: crate::app::socket_caller::Ingress::Socket { peer, .. },
            request,
        } if matches!(**request, crate::protocol::HostRequest::FileOpen { .. }) => {
            let crate::protocol::HostRequest::FileOpen { response_file, .. } = &**request else {
                unreachable!()
            };
            assert_eq!(response_file, "y");
            assert_eq!(peer.as_ref(), Some(&walk));
        }
        other => panic!("expected a socket file_open, got {other:?}"),
    }
}

/// Codex 3290 r2/r3, through the real follow stream: a pane following its
/// own context's workspace gets the projection of that workspace only (no
/// pending request filed for another workspace), and a pane naming another
/// workspace is refused before anything is served.
#[cfg(unix)]
#[test]
fn dashboard_follow_stream_serves_only_the_callers_workspace() {
    use std::io::{BufRead as _, Write as _};
    let _dashboard =
        crate::release::set_test_experiments(&[crate::release::ReleaseFeature::AgentsDashboard]);
    let mut h = crate::testing::HostHarness::new();
    let pane = h.add_focused_terminal();
    let root = h.workspace_root();
    h.app.set_context_root(root.clone(), None);
    let elsewhere = tempfile::tempdir().unwrap();
    // A pane's dashboard shows its own pending requests (stint 1743), so the
    // asks are the pane's own: one in its workspace, one elsewhere.
    let caller = h.app.pane_principal(pane).unwrap();
    let monitor =
        crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir()).unwrap();
    for (workspace, marker) in [(root.clone(), "own-1285"), (elsewhere.path().to_path_buf(), "foreign-1285")] {
        let admission = monitor.admit(crate::broker::gate::AdmitRequest {
            call_id: &format!("follow-{marker}"),
            tool: "agents.review",
            input_json: &serde_json::json!({"marker": marker}).to_string(),
            principal: &crate::broker::principal::Principal::Pane(caller.clone()),
            actor_scope: crate::broker::ActorScope::Workspace,
            trust_origin: "host",
            workspace_root: &crate::platform::path::canonical_or_self(&workspace),
            context_id: 0,
            package_id: "agents",
            instance_id: 0,
            target_type: crate::broker::TargetType::HostTool,
        });
        assert!(matches!(admission, crate::broker::gate::Admission::Required { .. }));
    }
    let peer = crate::broker::caller::PeerWitness::for_live_pids(&[h.shell_pid(pane)]);
    let follow = |h: &mut crate::testing::HostHarness, workspace: &std::path::Path| {
        let ctx = egui::Context::default();
        let wake = std::sync::Arc::new(crate::app::ui_mailbox::EguiWake::new(ctx));
        let (subscribe, _subscribe_rx) = crate::app::ui_mailbox::UiMailbox::<
            crate::host::event_subscriptions::HostSubscribeRequest,
        >::channel(wake.clone(), "event_subscribe");
        let (publish, _publish_rx) = crate::app::ui_mailbox::UiMailbox::<
            crate::host::event_subscriptions::HostPublishRequest,
        >::channel(wake, "event_publish");
        let (server, mut client) = std::os::unix::net::UnixStream::pair().unwrap();
        let line = serde_json::json!({"type": "command_view_follow", "workspace": workspace});
        writeln!(client, "{line}").unwrap();
        let mailbox = h.ipc_tx.clone();
        let peer = peer.clone();
        let profile = tempfile::tempdir().unwrap();
        let profile_path = profile.path().to_path_buf();
        // The stream thread reads the same profile as the host (the test
        // profile is per thread).
        let host_profile = crate::config::config_dir();
        let worker = std::thread::spawn(move || {
            let _profile = crate::config::set_test_profile_dir(host_profile);
            handle_socket_connection(&profile_path, server, mailbox, subscribe, publish, Some(peer))
        });
        client
            .set_read_timeout(Some(std::time::Duration::from_millis(50)))
            .unwrap();
        let mut reader = std::io::BufReader::new(client.try_clone().unwrap());
        let mut lines = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while lines.len() < 2 && std::time::Instant::now() < deadline {
            h.run_frames(1);
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => lines.push(serde_json::from_str::<serde_json::Value>(&line).unwrap()),
                Err(_) => {}
            }
        }
        drop(reader);
        drop(client);
        h.run_frames(1);
        worker.join().unwrap();
        drop(profile);
        lines
    };
    let own = follow(&mut h, &root);
    assert_eq!(own.len(), 2, "{own:?}");
    assert_eq!(own[0]["type"], "subscribed", "{own:?}");
    let projection = own[1].to_string();
    assert!(projection.contains("own-1285"), "{projection}");
    assert!(!projection.contains("foreign-1285"), "another workspace leaked: {projection}");
    let foreign = follow(&mut h, elsewhere.path());
    assert_eq!(foreign[0]["type"], "error", "{foreign:?}");
    assert_eq!(foreign[0]["error_code"], "permission_denied", "{foreign:?}");
    assert_eq!(foreign.len(), 1, "nothing served after the refusal: {foreign:?}");
}
