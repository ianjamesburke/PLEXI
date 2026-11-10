//! End-to-end proof of approved CLI operations: a real sandboxed Python app
//! requests an operation, a person decides, and a real compiled fixture CLI runs
//! (or provably does not). Hidden-window and inactive-context servicing included.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::broker::gate::{ApprovalChoice, PermissionMonitor};
use crate::host::cli_ops::fixture;
use crate::host::pane::{ViewRuntime, Pane};
use crate::release::ReleaseFeature;

use super::HostHarness;

const MAIN_PY: &str = r#"import json
from plexi_sdk.effects import CliOperation, SetTitle
from plexi_sdk.events import CliOperationResult
from plexi_sdk.ui import Column, Text

PLAN = []


def _title(event):
    if event.ok:
        return "cliop:ok:" + event.request_id + ":" + json.dumps(event.data, sort_keys=True)
    return "cliop:fail:" + event.request_id + ":" + str(event.error_code)


def _send(step):
    return CliOperation(
        request_id=step["id"], operation_id=step["op"], values=step.get("values", {})
    )


def init(size, args):
    global PLAN
    PLAN = json.loads(args[0]) if args else []
    out = [SetTitle("cliop:start")]
    if PLAN:
        out.append(_send(PLAN.pop(0)))
    return out


def update(event):
    if isinstance(event, CliOperationResult):
        out = [SetTitle(_title(event))]
        if PLAN:
            out.append(_send(PLAN.pop(0)))
        return out
    return []


def view():
    return Column([Text("cli operation fixture")])
"#;

const MANIFEST: &str = r#"schema_version = 2

[[cli_operation]]
id = "fixture.echo"
executable = "fixturecli"
argv = ["echo", "{word}"]
timeout_ms = 10000
[cli_operation.output]
format = "json"
max_bytes = 4096
[cli_operation.authority]
mode = "read"
[cli_operation.slots.word]
type = "string"
max_len = 32

[[cli_operation]]
id = "fixture.children"
executable = "fixturecli"
argv = ["spawn-child"]
timeout_ms = 60000
[cli_operation.output]
format = "json"
max_bytes = 4096
[cli_operation.authority]
mode = "read"

[plugin]
id = "cliop-e2e"
name = "Cli Op E2E"
version = "0.1.0"
description = "approved operation end-to-end fixture"

[plugin.capabilities]
capabilities = ["cli.operations"]

[runtime]
kind = "python"
entry = "main.py"

[[contributes.views]]
id = "cliop-e2e.main"
default = true
"#;

struct E2e {
    _root: tempfile::TempDir,
    plugin_dir: PathBuf,
    workspace: PathBuf,
    _search: crate::host::cli_ops::TestSearchDirsGuard,
    _experiments: Option<crate::release::TestExperimentsGuard>,
    h: HostHarness,
}

impl E2e {
    fn new(enabled: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let plugin_dir = root.path().join("app");
        let workspace = root.path().join("ws");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(plugin_dir.join("manifest.toml"), MANIFEST).unwrap();
        std::fs::write(plugin_dir.join("main.py"), MAIN_PY).unwrap();
        let bin = fixture::install("a", &root.path().join("bin/fixturecli"));
        let search =
            crate::host::cli_ops::set_test_search_dirs(vec![bin.parent().unwrap().to_path_buf()]);
        let experiments =
            enabled.then(|| crate::release::set_test_experiments(&[ReleaseFeature::CliOperations]));
        Self {
            h: HostHarness::new(),
            _root: root,
            plugin_dir,
            workspace,
            _search: search,
            _experiments: experiments,
        }
    }

    fn launch(&mut self, plan: &str) -> u64 {
        let before = self.h.state().open_panes;
        self.h
            .app
            .launch_app_by_path_with_layout(
                &self.plugin_dir.to_string_lossy(),
                None,
                Some(self.workspace.clone()),
                &[plan.to_string()],
            )
            .expect("launch fixture app");
        self.h
            .state()
            .open_panes
            .into_iter()
            .find(|id| !before.contains(id))
            .expect("a pane appears")
    }

    fn title(&self, pane: u64) -> Option<String> {
        self.h
            .app
            .windows
            .iter()
            .find_map(|w| w.panes.get(&pane))
            .and_then(Pane::as_app)
            .and_then(|p| match &p.runtime {
                ViewRuntime::Python(py) => Some(py.display_name()),
                _ => None,
            })
    }

    fn monitor() -> std::sync::Arc<PermissionMonitor> {
        PermissionMonitor::for_profile(&crate::config::config_dir())
    }

    fn pending() -> Vec<crate::broker::gate::PendingView> {
        Self::monitor()
            .list_pending()
            .into_iter()
            .filter(|p| p.tool.starts_with("cli.op."))
            .collect()
    }

    /// Frames until `done` holds. `hidden` runs logic-only passes.
    fn until(
        &mut self,
        pane: u64,
        hidden: bool,
        what: &str,
        done: impl Fn(&E2e, Option<String>) -> bool,
    ) {
        let start = Instant::now();
        loop {
            if hidden {
                self.h.run_hidden_frames(1);
            } else {
                self.h.run_frames(1);
            }
            let title = self.title(pane);
            if done(self, title.clone()) {
                return;
            }
            assert!(
                start.elapsed() < crate::testing::load_aware_timeout(Duration::from_secs(30)),
                "timed out waiting for {what} (title: {title:?})"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn wait_for_approval(&mut self, pane: u64, hidden: bool) -> String {
        self.until(pane, hidden, "an approval request", |_, _| {
            !Self::pending().is_empty()
        });
        Self::pending()[0].pending_request_id.clone()
    }

    fn launches(&self) -> Vec<String> {
        fixture::launches(&self.workspace)
    }
}

const ECHO_PLAN: &str = r#"[{"id":"r1","op":"fixture.echo","values":{"word":"hi"}}]"#;

#[test]
fn sandboxed_app_gets_a_correlated_result_after_approval_while_the_window_is_hidden() {
    let mut e2e = E2e::new(true);
    let pane = e2e.launch(ECHO_PLAN);
    let id = e2e.wait_for_approval(pane, false);
    assert!(
        e2e.launches().is_empty(),
        "nothing runs before the decision"
    );
    let text = E2e::pending()[0].input_summary.clone();
    assert!(text.contains("full authority"), "{text}");
    E2e::monitor()
        .approve_pending(&id, ApprovalChoice::Once)
        .unwrap();
    // From here no `App::ui` pass runs at all.
    e2e.until(pane, true, "the result", |_, t| {
        t.as_deref().is_some_and(|t| t.starts_with("cliop:ok:r1:"))
    });
    let title = e2e.title(pane).unwrap();
    assert!(title.contains(r#""argv": ["hi"]"#), "{title}");
    assert_eq!(e2e.launches().len(), 1);
}

#[test]
fn result_arrives_while_the_apps_context_is_inactive_and_the_window_hidden() {
    let mut e2e = E2e::new(true);
    let pane = e2e.launch(ECHO_PLAN);
    let id = e2e.wait_for_approval(pane, false);
    // Move the person to another context: the app's window is now inactive.
    let other = tempfile::tempdir().unwrap();
    let ctx_id = crate::testing::reserve_test_context_id();
    let idx = super::harness_tests::push_secondary_context(&mut e2e.h, ctx_id, other.path());
    e2e.h.app.active_window = idx;
    assert!(
        e2e.h.app.windows[0].panes.contains_key(&pane),
        "the pane stays in its own window"
    );
    E2e::monitor()
        .approve_pending(&id, ApprovalChoice::Once)
        .unwrap();
    e2e.until(pane, true, "the result in an inactive context", |_, t| {
        t.as_deref().is_some_and(|t| t.starts_with("cliop:ok:r1:"))
    });
    assert_eq!(e2e.launches().len(), 1);
}

#[test]
fn denial_reaches_the_app_and_launches_nothing() {
    let mut e2e = E2e::new(true);
    let pane = e2e.launch(ECHO_PLAN);
    let id = e2e.wait_for_approval(pane, false);
    E2e::monitor()
        .approve_pending(&id, ApprovalChoice::Deny)
        .unwrap();
    e2e.until(pane, true, "the denial", |_, t| {
        t.as_deref() == Some("cliop:fail:r1:denied")
    });
    assert!(e2e.launches().is_empty());
}

#[test]
fn experiment_off_refuses_before_any_approval_or_launch() {
    let mut e2e = E2e::new(false);
    let pane = e2e.launch(ECHO_PLAN);
    e2e.until(pane, false, "the gate error", |_, t| {
        t.as_deref() == Some("cliop:fail:r1:disabled")
    });
    assert!(
        E2e::pending().is_empty(),
        "a disabled experiment asks nobody"
    );
    assert!(e2e.launches().is_empty());
}

#[test]
fn closing_the_app_kills_a_running_operation_and_its_children() {
    let mut e2e = E2e::new(true);
    let pane = e2e.launch(r#"[{"id":"k","op":"fixture.children","values":{}}]"#);
    let id = e2e.wait_for_approval(pane, false);
    E2e::monitor()
        .approve_pending(&id, ApprovalChoice::Once)
        .unwrap();
    let pid_file = e2e.workspace.join(".child-pid");
    e2e.until(pane, true, "the program to start", |_, _| pid_file.exists());
    let pid: i32 = std::fs::read_to_string(&pid_file).unwrap().parse().unwrap();
    assert_eq!(unsafe { libc::kill(pid, 0) }, 0, "child runs before close");
    e2e.h.app.close_pane_by_id(pane);
    e2e.h.run_frames(2);
    let start = Instant::now();
    while unsafe { libc::kill(pid, 0) } == 0 {
        assert!(
            start.elapsed() < crate::testing::load_aware_timeout(Duration::from_secs(5)),
            "grandchild {pid} survived closing the app"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_physical_key_in_the_attention_overlay_approves_once_and_the_fixture_runs() {
    let mut e2e = E2e::new(true);
    let pane = e2e.launch(ECHO_PLAN);
    let id = e2e.wait_for_approval(pane, false);
    assert!(e2e.launches().is_empty());
    e2e.h.app.sync_attention_queue();
    assert!(
        e2e.h.app.open_decision_overlay_for_ask(&id),
        "the operation approval is in the attention queue"
    );
    // Option 1 is "Allow once"; the digit travels the production key path.
    e2e.h.press_key(egui::Key::Num1, egui::Modifiers::NONE);
    e2e.until(pane, false, "the result", |_, t| {
        t.as_deref().is_some_and(|t| t.starts_with("cliop:ok:r1:"))
    });
    assert_eq!(e2e.launches().len(), 1);
    assert!(
        E2e::pending().is_empty(),
        "the decision closed the approval request"
    );
}

#[test]
fn revocation_of_a_remembered_operation_survives_a_restart() {
    let mut e2e = E2e::new(true);
    let pane = e2e.launch(ECHO_PLAN);
    let id = e2e.wait_for_approval(pane, false);
    E2e::monitor()
        .approve_pending(&id, ApprovalChoice::Always)
        .unwrap();
    e2e.until(pane, true, "the result", |_, t| {
        t.as_deref().is_some_and(|t| t.starts_with("cliop:ok:r1:"))
    });
    let grant = E2e::monitor()
        .list_entries()
        .into_iter()
        .find(|row| row.tool == "cli.op.read" && row.kind == "allow")
        .expect("the remembered grant is in the Permissions inventory");
    assert!(E2e::monitor().revoke_grant_id(&grant.id));
    // Restart: the process cache is dropped and the sealed store reloaded.
    let monitor = PermissionMonitor::reload_for_test(&crate::config::config_dir());
    assert!(
        monitor
            .list_entries()
            .iter()
            .all(|row| !(row.tool == "cli.op.read" && row.kind == "allow")),
        "a revoked grant must not come back after restart"
    );
    let pane = e2e.launch(r#"[{"id":"r2","op":"fixture.echo","values":{"word":"again"}}]"#);
    e2e.wait_for_approval(pane, false);
    assert_eq!(
        e2e.launches().len(),
        1,
        "revoked consent launched nothing new"
    );
}
