//! Stint 1489: a Python plugin that declares a consent capability (`net.http`,
//! `fs.write`, `ai.query`) is asked about it in its pane on a fresh profile,
//! instead of being refused with nothing the user can click. Answers go
//! through `PermissionMonitor`; a refusal says how to allow it.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::consent::{self, ConsentAdmission, PaneConsent, Refusal};
use crate::broker::gate::{PermissionCaller, PermissionMonitor, PermissionMutation};
use crate::broker::{Decision, GrantSource};
use crate::host::pane::{Pane, ViewRuntime};
use crate::spatial::tiling::PaneId;
use crate::testing::HostHarness;

const PLUGIN: &str = "consent-probe";

fn write_plugin(dir: &std::path::Path, capability: &str, init: &str, update: &str) {
    std::fs::write(
        dir.join("manifest.toml"),
        format!(
            "schema_version = 2\n\n[plugin]\nid = \"{PLUGIN}\"\nname = \"Consent Probe\"\nversion = \"0.1.0\"\n\n[plugin.capabilities]\ncapabilities = [\"{capability}\"]\nallowed_hosts = [\"127.0.0.1\"]\n\n[runtime]\nkind = \"python\"\nentry = \"main.py\"\n\n[[contributes.views]]\nid = \"{PLUGIN}.main\"\ndefault = true\n"
        ),
    )
    .expect("write manifest");
    std::fs::write(
        dir.join("main.py"),
        format!(
            "from plexi_sdk.effects import FileWrite, HttpFetch, RequestCapability, SetTitle\n\
             from plexi_sdk.events import CapabilityDenied, CapabilityGranted, FileWriteResult, HttpResponse\n\
             from plexi_sdk.ui import Column\n\n\
             def init(size, args):\n\
             \x20   return [SetTitle(\"sent\"), {init}]\n\n\
             def update(event):\n\
             {update}\
             \x20   return []\n\n\
             def view():\n\
             \x20   return Column([])\n"
        ),
    )
    .expect("write main.py");
}

const HTTP_UPDATE: &str = "\x20   if isinstance(event, HttpResponse):\n\
\x20       if event.refused:\n\
\x20           return [SetTitle(f\"refused:{event.refused}:{event.error}\")]\n\
\x20       if event.error:\n\
\x20           return [SetTitle(f\"error:{event.error}\")]\n\
\x20       return [SetTitle(f\"http:{event.status}:{event.text()}\")]\n";

fn write_http_probe(dir: &std::path::Path, url: &str) {
    write_plugin(
        dir,
        "net.http",
        &format!("HttpFetch(\"{url}\")"),
        HTTP_UPDATE,
    );
}

/// A loopback server that answers every request `200 ok` and counts them.
fn loopback_server() -> (u16, Arc<AtomicUsize>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().expect("local addr").port();
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&hits);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            counter.fetch_add(1, Ordering::SeqCst);
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
            );
        }
    });
    (port, hits)
}

fn launch(h: &mut HostHarness, dir: &std::path::Path) -> PaneId {
    h.app
        .launch_view_by_path_with_layout_no_review_modal(&dir.to_string_lossy(), None, None, &[])
        .expect("launch probe")
        .expect("probe pane id")
}

fn python(h: &mut HostHarness, pane: PaneId) -> &mut super::LivePythonPane {
    let window = h.app.active_window;
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

fn wait_for_prompt(h: &mut HostHarness, pane: PaneId, capability: &str) {
    wait_until(h, &format!("the {capability} prompt"), |h| {
        python(h, pane)
            .pending_capability_prompt()
            .is_some_and(|prompt| prompt.capability() == capability)
    });
}

fn wait_for_title(h: &mut HostHarness, pane: PaneId, prefix: &str) -> String {
    wait_until(h, &format!("a title starting {prefix:?}"), |h| {
        title(h, pane).starts_with(prefix)
    });
    title(h, pane)
}

fn monitor() -> Arc<PermissionMonitor> {
    PermissionMonitor::for_profile(&crate::config::config_dir()).expect("tests run as the host")
}

fn human() -> PermissionCaller {
    PermissionCaller {
        human: true,
        actor_id: "user:local".to_string(),
    }
}

/// The acceptance case. Fresh profile: the fetch waits on a prompt naming
/// the capability and its hosts, and nothing is sent. "Always allow" sends
/// the held request, stores the grant through the monitor (listed for the
/// Permissions plugin and `plexi permissions`), survives a restart, and a
/// revoke is seen by the running plugin on its next use.
#[test]
fn fresh_profile_fetch_asks_then_always_allow_fetches_and_revoke_applies_live() {
    let (port, hits) = loopback_server();
    let dir = tempfile::tempdir().expect("plugin dir");
    write_http_probe(dir.path(), &format!("http://127.0.0.1:{port}/x"));
    let mut h = HostHarness::new();
    let pane = launch(&mut h, dir.path());

    wait_for_prompt(&mut h, pane, "net.http");
    h.run_frames(5);
    assert_eq!(
        title(&mut h, pane),
        "sent",
        "the request waits, it is not refused"
    );
    assert_eq!(
        hits.load(Ordering::SeqCst),
        0,
        "nothing is sent before the answer"
    );
    let lines = consent::prompt_lines(PLUGIN, "net.http", &["127.0.0.1".to_string()]);
    assert!(
        lines.iter().any(|line| line == "Hosts: 127.0.0.1"),
        "{lines:?}"
    );

    python(&mut h, pane).decide_capability_prompt(true, true);
    assert_eq!(wait_for_title(&mut h, pane, "http:"), "http:200:ok");
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert!(!python(&mut h, pane).has_pending_capability_prompt());

    let workspace = python(&mut h, pane).config.workspace_root.clone();
    let id = format!("cap:{PLUGIN}:net.http");
    assert_eq!(
        monitor().live_capability_decision(PLUGIN, &workspace, "net.http"),
        Some((Decision::Allow, id.clone()))
    );
    assert!(
        monitor()
            .list_entries()
            .iter()
            .any(|entry| entry.id == id && entry.kind == "allow" && entry.source == "user"),
        "the grant is listed for the Permissions plugin and `plexi permissions list`"
    );
    let restarted = PermissionMonitor::reload_for_test(&crate::config::config_dir());
    assert_eq!(
        restarted
            .live_capability_decision(PLUGIN, &workspace, "net.http")
            .map(|(decision, _)| decision),
        Some(Decision::Allow),
        "the remembered grant survives a restart"
    );
    python(&mut h, pane).set_permission_monitor_for_test(Some(Arc::clone(&restarted)));

    assert!(matches!(
        restarted.mutate_entry(&id, "revoke", &human()),
        PermissionMutation::Applied { .. }
    ));
    assert!(
        !python(&mut h, pane).has_capability("net.http"),
        "the running plugin sees the revoke on its next use"
    );
    assert_eq!(
        python(&mut h, pane).consent_admission("net.http"),
        ConsentAdmission::Ask,
        "after a revoke the plugin is asked again, not silently allowed"
    );
}

/// "Deny once": the held request comes back as a refusal that says how to be
/// asked again, and nothing is sent or stored.
#[test]
fn deny_once_refuses_with_a_way_back_and_sends_nothing() {
    let (port, hits) = loopback_server();
    let dir = tempfile::tempdir().expect("plugin dir");
    write_http_probe(dir.path(), &format!("http://127.0.0.1:{port}/x"));
    let mut h = HostHarness::new();
    let pane = launch(&mut h, dir.path());
    wait_for_prompt(&mut h, pane, "net.http");

    python(&mut h, pane).decide_capability_prompt(false, false);
    let shown = wait_for_title(&mut h, pane, "refused:");
    assert_eq!(
        shown,
        format!(
            "refused:net.http:net.http was denied for this pane. Reopen {PLUGIN} to be asked again."
        )
    );
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    let workspace = python(&mut h, pane).config.workspace_root.clone();
    assert_eq!(
        monitor().live_capability_decision(PLUGIN, &workspace, "net.http"),
        None,
        "an unremembered answer stores nothing"
    );
}

/// "Always deny" is stored; the refusal names the exact command that allows
/// it, and allowing it there is seen by the running plugin.
#[test]
fn always_deny_names_the_allow_command_and_allowing_applies_live() {
    let (port, hits) = loopback_server();
    let dir = tempfile::tempdir().expect("plugin dir");
    write_http_probe(dir.path(), &format!("http://127.0.0.1:{port}/x"));
    let mut h = HostHarness::new();
    let pane = launch(&mut h, dir.path());
    wait_for_prompt(&mut h, pane, "net.http");

    python(&mut h, pane).decide_capability_prompt(false, true);
    let id = format!("cap:{PLUGIN}:net.http");
    // The title is clipped, so read the full refusal from the gate itself.
    wait_for_title(&mut h, pane, "refused:net.http:net.http is denied");
    assert_eq!(
        python(&mut h, pane).consent_gate("net.http", &serde_json::json!({})),
        Err(Some(format!(
            "net.http is denied for {PLUGIN}. To allow it, use the Permissions plugin or run \
             `plexi permissions allow {id}`."
        )))
    );
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    assert!(matches!(
        monitor().mutate_entry(&id, "allow", &human()),
        PermissionMutation::Applied { .. }
    ));
    assert!(
        python(&mut h, pane).has_capability("net.http"),
        "the running plugin sees the allow on its next use"
    );
}

/// `fs.write` takes the same path: the write waits on the prompt and runs
/// once allowed.
#[test]
fn fs_write_asks_in_the_pane_and_writes_once_allowed() {
    let dir = tempfile::tempdir().expect("plugin dir");
    write_plugin(
        dir.path(),
        "fs.write",
        "FileWrite(\"consent-out.txt\", b\"hi\")",
        "\x20   if isinstance(event, FileWriteResult):\n\
         \x20       return [SetTitle(f\"write:{event.error or 'ok'}\")]\n",
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

    python(&mut h, pane).decide_capability_prompt(true, false);
    assert_eq!(wait_for_title(&mut h, pane, "write:"), "write:ok");
    assert_eq!(std::fs::read(&target).expect("written file"), b"hi");
}

/// `ai.query` takes the same path: an explicit `RequestCapability` asks in
/// the pane and the plugin hears the answer.
#[test]
fn ai_query_request_asks_in_the_pane_and_reports_the_answer() {
    let dir = tempfile::tempdir().expect("plugin dir");
    write_plugin(
        dir.path(),
        "ai.query",
        "RequestCapability(\"ai.query\")",
        "\x20   if isinstance(event, CapabilityGranted):\n\
         \x20       return [SetTitle(f\"granted:{event.name}\")]\n\
         \x20   if isinstance(event, CapabilityDenied):\n\
         \x20       return [SetTitle(f\"denied:{event.name}\")]\n",
    );
    let mut h = HostHarness::new();
    let pane = launch(&mut h, dir.path());
    wait_for_prompt(&mut h, pane, "ai.query");
    h.run_frames(5);
    assert_eq!(title(&mut h, pane), "sent");

    python(&mut h, pane).decide_capability_prompt(true, false);
    assert_eq!(wait_for_title(&mut h, pane, "granted:"), "granted:ai.query");
    assert!(python(&mut h, pane).has_capability("ai.query"));
}

/// The decision order, without a runtime: undeclared and not-host refuse
/// without asking; a stored row beats the pane's answer; with neither, ask.
#[test]
fn consent_admission_order() {
    let monitor = PermissionMonitor::ephemeral();
    let ws = tempfile::tempdir().expect("workspace");
    let declared = vec!["net.http".to_string()];
    let decide = |monitor: Option<&PermissionMonitor>, declared: &[String], answer, cap| {
        consent::consent_admission(monitor, PLUGIN, ws.path(), declared, answer, cap)
    };
    assert_eq!(
        decide(Some(&monitor), &[], None, "net.http"),
        ConsentAdmission::Refuse(Refusal::Undeclared)
    );
    assert_eq!(
        decide(None, &declared, Some(true), "net.http"),
        ConsentAdmission::Refuse(Refusal::NotHost)
    );
    assert_eq!(
        decide(Some(&monitor), &declared, None, "net.http"),
        ConsentAdmission::Ask
    );
    assert_eq!(
        decide(Some(&monitor), &declared, Some(false), "net.http"),
        ConsentAdmission::Refuse(Refusal::Pane)
    );
    monitor.grant_capability_id(
        PLUGIN,
        ws.path(),
        "net.http",
        Decision::Allow,
        GrantSource::User,
    );
    assert_eq!(
        decide(Some(&monitor), &declared, Some(false), "net.http"),
        ConsentAdmission::Admit {
            mechanism: "python_live_row"
        },
        "a stored allow beats the pane's earlier deny"
    );
    monitor.grant_capability_id(
        PLUGIN,
        ws.path(),
        "net.http",
        Decision::Deny,
        GrantSource::User,
    );
    assert_eq!(
        decide(Some(&monitor), &declared, Some(true), "net.http"),
        ConsentAdmission::Refuse(Refusal::Stored {
            entry_id: format!("cap:{PLUGIN}:net.http")
        }),
        "a stored deny beats the pane's earlier allow"
    );
    assert_eq!(
        consent::refusal_text(PLUGIN, "net.http", &Refusal::Undeclared),
        "missing capability net.http",
        "an undeclared capability keeps the manifest error"
    );
}

/// One prompt per capability however many requests wait, and the queue of
/// held requests is bounded.
#[test]
fn held_requests_share_one_prompt_and_are_bounded() {
    let mut pane = PaneConsent::default();
    for n in 0..consent::MAX_PARKED {
        assert!(pane.park("net.http", serde_json::json!({ "n": n })));
    }
    assert!(!pane.park("net.http", serde_json::json!({ "n": "over" })));
    assert_eq!(pane.prompts.len(), 1);
    assert_eq!(pane.answer_front(true, false).as_deref(), Some("net.http"));
    assert_eq!(pane.replay.len(), consent::MAX_PARKED);
    assert!(pane.parked.is_empty());
    assert_eq!(pane.answers.get("net.http"), Some(&true));
}
