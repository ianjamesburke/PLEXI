use crate::app::ui_mailbox::{EguiWake, UiMailbox};
use crate::host::event_subscriptions::ConsentChoice;
use crate::testing::HostHarness;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

struct Connection {
    socket: UnixStream,
    lines: mpsc::Receiver<String>,
    server: Option<std::thread::JoinHandle<()>>,
}

impl Connection {
    fn open(h: &mut HostHarness, request: Value) -> Self {
        let wake = Arc::new(EguiWake::new(h.app.ctx.clone()));
        let (subscribe, rx) = UiMailbox::channel(wake.clone(), "consumer-test");
        h.app.event_subscribe_rx = rx;
        let (publish, rx) = UiMailbox::channel(wake, "consumer-publish-test");
        h.app.event_publish_rx = rx;
        let ipc = h.ipc_tx.clone();
        let (mut socket, server) = UnixStream::pair().unwrap();
        let thread = std::thread::spawn(move || {
            crate::app::handle_socket_connection(server, ipc, subscribe, publish, None);
        });
        writeln!(socket, "{request}").unwrap();
        let input = socket.try_clone().unwrap();
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(input).lines() {
                match line {
                    Ok(line) => {
                        if tx.send(line).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            socket,
            lines,
            server: Some(thread),
        }
    }

    fn next(&self, h: &mut HostHarness) -> Value {
        let until = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(5));
        loop {
            h.hidden_frame();
            while let Some(consent) = h.app.pending_event_consents.pop_front() {
                h.app.host_subscriptions.resolve_consent(
                    consent,
                    ConsentChoice::AllowOnce,
                    &crate::config::config_dir(),
                );
            }
            if let Ok(line) = self.lines.try_recv() {
                return serde_json::from_str(&line).unwrap();
            }
            assert!(
                Instant::now() < until,
                "host did not return a consumer record"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let _ = self.socket.shutdown(std::net::Shutdown::Both);
        if let Some(thread) = self.server.take() {
            thread.join().unwrap();
        }
    }
}

fn report(h: &mut HostHarness, pane: u64, state: &str, event: &str) {
    h.inject_ipc(
        serde_json::from_value(json!({
            "type":"set_agent_state", "pane_id":pane, "agent":"test-provider",
            "state":state, "event":event,
        }))
        .unwrap(),
    );
    h.hidden_frame();
}

#[test]
fn pane_consumer_wait_matches_current_idle_while_hidden() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    report(&mut h, pane, "idle", "Stop");
    let connection = Connection::open(
        &mut h,
        json!({
            "type":"pane_lifecycle_wait", "pane_id":pane, "until":"idle",
            "timeout":1.0, "from_pane_id":pane,
        }),
    );
    let reply = connection.next(&mut h);
    assert_eq!(
        reply["type"], "event",
        "wait must return the matched event: {reply}"
    );
    assert_eq!(reply["payload"]["kind"], "agent_idle");
    assert_eq!(reply["payload"]["provenance"]["raw_event"], "Stop");
}

#[test]
fn pane_consumer_cli_surfaces_parse() {
    use clap::Parser;
    for args in [
        vec![
            "plexi",
            "pane",
            "wait",
            "7",
            "--until",
            "idle",
            "--timeout",
            "1",
        ],
        vec!["plexi", "pane", "events", "--follow", "--pane", "7"],
    ] {
        assert!(
            crate::cli::args::Cli::try_parse_from(&args).is_ok(),
            "missing CLI surface: {args:?}"
        );
    }
}

fn wait_request(pane: u64, caller: u64, predicate: &str, timeout: f64) -> Value {
    json!({"type":"pane_lifecycle_wait", "pane_id":pane, "from_pane_id":caller,
        "until":predicate, "timeout":timeout})
}

fn close(h: &mut HostHarness, pane: u64) {
    let (window, tile) = h.app.find_pane_in_any_window(pane).unwrap();
    h.app.close_tile(window, tile);
}

fn subscriptions(pane: u64) -> usize {
    crate::host::app_timeline::global()
        .lock()
        .unwrap()
        .subscriptions()
        .iter()
        .filter(|sub| sub.resource_id.as_deref() == Some(pane.to_string().as_str()))
        .count()
}

fn pump_until(h: &mut HostHarness, mut condition: impl FnMut(&HostHarness) -> bool) {
    let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(5));
    while !condition(h) {
        h.hidden_frame();
        assert!(Instant::now() < deadline, "host condition did not resolve");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn follow(h: &mut HostHarness, pane: u64) -> Connection {
    let connection = Connection::open(
        h,
        json!({"type":"pane_lifecycle_follow", "pane_id":pane, "from_pane_id":pane}),
    );
    assert_eq!(connection.next(h)["type"], "subscribed");
    connection
}

#[test]
fn pane_consumer_closed_pane_cannot_match_stale_idle() {
    let mut h = HostHarness::new();
    let caller = h.add_test_pane();
    let target = h.add_test_pane();
    report(&mut h, target, "idle", "Stop");
    close(&mut h, target);
    let connection = Connection::open(&mut h, wait_request(target, caller, "idle", 1.0));
    let reply = connection.next(&mut h);
    assert_eq!(
        reply["type"], "error",
        "closed pane must not remain ready: {reply}"
    );
}

#[test]
fn pane_consumer_retains_unknown_exit_after_close() {
    use crate::host::pane_lifecycle::{ExitStatus, PaneLifecycleEvent};
    let mut h = HostHarness::new();
    let caller = h.add_test_pane();
    let target = h.add_test_pane();
    h.app.emit_pane_lifecycle(
        target,
        PaneLifecycleEvent::Exited {
            status: ExitStatus::Unknown,
        },
    );
    close(&mut h, target);
    let connection = Connection::open(&mut h, wait_request(target, caller, "exited", 1.0));
    let reply = connection.next(&mut h);
    assert_eq!(reply["payload"]["kind"], "exited", "{reply}");
    assert_eq!(reply["payload"]["status"], "unknown");
}

#[test]
fn pane_consumer_failure_and_session_end_invalidate_idle() {
    for event in ["StopFailure", "SessionEnd", "UserPromptSubmit"] {
        let mut h = HostHarness::new();
        let pane = h.add_test_pane();
        report(&mut h, pane, "idle", "Stop");
        report(
            &mut h,
            pane,
            if event == "UserPromptSubmit" {
                "working"
            } else {
                "idle"
            },
            event,
        );
        let connection = Connection::open(&mut h, wait_request(pane, pane, "idle", 0.1));
        assert_eq!(
            connection.next(&mut h)["type"],
            "timeout",
            "stale idle matched after {event}"
        );
    }
}

#[test]
fn pane_consumer_follow_connections_do_not_steal_or_cancel_each_other() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    let first = follow(&mut h, pane);
    let second = follow(&mut h, pane);
    assert_eq!(subscriptions(pane), 2);
    report(&mut h, pane, "blocked", "PermissionRequest");
    let a = first.next(&mut h);
    let b = second.next(&mut h);
    assert_eq!(a["event_id"], b["event_id"]);
    assert_eq!(a["payload"]["reason"], "permission-prompt");
    assert_ne!(a["subscription_id"], b["subscription_id"]);
    drop(first);
    assert_eq!(subscriptions(pane), 1);
    report(&mut h, pane, "blocked", "UsageLimit");
    assert_eq!(second.next(&mut h)["payload"]["reason"], "usage-limit");
    drop(second);
    assert_eq!(subscriptions(pane), 0);
}

#[test]
fn pane_consumer_cancel_during_consent_releases_pending_state() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    let connection = Connection::open(&mut h, wait_request(pane, pane, "idle", 5.0));
    pump_until(&mut h, |h| !h.app.pending_event_consents.is_empty());
    drop(connection);
    h.hidden_frame();
    assert!(h.app.pending_event_consents.is_empty());
    assert_eq!(subscriptions(pane), 0);
}

#[test]
fn pane_consumer_denied_subscription_returns_no_record() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    report(&mut h, pane, "idle", "Stop");
    let connection = Connection::open(&mut h, wait_request(pane, pane, "idle", 2.0));
    pump_until(&mut h, |h| !h.app.pending_event_consents.is_empty());
    let consent = h.app.pending_event_consents.pop_front().unwrap();
    h.app.host_subscriptions.resolve_consent(
        consent,
        ConsentChoice::Deny,
        &crate::config::config_dir(),
    );
    assert_eq!(connection.next(&mut h)["type"], "error");
    assert_eq!(subscriptions(pane), 0);
}

#[test]
fn pane_consumer_follow_preserves_terminal_event_provenance() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    let connection = follow(&mut h, pane);
    for (raw, kind) in [
        ("Stop", "turn_finished"),
        ("StopFailure", "turn_failed"),
        ("SessionEnd", "session_ended"),
    ] {
        report(&mut h, pane, "idle", raw);
        let event = loop {
            let event = connection.next(&mut h);
            if event["payload"]["kind"] != "agent_idle" {
                break event;
            }
        };
        assert_eq!(event["payload"]["kind"], kind);
        assert_eq!(event["payload"]["provenance"]["raw_event"], raw);
    }
}

fn approve(h: &mut HostHarness) {
    while let Some(consent) = h.app.pending_event_consents.pop_front() {
        h.app.host_subscriptions.resolve_consent(
            consent,
            ConsentChoice::AllowOnce,
            &crate::config::config_dir(),
        );
    }
}

#[test]
fn pane_consumer_pending_wait_matches_transition_and_releases_subscription() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    let connection = Connection::open(&mut h, wait_request(pane, pane, "blocked", 5.0));
    pump_until(&mut h, |h| !h.app.pending_event_consents.is_empty());
    approve(&mut h);
    assert_eq!(subscriptions(pane), 1);
    report(&mut h, pane, "blocked", "PermissionRequest");
    let event = connection.next(&mut h);
    assert_eq!(event["payload"]["kind"], "agent_blocked");
    assert_eq!(event["payload"]["reason"], "permission-prompt");
    drop(connection);
    assert_eq!(subscriptions(pane), 0);
}

#[test]
fn pane_consumer_close_during_wait_returns_error_without_deadline_delay() {
    let mut h = HostHarness::new();
    let caller = h.add_test_pane();
    let pane = h.add_test_pane();
    let connection = Connection::open(&mut h, wait_request(pane, caller, "idle", 60.0));
    pump_until(&mut h, |h| !h.app.pending_event_consents.is_empty());
    approve(&mut h);
    close(&mut h, pane);
    assert_eq!(connection.next(&mut h)["type"], "error");
    drop(connection);
    assert_eq!(subscriptions(pane), 0);
}

fn another_context(h: &mut HostHarness, id: u64) -> u64 {
    let root = h.app.router.active().root.clone();
    h.app.router.push(crate::host::context::Context {
        name: "consumer context".into(),
        root: root.clone(),
        description: None,
        context_id: id,
        parent_id: None,
        depth: 0,
        parked: false,
    });
    h.app.windows.push(crate::host::context::Window {
        name: "consumer context".into(),
        path: root,
        tree: egui_tiles::Tree::empty(format!("consumer-{id}")),
        panes: std::collections::HashMap::new(),
        focused_pane: None,
        zoomed_pane: None,
        grid_x: 1,
        grid_y: 0,
        window_id: id,
        context_id: id,
    });
    let pane = h.add_test_pane();
    let (_, tile) = h.app.find_pane_in_any_window(pane).unwrap();
    h.app.windows[0].tree.tiles.remove(tile);
    let value = h.app.windows[0].panes.remove(&pane).unwrap();
    let win = &mut h.app.windows[1];
    win.panes.insert(pane, value);
    win.tree.root = Some(win.tree.tiles.insert_pane(pane));
    h.app.switch_workspace(1);
    pane
}

#[test]
fn pane_consumer_inactive_context_uses_caller_scope_and_rejects_other_context() {
    let mut h = HostHarness::new();
    let caller = h.add_test_pane();
    // Global timeline is shared by parallel harness tests; this unfiltered
    // subscription needs its own context as well as unique pane IDs.
    h.app.router.get_mut(0).context_id = caller + 200_000;
    h.app.windows[0].context_id = caller + 200_000;
    let other = another_context(&mut h, caller + 100_000);
    assert_ne!(
        h.app.windows[h.app.active_window].context_id,
        h.app.windows[0].context_id
    );
    report(&mut h, caller, "idle", "Stop");
    let wait = Connection::open(&mut h, wait_request(caller, caller, "idle", 5.0));
    assert_eq!(wait.next(&mut h)["payload"]["kind"], "agent_idle");
    drop(wait);
    report(&mut h, other, "idle", "Stop");
    let denied = Connection::open(&mut h, wait_request(other, caller, "idle", 5.0));
    assert_eq!(denied.next(&mut h)["type"], "error");
    drop(denied);
    let stream = Connection::open(
        &mut h,
        json!({"type":"pane_lifecycle_follow", "from_pane_id":caller}),
    );
    assert_eq!(stream.next(&mut h)["type"], "subscribed");
    report(&mut h, other, "blocked", "UsageLimit");
    report(&mut h, caller, "blocked", "PermissionRequest");
    let event = stream.next(&mut h);
    assert_eq!(event["pane_id"], caller, "other context leaked: {event}");
    assert_eq!(event["payload"]["reason"], "permission-prompt");
}

#[test]
fn pane_consumer_follow_filters_resource_and_timeout_cleans_up() {
    let mut h = HostHarness::new();
    let caller = h.add_test_pane();
    let other = h.add_test_pane();
    let stream = follow(&mut h, caller);
    report(&mut h, other, "blocked", "UsageLimit");
    report(&mut h, caller, "blocked", "PermissionRequest");
    assert_eq!(stream.next(&mut h)["pane_id"], caller);
    drop(stream);
    let wait = Connection::open(&mut h, wait_request(caller, caller, "idle", 0.1));
    assert_eq!(wait.next(&mut h)["type"], "timeout");
    drop(wait);
    assert_eq!(subscriptions(caller), 0);
}
