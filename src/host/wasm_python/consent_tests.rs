//! Stint 1489: a Python plugin's `net.http`, `fs.write` and `ai.query` are
//! ordinary broker requests. Each use is admitted by `PermissionMonitor` from
//! its grant store; an ask is a broker pending row the pane's prompt renders,
//! and a key press on that prompt settles exactly that row. One test per
//! finding of the Codex review (`.stint/runs/codex-3228/REPORT.md`).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;

use super::consent;
use crate::broker::consent::ARM_DELAY;
use crate::broker::gate::{
    ApprovalChoice, NeedsYouResolution, PermissionCaller, PermissionMonitor, PermissionMutation,
};
use crate::host::pane::{Pane, ViewRuntime};
use crate::spatial::tiling::PaneId;
use crate::testing::HostHarness;

const PLUGIN: &str = "consent-probe";

const PROBE: &str = r#"from plexi_sdk.effects import FileWrite, HttpFetch, RequestCapability, SetTitle
from plexi_sdk.events import CapabilityDenied, CapabilityGranted, FileWriteResult, HttpResponse
from plexi_sdk.ui import Column

seen = []

def init(size, args):
    return [SetTitle("sent"), INIT]

def note(text):
    seen.append(text)
    return [SetTitle(" ".join(seen))]

def update(event):
    if isinstance(event, HttpResponse):
        if event.refused:
            return note("refused")
        if event.error:
            return note("error")
        return note(f"http:{event.status}")
    if isinstance(event, FileWriteResult):
        return note("write:" + ("error" if event.error else "ok"))
    if isinstance(event, CapabilityGranted):
        return note(f"granted:{event.name}")
    if isinstance(event, CapabilityDenied):
        return note(f"denied:{event.name}")
    return []

def view():
    return Column([])
"#;

/// Write the probe plugin. `init` is the Python list body of effects the
/// plugin sends at launch.
fn write_probe(dir: &std::path::Path, capability: &str, hosts: &[&str], init: &str) {
    let hosts = hosts
        .iter()
        .map(|host| format!("\"{host}\""))
        .collect::<Vec<_>>()
        .join(", ");
    std::fs::write(
        dir.join("manifest.toml"),
        format!(
            "schema_version = 2\n\n[plugin]\nid = \"{PLUGIN}\"\nname = \"Consent Probe\"\nversion = \"0.1.0\"\n\n[plugin.capabilities]\ncapabilities = [\"{capability}\"]\nallowed_hosts = [{hosts}]\n\n[runtime]\nkind = \"python\"\nentry = \"main.py\"\n\n[[contributes.views]]\nid = \"{PLUGIN}.main\"\ndefault = true\n"
        ),
    )
    .expect("write manifest");
    std::fs::write(dir.join("main.py"), PROBE.replace("INIT", init)).expect("write main.py");
}

/// A loopback server that answers every request `200 ok` and records paths.
fn loopback_server() -> (u16, Arc<Mutex<Vec<String>>>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().expect("local addr").port();
    let hits = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&hits);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0u8; 4096];
            let read = stream.read(&mut buf).unwrap_or(0);
            let head = String::from_utf8_lossy(&buf[..read]).to_string();
            let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
            seen.lock().unwrap().push(path);
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
            );
        }
    });
    (port, hits)
}

fn hits(server: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    server.lock().unwrap().clone()
}

fn fetch(port: u16, path: &str) -> String {
    format!("HttpFetch(\"http://127.0.0.1:{port}{path}\")")
}

fn launch(h: &mut HostHarness, dir: &std::path::Path) -> PaneId {
    h.app.trust_plugin_path_for_test(&dir.to_string_lossy());
    h.app
        .launch_view_by_path_with_layout_no_review_modal(&dir.to_string_lossy(), None, None, &[])
        .expect("launch probe")
        .expect("probe pane id")
}

fn python(h: &mut HostHarness, pane: PaneId) -> &mut super::LivePythonPane {
    let window = h
        .app
        .windows
        .iter()
        .position(|window| window.panes.contains_key(&pane))
        .unwrap_or_else(|| panic!("pane {pane} is in no window"));
    match h.app.windows[window]
        .panes
        .get_mut(&pane)
        .and_then(Pane::as_view_mut)
        .map(|view| &mut view.runtime)
    {
        Some(ViewRuntime::Python(p)) => p,
        _ => panic!("pane {pane} is not a Python pane"),
    }
}

fn title(h: &mut HostHarness, pane: PaneId) -> String {
    python(h, pane).display_name()
}

fn wait_until(h: &mut HostHarness, what: &str, mut done: impl FnMut(&mut HostHarness) -> bool) {
    let start = std::time::Instant::now();
    while !done(h) {
        h.run_frames(1);
        assert!(
            start.elapsed() < crate::testing::load_aware_timeout(Duration::from_secs(30)),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// The pending id and tool the pane's prompt shows.
fn prompt(h: &mut HostHarness, pane: PaneId) -> Option<(String, String)> {
    python(h, pane)
        .pending_capability_prompt()
        .map(|(_, view)| (view.pending_request_id, view.tool))
}

fn wait_for_prompt(h: &mut HostHarness, pane: PaneId, capability: &str) -> String {
    wait_until(h, &format!("the {capability} prompt"), |h| {
        prompt(h, pane).is_some_and(|(_, tool)| tool == capability)
    });
    prompt(h, pane).expect("prompt").0
}

fn wait_for_title(h: &mut HostHarness, pane: PaneId, prefix: &str) -> String {
    let start = std::time::Instant::now();
    while !title(h, pane).contains(prefix) {
        h.run_frames(1);
        assert!(
            start.elapsed() < crate::testing::load_aware_timeout(Duration::from_secs(30)),
            "timed out waiting for a title containing {prefix:?}; title is {:?}, prompt {:?}",
            title(h, pane),
            prompt(h, pane)
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    title(h, pane)
}

/// Answer `pane`'s prompt the way a person does: focus the pane so its
/// prompt owns the keyboard, let it arm, then press the dialog's key on a
/// real frame. Enter is "Grant once", Shift+Enter "Always allow", Esc "Deny
/// once", Shift+Esc "Always deny".
fn press_on_prompt(h: &mut HostHarness, pane: PaneId, key: egui::Key, shift: bool) {
    h.focus_pane(pane);
    h.run_frames(2);
    assert!(
        python(h, pane).has_pending_capability_prompt(),
        "no prompt to answer"
    );
    python(h, pane).backdate_capability_guard(ARM_DELAY);
    let modifiers = if shift {
        egui::Modifiers::SHIFT
    } else {
        egui::Modifiers::NONE
    };
    h.press_key(key, modifiers);
    // Release it, so the next press of the same key is fresh, not a repeat.
    h.frame(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1280.0, 800.0),
        )),
        events: vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers,
        }],
        ..Default::default()
    });
}

fn monitor() -> Arc<PermissionMonitor> {
    PermissionMonitor::for_profile(&crate::config::config_dir()).expect("tests run as the host")
}

fn human() -> PermissionCaller {
    PermissionCaller {
        revoke_scope: crate::broker::gate::RevokeScope::Any,
        actor_id: "user:local".to_string(),
    }
}

/// A guest `http_request` handled as if the plugin had just sent it.
fn guest_fetch(h: &mut HostHarness, pane: PaneId, port: u16, path: &str) {
    python(h, pane).handle_message(json!({
        "type": "http_request",
        "request_id": format!("guest-{path}"),
        "method": "GET",
        "url": format!("http://127.0.0.1:{port}{path}"),
    }));
}

/// Finding 1. A and B wait on the same ask. "Deny once" in A, then "Always
/// allow" for B (the broker click the Needs-you bar makes), both before A
/// drains again: A's request is refused and never sent.
#[test]
fn deny_once_is_final_when_a_standing_allow_lands_before_the_next_drain() {
    let (port, server) = loopback_server();
    let dir = tempfile::tempdir().expect("plugin dir");
    write_probe(dir.path(), "net.http", &["127.0.0.1"], &fetch(port, "/x"));
    let mut h = HostHarness::new();
    let a = launch(&mut h, dir.path());
    let b = launch(&mut h, dir.path());
    wait_for_prompt(&mut h, a, "net.http");
    let b_pending = wait_for_prompt(&mut h, b, "net.http");

    press_on_prompt(&mut h, a, egui::Key::Escape, false);
    monitor()
        .approve_pending(&b_pending, ApprovalChoice::Always)
        .expect("always allow for B");

    assert_eq!(wait_for_title(&mut h, a, "refused"), "refused");
    assert_eq!(wait_for_title(&mut h, b, "http:"), "http:200");
    h.run_frames(5);
    assert_eq!(hits(&server), vec!["/x"], "only B's request was sent");
}

/// Finding 2. Two different requests wait; one "Grant once" sends exactly
/// the request it was shown for. The other still waits, and the same
/// request again asks.
#[test]
fn grant_once_admits_exactly_one_request() {
    let (port, server) = loopback_server();
    let dir = tempfile::tempdir().expect("plugin dir");
    write_probe(
        dir.path(),
        "net.http",
        &["127.0.0.1"],
        &format!("{}, {}", fetch(port, "/a"), fetch(port, "/b")),
    );
    let mut h = HostHarness::new();
    let pane = launch(&mut h, dir.path());
    wait_until(&mut h, "two asks", |h| {
        python(h, pane).consent.waiting.len() == 2
    });
    let first = wait_for_prompt(&mut h, pane, "net.http");
    let shown = python(&mut h, pane).consent.waiting[0].message.clone();

    press_on_prompt(&mut h, pane, egui::Key::Enter, false);
    assert_eq!(wait_for_title(&mut h, pane, "http:"), "http:200");
    h.run_frames(5);
    assert_eq!(hits(&server), vec!["/a"]);
    let (second, _) = prompt(&mut h, pane).expect("the second request still asks");
    assert_ne!(second, first);

    // The very request the grant was for, sent again.
    python(&mut h, pane).handle_message(shown);
    h.run_frames(5);
    assert_eq!(hits(&server), vec!["/a"], "the once grant is spent");
    assert_eq!(python(&mut h, pane).consent.waiting.len(), 2);
}

/// Finding 3. Grant once in A, "Always allow" in B, then revoke that grant:
/// A's next request asks and sends nothing.
#[test]
fn revoke_is_seen_after_a_once_grant() {
    let (port, server) = loopback_server();
    let dir = tempfile::tempdir().expect("plugin dir");
    write_probe(dir.path(), "net.http", &["127.0.0.1"], &fetch(port, "/x"));
    let mut h = HostHarness::new();
    let a = launch(&mut h, dir.path());
    wait_for_prompt(&mut h, a, "net.http");
    let shown = python(&mut h, a).consent.waiting[0].message.clone();
    press_on_prompt(&mut h, a, egui::Key::Enter, false);
    wait_for_title(&mut h, a, "http:");

    let b = launch(&mut h, dir.path());
    wait_for_prompt(&mut h, b, "net.http");
    press_on_prompt(&mut h, b, egui::Key::Enter, true);
    wait_for_title(&mut h, b, "http:");
    assert_eq!(hits(&server).len(), 2);

    let grant = monitor()
        .list_entries()
        .into_iter()
        .find(|entry| entry.kind == "allow" && entry.actor_id == PLUGIN)
        .expect("the always grant is listed");
    assert!(matches!(
        monitor().mutate_entry(&grant.id, "revoke", &human()),
        PermissionMutation::Applied { .. }
    ));

    // The very request A was granted once, sent again.
    python(&mut h, a).handle_message(shown);
    h.run_frames(5);
    assert_eq!(hits(&server).len(), 2, "nothing is sent after the revoke");
    assert!(
        python(&mut h, a).has_pending_capability_prompt(),
        "A asks again"
    );
}

/// Finding 4. "Always allow" is bound to the host list it showed: a wider
/// list, then an empty one, each ask again and show the new list.
#[test]
fn always_allow_is_bound_to_the_host_list() {
    let (port, server) = loopback_server();
    let dir = tempfile::tempdir().expect("plugin dir");
    write_probe(dir.path(), "net.http", &["127.0.0.1"], &fetch(port, "/x"));
    let mut h = HostHarness::new();
    let first = launch(&mut h, dir.path());
    wait_for_prompt(&mut h, first, "net.http");
    press_on_prompt(&mut h, first, egui::Key::Enter, true);
    wait_for_title(&mut h, first, "http:");
    assert!(
        monitor()
            .list_entries()
            .iter()
            .any(|entry| entry.kind == "allow"
                && entry.actor_id == PLUGIN
                && entry.resource_id.as_deref() == Some("127.0.0.1")),
        "the stored grant names the host list"
    );

    h.app.close_pane_by_id(first);
    h.run_frames(2);
    for (hosts, shown) in [
        (
            &["127.0.0.1", "localhost"][..],
            "Hosts: 127.0.0.1, localhost and their subdomains",
        ),
        (&[][..], "Hosts: any host (the manifest lists none)"),
    ] {
        write_probe(dir.path(), "net.http", hosts, &fetch(port, "/x"));
        let pane = launch(&mut h, dir.path());
        wait_for_prompt(&mut h, pane, "net.http");
        let (_, view) = python(&mut h, pane)
            .pending_capability_prompt()
            .expect("asks again");
        let lines = consent::prompt_lines(&view);
        assert!(lines.iter().any(|line| line == shown), "{lines:?}");
        h.run_frames(5);
        assert_eq!(hits(&server).len(), 1, "{hosts:?} sent nothing");
        h.app.close_pane_by_id(pane);
        h.run_frames(2);
    }
}

/// "Always allow" in one pane ends the same ask in another pane of the plugin.
#[test]
fn always_allow_in_one_pane_releases_the_same_ask_in_another() {
    let (port, server) = loopback_server();
    let dir = tempfile::tempdir().expect("plugin dir");
    write_probe(dir.path(), "net.http", &["127.0.0.1"], &fetch(port, "/x"));
    let mut h = HostHarness::new();
    let a = launch(&mut h, dir.path());
    let b = launch(&mut h, dir.path());
    wait_for_prompt(&mut h, a, "net.http");
    wait_for_prompt(&mut h, b, "net.http");

    press_on_prompt(&mut h, b, egui::Key::Enter, true);
    assert_eq!(wait_for_title(&mut h, a, "http:"), "http:200");
    assert_eq!(wait_for_title(&mut h, b, "http:"), "http:200");
    assert!(!python(&mut h, a).has_pending_capability_prompt());
    assert_eq!(hits(&server).len(), 2);
}

/// A remembered grant survives a restart and answers a new pane.
#[test]
fn always_allow_survives_restart_in_a_new_pane() {
    let (port, server) = loopback_server();
    let dir = tempfile::tempdir().expect("plugin dir");
    write_probe(dir.path(), "net.http", &["127.0.0.1"], &fetch(port, "/x"));
    let mut h = HostHarness::new();
    let first = launch(&mut h, dir.path());
    wait_for_prompt(&mut h, first, "net.http");
    press_on_prompt(&mut h, first, egui::Key::Enter, true);
    wait_for_title(&mut h, first, "http:");

    PermissionMonitor::reload_for_test(&crate::config::config_dir());
    let second = launch(&mut h, dir.path());
    assert_eq!(wait_for_title(&mut h, second, "http:"), "http:200");
    assert!(!python(&mut h, second).has_pending_capability_prompt());
    assert_eq!(hits(&server).len(), 2);
}

/// Finding 6. An "Always allow" whose audit cannot be written, and an
/// "Always deny" whose grant file cannot be sealed, keep the prompt open
/// with the error and run nothing. Choosing again once saving works decides.
#[test]
fn unsaved_always_keeps_the_prompt_and_runs_nothing() {
    let (port, server) = loopback_server();
    let dir = tempfile::tempdir().expect("plugin dir");
    write_probe(dir.path(), "net.http", &["127.0.0.1"], &fetch(port, "/x"));
    let mut h = HostHarness::new();
    let pane = launch(&mut h, dir.path());
    let pending = wait_for_prompt(&mut h, pane, "net.http");

    monitor().fail_audit(true);
    press_on_prompt(&mut h, pane, egui::Key::Enter, true);
    monitor().fail_audit(false);
    h.run_frames(5);
    let error = python(&mut h, pane)
        .consent
        .last_error
        .clone()
        .expect("the failure is kept");
    assert_eq!(error.0, pending);
    assert_eq!(
        prompt(&mut h, pane).map(|(id, _)| id),
        Some(pending.clone())
    );
    assert!(hits(&server).is_empty(), "nothing ran");

    {
        let _unsealable =
            crate::broker::host_key::UnreadableMacKey::acquire().delete_needs_prompt();
        press_on_prompt(&mut h, pane, egui::Key::Escape, true);
        h.run_frames(5);
        let error = python(&mut h, pane).consent.last_error.clone();
        assert!(
            error
                .as_ref()
                .is_some_and(|(id, text)| *id == pending && text.contains("not saved")),
            "{error:?}"
        );
        assert_eq!(
            prompt(&mut h, pane).map(|(id, _)| id),
            Some(pending.clone())
        );
    }

    press_on_prompt(&mut h, pane, egui::Key::Enter, true);
    assert_eq!(wait_for_title(&mut h, pane, "http:"), "http:200");
    assert!(python(&mut h, pane).consent.last_error.is_none());
}

/// Closing the pane retires its asks; nothing is granted.
#[test]
fn closing_the_pane_retires_its_asks() {
    let (port, _server) = loopback_server();
    let dir = tempfile::tempdir().expect("plugin dir");
    write_probe(dir.path(), "net.http", &["127.0.0.1"], &fetch(port, "/x"));
    let mut h = HostHarness::new();
    let pane = launch(&mut h, dir.path());
    let pending = wait_for_prompt(&mut h, pane, "net.http");
    let item = monitor().get_needs_you(&pending).expect("a Needs-you item");
    assert_eq!(item.details.and_then(|d| d.pane_id), Some(pane));

    h.app.close_pane_by_id(pane);
    h.run_frames(2);
    assert!(
        monitor().show_pending(&pending).is_none(),
        "the ask is gone"
    );
    assert_eq!(
        monitor()
            .get_needs_you(&pending)
            .and_then(|record| record.resolution),
        Some(NeedsYouResolution::Retired)
    );
    assert!(
        !monitor()
            .list_entries()
            .iter()
            .any(|entry| entry.kind == "allow" && entry.actor_id == PLUGIN),
        "closing grants nothing"
    );
}

/// The ask shows both as the in-pane prompt and as an open Needs-you item.
/// One key press settles it once: one admission, one sent request.
#[test]
fn one_keypress_resolves_one_ask() {
    let (port, server) = loopback_server();
    let dir = tempfile::tempdir().expect("plugin dir");
    write_probe(dir.path(), "net.http", &["127.0.0.1"], &fetch(port, "/x"));
    let mut h = HostHarness::new();
    let pane = launch(&mut h, dir.path());
    let pending = wait_for_prompt(&mut h, pane, "net.http");
    assert!(
        crate::app::attention_queue::open_needs_you_items()
            .iter()
            .any(|item| item.id == pending),
        "the attention bar lists the ask"
    );

    press_on_prompt(&mut h, pane, egui::Key::Enter, false);
    assert_eq!(wait_for_title(&mut h, pane, "http:"), "http:200");
    h.run_frames(5);
    assert_eq!(hits(&server).len(), 1);
    assert_eq!(
        monitor()
            .get_needs_you(&pending)
            .and_then(|record| record.resolution),
        Some(NeedsYouResolution::Approved)
    );
    assert!(!crate::app::attention_queue::open_needs_you_items()
        .iter()
        .any(|item| item.id == pending));
    assert!(!python(&mut h, pane).has_pending_capability_prompt());
}

/// "Always deny" is stored; the refusal names the command that allows it, and
/// allowing it there is seen by the running plugin's next request.
#[test]
fn always_deny_names_the_allow_command_and_allowing_applies_live() {
    let capture = crate::testing::test_log::capture();
    let (port, server) = loopback_server();
    let dir = tempfile::tempdir().expect("plugin dir");
    write_probe(dir.path(), "net.http", &["127.0.0.1"], &fetch(port, "/x"));
    let mut h = HostHarness::new();
    let pane = launch(&mut h, dir.path());
    wait_for_prompt(&mut h, pane, "net.http");
    press_on_prompt(&mut h, pane, egui::Key::Escape, true);
    wait_for_title(&mut h, pane, "refused");

    guest_fetch(&mut h, pane, port, "/y");
    let deny = monitor()
        .list_entries()
        .into_iter()
        .find(|entry| entry.kind == "deny" && entry.actor_id == PLUGIN)
        .expect("the denial is listed");
    assert_eq!(deny.resource_id.as_deref(), Some("127.0.0.1"));
    let expected = format!(
        "net.http refused: net.http is denied for {PLUGIN}. To allow it, use the Permissions \
         plugin or run `plexi permissions allow {}`.",
        deny.id
    );
    assert!(
        capture.lines().iter().any(|line| line.ends_with(&expected)),
        "no refusal naming the allow command"
    );
    assert!(hits(&server).is_empty());

    // The socket files the widening; the desktop approval applies it.
    let PermissionMutation::NeedsYou { needs_you_id, .. } =
        monitor().mutate_entry(&deny.id, "allow", &human())
    else {
        panic!("a socket allow files a Needs-you item");
    };
    guest_fetch(&mut h, pane, port, "/still-denied");
    assert!(hits(&server).is_empty(), "filing the request grants nothing");
    monitor()
        .resolve_needs_you_from(
            &needs_you_id,
            true,
            crate::broker::gate::ResolveSurface::Desktop,
        )
        .expect("the desktop approves");
    guest_fetch(&mut h, pane, port, "/z");
    wait_until(&mut h, "the allowed request", |_| !hits(&server).is_empty());
    assert_eq!(hits(&server), vec!["/z"]);
}

/// `fs.write` takes the same gate: the write waits on the prompt and runs once
/// allowed.
#[test]
fn fs_write_asks_in_the_pane_and_writes_once_allowed() {
    let dir = tempfile::tempdir().expect("plugin dir");
    write_probe(
        dir.path(),
        "fs.write",
        &[],
        "FileWrite(\"consent-out.txt\", b\"hi\")",
    );
    let mut h = HostHarness::new();
    let pane = launch(&mut h, dir.path());
    wait_for_prompt(&mut h, pane, "fs.write");
    h.run_frames(5);
    assert_eq!(title(&mut h, pane), "sent");
    let target = python(&mut h, pane)
        .config
        .workspace_root
        .join("consent-out.txt");
    assert!(!target.exists(), "nothing is written before the answer");

    press_on_prompt(&mut h, pane, egui::Key::Enter, false);
    assert_eq!(wait_for_title(&mut h, pane, "write:"), "write:ok");
    assert_eq!(std::fs::read(&target).expect("written file"), b"hi");
}

/// `fs.write` "Deny once" refuses that write and stores nothing.
#[test]
fn fs_write_deny_once_refuses_and_stores_nothing() {
    let dir = tempfile::tempdir().expect("plugin dir");
    write_probe(
        dir.path(),
        "fs.write",
        &[],
        "FileWrite(\"consent-out.txt\", b\"hi\")",
    );
    let mut h = HostHarness::new();
    let pane = launch(&mut h, dir.path());
    wait_for_prompt(&mut h, pane, "fs.write");
    press_on_prompt(&mut h, pane, egui::Key::Escape, false);
    assert_eq!(wait_for_title(&mut h, pane, "write:"), "write:error");
    assert!(!python(&mut h, pane)
        .config
        .workspace_root
        .join("consent-out.txt")
        .exists());
    assert!(!monitor()
        .list_entries()
        .iter()
        .any(|entry| entry.actor_id == PLUGIN && entry.kind != "pending"));
}

/// `ai.query` takes the same gate through `RequestCapability` until PR #3137
/// adds its effect: the request asks in the pane and the plugin hears the
/// answer.
#[test]
fn ai_query_request_asks_in_the_pane_and_reports_the_answer() {
    let dir = tempfile::tempdir().expect("plugin dir");
    write_probe(
        dir.path(),
        "ai.query",
        &[],
        "RequestCapability(\"ai.query\")",
    );
    let mut h = HostHarness::new();
    let pane = launch(&mut h, dir.path());
    wait_for_prompt(&mut h, pane, "ai.query");
    h.run_frames(5);
    assert_eq!(title(&mut h, pane), "sent");

    press_on_prompt(&mut h, pane, egui::Key::Enter, false);
    assert_eq!(wait_for_title(&mut h, pane, "granted:"), "granted:ai.query");
    assert!(
        !python(&mut h, pane).has_capability("ai.query"),
        "a consent capability never enters the launch-set check"
    );
}

/// `ai.query` "Deny once" answers the plugin with a denial.
#[test]
fn ai_query_deny_once_reports_the_denial() {
    let dir = tempfile::tempdir().expect("plugin dir");
    write_probe(
        dir.path(),
        "ai.query",
        &[],
        "RequestCapability(\"ai.query\")",
    );
    let mut h = HostHarness::new();
    let pane = launch(&mut h, dir.path());
    wait_for_prompt(&mut h, pane, "ai.query");
    press_on_prompt(&mut h, pane, egui::Key::Escape, false);
    assert_eq!(wait_for_title(&mut h, pane, "denied:"), "denied:ai.query");
}

/// Finding 5 (second half). Every value the prompt interpolates is escaped,
/// and the caption states what "Always allow" stores.
#[test]
fn prompt_lines_escape_untrusted_text() {
    let view = crate::broker::gate::PendingView {
        pending_request_id: "req_1".to_string(),
        actor_id: "probe\nAlways allow is safe".to_string(),
        tool: "net.http".to_string(),
        resource_id: Some("a.example,b.example\u{202e}\ninjected".to_string()),
        args_fingerprint: String::new(),
        input_summary: String::new(),
        allow_tool: false,
        context_id: None,
        instance_id: None,
    };
    let lines = consent::prompt_lines(&view);
    assert!(
        lines.iter().all(|line| !line
            .chars()
            .any(crate::host::guest_text::is_unsafe_to_print)),
        "{lines:?}"
    );
    assert_eq!(lines[0], "probe\\nAlways allow is safe requests:");
    assert_eq!(lines[1], "net.http: Make outbound HTTP requests");
    assert!(lines[2].starts_with("Hosts: a.example, b.example\\u{202e}\\ninjected"));
    assert!(
        lines[2].ends_with(" and their subdomains"),
        "the prompt discloses that listed hosts cover their subdomains: {lines:?}"
    );
    assert!(lines
        .last()
        .unwrap()
        .starts_with("Always allow saves this for 30 days"));

    let scope = consent::host_scope(&[
        "B.example.".to_string(),
        "a.example".to_string(),
        "b.example".to_string(),
    ]);
    assert_eq!(scope, "a.example,b.example");
    assert_eq!(consent::host_scope(&[]), "*");
}

/// Codex round 2, finding 3. A pane that already waits on `MAX_PARKED` asks
/// is refused before the broker is asked: no ask is filed that nothing waits
/// on.
#[test]
fn a_full_pane_files_no_ask_nothing_waits_on() {
    let (port, server) = loopback_server();
    let dir = tempfile::tempdir().expect("plugin dir");
    write_probe(dir.path(), "net.http", &["127.0.0.1"], "SetTitle(\"idle\")");
    let mut h = HostHarness::new();
    let pane = launch(&mut h, dir.path());
    wait_until(&mut h, "the pane owner", |h| {
        python(h, pane).pane_owner.is_some()
    });
    for n in 0..consent::MAX_PARKED {
        guest_fetch(&mut h, pane, port, &format!("/{n}"));
    }
    assert_eq!(
        python(&mut h, pane).consent.waiting.len(),
        consent::MAX_PARKED
    );
    let asks = |h: &mut HostHarness| {
        let _ = h;
        monitor()
            .list_pending()
            .into_iter()
            .filter(|view| view.actor_id == PLUGIN)
            .count()
    };
    assert_eq!(asks(&mut h), consent::MAX_PARKED);
    guest_fetch(&mut h, pane, port, "/over");
    assert_eq!(
        asks(&mut h),
        consent::MAX_PARKED,
        "the refused request filed no ask"
    );
    assert_eq!(
        crate::app::attention_queue::open_needs_you_items()
            .iter()
            .filter(|item| item.actor == PLUGIN)
            .count(),
        consent::MAX_PARKED
    );
    assert_eq!(
        python(&mut h, pane).consent.waiting.len(),
        consent::MAX_PARKED
    );
    assert!(hits(&server).is_empty());
}

/// Codex round 2, finding 4. Replacing the pane's runtime (hot reload)
/// retires the asks the old guest made; no later decision runs the old
/// guest's request.
#[test]
fn relaunch_retires_the_old_guests_asks() {
    let (port, server) = loopback_server();
    let dir = tempfile::tempdir().expect("plugin dir");
    write_probe(dir.path(), "net.http", &["127.0.0.1"], "SetTitle(\"idle\")");
    let mut h = HostHarness::new();
    let pane = launch(&mut h, dir.path());
    wait_until(&mut h, "the pane owner", |h| {
        python(h, pane).pane_owner.is_some()
    });
    guest_fetch(&mut h, pane, port, "/old");
    let old = wait_for_prompt(&mut h, pane, "net.http");

    python(&mut h, pane).relaunch_for_test().expect("relaunch");
    assert!(python(&mut h, pane).consent.waiting.is_empty());
    assert!(
        monitor().show_pending(&old).is_none(),
        "the old ask is gone"
    );
    assert_eq!(
        monitor()
            .get_needs_you(&old)
            .and_then(|record| record.resolution),
        Some(NeedsYouResolution::Retired)
    );
    assert!(monitor()
        .approve_pending(&old, ApprovalChoice::Always)
        .is_err());
    h.run_frames(10);
    assert!(
        hits(&server).is_empty(),
        "the old guest's request never ran"
    );
}
