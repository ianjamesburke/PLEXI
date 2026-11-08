//! Host-side proof of the approved-operation contract. Every refusal asserts the
//! fixture CLI never started: its marker file is absent from the workspace.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::exec::Wake;
use super::fixture;
use super::service::*;
use crate::broker::gate::{ApprovalChoice, PermissionMonitor};

const MANIFEST: &str = r#"schema_version = 1

[app]
id = "ops-app"
type = "app"
name = "Ops App"
entry = "main.py"
version = "0.1.0"
description = "approved operation fixture"

[app.capabilities]
capabilities = ["cli.operations"]

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
id = "fixture.set"
executable = "fixturecli"
argv = ["echo", "--priority={priority}", "--", "{task}"]
timeout_ms = 10000
[cli_operation.output]
format = "json"
max_bytes = 4096
[cli_operation.authority]
mode = "mutate"
fs_write = ["workspace"]
[cli_operation.slots.priority]
type = "enum"
values = ["p0", "p1"]
[cli_operation.slots.task]
type = "string"
max_len = 32

[[cli_operation]]
id = "fixture.slow"
executable = "fixturecli"
argv = ["sleep", "{ms}"]
timeout_ms = 400
[cli_operation.output]
format = "json"
max_bytes = 4096
[cli_operation.authority]
mode = "read"
[cli_operation.slots.ms]
type = "int"
min = 0
max = 60000

[[cli_operation]]
id = "fixture.long"
executable = "fixturecli"
argv = ["sleep", "{ms}"]
timeout_ms = 60000
[cli_operation.output]
format = "json"
max_bytes = 4096
[cli_operation.authority]
mode = "read"
[cli_operation.slots.ms]
type = "int"
min = 0
max = 60000

[[cli_operation]]
id = "fixture.spray"
executable = "fixturecli"
argv = ["spray", "{n}"]
timeout_ms = 10000
[cli_operation.output]
format = "text"
max_bytes = 2048
[cli_operation.authority]
mode = "read"
[cli_operation.slots.n]
type = "int"
min = 0
max = 10000000

[[cli_operation]]
id = "fixture.badjson"
executable = "fixturecli"
argv = ["text"]
timeout_ms = 10000
[cli_operation.output]
format = "json"
max_bytes = 4096
[cli_operation.authority]
mode = "read"

[[cli_operation]]
id = "fixture.exit"
executable = "fixturecli"
argv = ["exit", "{code}"]
timeout_ms = 10000
[cli_operation.output]
format = "json"
max_bytes = 4096
[cli_operation.authority]
mode = "read"
[cli_operation.slots.code]
type = "int"
min = 0
max = 255

[[cli_operation]]
id = "fixture.read"
executable = "fixturecli"
argv = ["read", "{path}"]
timeout_ms = 10000
[cli_operation.output]
format = "json"
max_bytes = 4096
[cli_operation.authority]
mode = "read"
fs_read = ["workspace"]
[cli_operation.slots.path]
type = "string"
max_len = 512

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

[[cli_operation]]
id = "fixture.dyld"
executable = "fixturecli"
argv = ["fake-dyld"]
timeout_ms = 10000
[cli_operation.output]
format = "json"
max_bytes = 4096
[cli_operation.authority]
mode = "read"

[[cli_operation]]
id = "fixture.failjson"
executable = "fixturecli"
argv = ["fail-json"]
timeout_ms = 10000
[cli_operation.output]
format = "json"
max_bytes = 2048
[cli_operation.authority]
mode = "read"

[[cli_operation]]
id = "fixture.failtext"
executable = "fixturecli"
argv = ["fail-text"]
timeout_ms = 10000
[cli_operation.output]
format = "json"
max_bytes = 2048
[cli_operation.authority]
mode = "read"

[[cli_operation]]
id = "fixture.failspray"
executable = "fixturecli"
argv = ["fail-spray"]
timeout_ms = 10000
[cli_operation.output]
format = "json"
max_bytes = 2048
[cli_operation.authority]
mode = "read"

[[cli_operation]]
id = "fixture.failplain"
executable = "fixturecli"
argv = ["fail-text"]
timeout_ms = 10000
[cli_operation.output]
format = "text"
max_bytes = 2048
[cli_operation.authority]
mode = "read"

[[cli_operation]]
id = "fixture.script"
executable = "scriptcli"
argv = ["echo"]
timeout_ms = 10000
[cli_operation.output]
format = "json"
max_bytes = 4096
[cli_operation.authority]
mode = "read"
"#;

struct Rig {
    root: tempfile::TempDir,
    profile: tempfile::TempDir,
    app_dir: PathBuf,
    workspace: PathBuf,
    bin: PathBuf,
    seal_base: PathBuf,
    monitor: Arc<PermissionMonitor>,
    service: CliOpService,
    who: PaneIdentity,
    enabled: bool,
    approval_timeout: Duration,
    _profile_guard: Option<crate::config::TestProfileDirGuard>,
}

fn wake() -> Wake {
    Arc::new(|| {})
}

impl Rig {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let profile = tempfile::tempdir().unwrap();
        let app_dir = root.path().join("app");
        let workspace = root.path().join("ws");
        std::fs::create_dir_all(&app_dir).unwrap();
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(app_dir.join("manifest.toml"), MANIFEST).unwrap();
        std::fs::write(app_dir.join("main.py"), "# entry\n").unwrap();
        let bin = fixture::install("a", &root.path().join("bin/fixturecli"));
        let monitor = PermissionMonitor::for_profile(profile.path());
        let who = PaneIdentity {
            app_id: "ops-app".into(),
            app_dir: app_dir.clone(),
            workspace_root: workspace.clone(),
            pane_id: 4,
            context_id: 9,
            has_capability: true,
        };
        Self {
            seal_base: root.path().join("sealed"),
            root,
            profile,
            app_dir,
            workspace,
            bin,
            monitor,
            service: CliOpService::default(),
            who,
            enabled: true,
            approval_timeout: Duration::from_secs(60),
            _profile_guard: None,
        }
    }

    /// A rig whose app was put in place by the real installer, so it lives in
    /// the registry's apps directory with an install-complete marker.
    fn installed() -> Self {
        let mut rig = Self::new();
        rig._profile_guard = Some(crate::config::set_test_profile_dir(
            rig.profile.path().to_path_buf(),
        ));
        rig.install_from(&rig.app_dir.clone());
        rig.app_dir = crate::app::registry::apps_dir().join("ops-app");
        rig.who.app_dir = rig.app_dir.clone();
        rig
    }

    fn install_from(&self, source: &std::path::Path) -> i32 {
        crate::cli::app::app_install_with_pin(
            &source.to_string_lossy(),
            None,
            crate::cli::app::InstallConfirm::PreApproved,
            true,
        )
    }

    fn search(&self) -> Vec<PathBuf> {
        vec![self.bin.parent().unwrap().to_path_buf()]
    }

    fn submit_raw(&mut self, message: Value) -> Vec<Value> {
        let search = self.search();
        let w = wake();
        let message_text = if self.enabled {
            ""
        } else {
            "cli_operations is an experiment"
        };
        let env = Env {
            monitor: &self.monitor,
            search_dirs: &search,
            seal_base: &self.seal_base,
            wake: &w,
            enabled: self.enabled,
            experiment_message: message_text,
            approval_timeout: self.approval_timeout,
        };
        self.service.submit(&self.who, &env, &message)
    }

    fn submit(&mut self, request: &str, operation: &str, values: Value) -> Vec<Value> {
        self.submit_raw(json!({
            "type": "cli_operation",
            "request_id": request,
            "operation_id": operation,
            "values": values,
        }))
    }

    fn poll(&mut self) -> Vec<Value> {
        let search = self.search();
        let w = wake();
        let env = Env {
            monitor: &self.monitor,
            search_dirs: &search,
            seal_base: &self.seal_base,
            wake: &w,
            enabled: self.enabled,
            experiment_message: "",
            approval_timeout: self.approval_timeout,
        };
        self.service.poll(&self.who, &env)
    }

    /// Poll until `n` replies arrived.
    fn replies(&mut self, n: usize) -> Vec<Value> {
        let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(20));
        let mut out = Vec::new();
        while out.len() < n {
            out.extend(self.poll());
            assert!(
                Instant::now() < deadline,
                "only {} of {n} replies: {out:?}",
                out.len()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        out
    }

    fn pending(&self) -> Vec<crate::broker::gate::PendingView> {
        self.monitor
            .list_pending()
            .into_iter()
            .filter(|p| p.tool.starts_with("cli.op."))
            .collect()
    }

    fn decide(&self, choice: ApprovalChoice) -> String {
        let pending = self.pending();
        assert_eq!(
            pending.len(),
            1,
            "exactly one operation approval is open: {pending:?}"
        );
        let id = pending[0].pending_request_id.clone();
        self.monitor
            .approve_pending(&id, choice)
            .expect("decision applies");
        id
    }

    fn launches(&self) -> Vec<String> {
        fixture::launches(&self.workspace)
    }

    fn assert_not_launched(&self) {
        assert!(
            self.launches().is_empty(),
            "the program must not have started: {:?}",
            self.launches()
        );
    }

    /// Run one echo with a remembered grant; returns the reply.
    fn run_ok(&mut self, request: &str, word: &str) -> Value {
        assert!(self
            .submit(request, "fixture.echo", json!({"word": word}))
            .is_empty());
        self.replies(1).remove(0)
    }

    fn remember(&mut self, choice: ApprovalChoice) {
        assert!(self
            .submit("seed", "fixture.echo", json!({"word": "seed"}))
            .is_empty());
        self.decide(choice);
        let reply = self.replies(1).remove(0);
        assert_eq!(reply["ok"], true, "{reply}");
    }
}

fn code_of(reply: &Value) -> &str {
    reply["error_code"].as_str().unwrap_or("")
}

// ---- Done-When 1: correlated, bounded results and lifecycle ----

#[test]
fn approved_once_returns_a_correlated_result_and_the_disclosure_names_ambient_authority() {
    let mut rig = Rig::new();
    assert!(rig
        .submit("req-1", "fixture.echo", json!({"word": "hi"}))
        .is_empty());
    let pending = rig.pending();
    assert_eq!(pending.len(), 1);
    let text = &pending[0].input_summary;
    for needle in [
        "ops-app",
        "fixture.echo",
        "full authority",
        "does not confine",
        "self-declared",
    ] {
        assert!(
            text.contains(needle),
            "disclosure must mention {needle}: {text}"
        );
    }
    assert!(
        text.contains(rig.workspace.canonicalize().unwrap().to_str().unwrap()),
        "{text}"
    );
    assert!(
        text.contains(rig.bin.canonicalize().unwrap().to_str().unwrap()),
        "{text}"
    );
    assert!(
        rig.launches().is_empty(),
        "nothing runs before the decision"
    );
    rig.decide(ApprovalChoice::Once);
    let reply = rig.replies(1).remove(0);
    assert_eq!(reply["type"], "cli_operation_result");
    assert_eq!(reply["request_id"], "req-1");
    assert_eq!(reply["operation_id"], "fixture.echo");
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["data"]["argv"], json!(["hi"]));
    assert_eq!(rig.launches().len(), 1);
}

#[test]
fn timeout_output_limit_nonzero_and_bad_output_are_typed_and_bounded() {
    let mut rig = Rig::new();
    rig.remember(ApprovalChoice::Session);
    // Remembered grants are per operation: approve each once more.
    for (op, values, expected) in [
        ("fixture.slow", json!({"ms": 30000}), "timeout"),
        ("fixture.spray", json!({"n": 5_000_000}), "output_too_large"),
        ("fixture.exit", json!({"code": 7}), "nonzero_exit"),
        ("fixture.badjson", json!({}), "bad_output"),
    ] {
        assert!(rig.submit("r", op, values).is_empty(), "{op}");
        rig.decide(ApprovalChoice::Session);
        let reply = rig.replies(1).remove(0);
        assert_eq!(code_of(&reply), expected, "{op}: {reply}");
        assert_eq!(reply["request_id"], "r");
    }
}

#[test]
fn concurrency_is_capped_per_app() {
    let mut rig = Rig::new();
    assert!(rig
        .submit("a", "fixture.long", json!({"ms": 20000}))
        .is_empty());
    rig.decide(ApprovalChoice::Session);
    assert!(rig
        .submit("b", "fixture.long", json!({"ms": 20000}))
        .is_empty());
    let busy = rig.submit("c", "fixture.long", json!({"ms": 20000}));
    assert_eq!(busy.len(), 1);
    assert_eq!(code_of(&busy[0]), "busy");
    assert_eq!(busy[0]["request_id"], "c");
    // Free the running program so the test leaves nothing behind.
    rig.service.cancel("a");
    rig.service.cancel("b");
}

#[test]
fn cancel_kills_a_running_operation_and_fails_a_parked_one() {
    let mut rig = Rig::new();
    assert!(rig
        .submit("run", "fixture.long", json!({"ms": 20000}))
        .is_empty());
    rig.decide(ApprovalChoice::Session);
    assert!(rig.service.cancel("nope").is_empty());
    // `run` is parked until the decision lands in a poll.
    let _ = rig.poll();
    assert!(rig.service.cancel("run").is_empty());
    let reply = rig.replies(1).remove(0);
    assert_eq!(code_of(&reply), "cancelled");

    assert!(rig
        .submit("wait", "fixture.echo", json!({"word": "x"}))
        .is_empty());
    let cancelled = rig.service.cancel("wait");
    assert_eq!(code_of(&cancelled[0]), "cancelled");
    let before = rig.launches().len();
    rig.decide(ApprovalChoice::Once);
    assert!(
        rig.poll().is_empty(),
        "a cancelled request must not resurrect"
    );
    assert_eq!(rig.launches().len(), before);
}

#[test]
fn closing_the_app_kills_every_process() {
    let mut rig = Rig::new();
    assert!(rig.submit("kids", "fixture.children", json!({})).is_empty());
    rig.decide(ApprovalChoice::Once);
    let _ = rig.poll();
    let pid_file = rig.workspace.join(".child-pid");
    let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(10));
    while !pid_file.exists() {
        assert!(Instant::now() < deadline, "fixture never started its child");
        std::thread::sleep(Duration::from_millis(10));
    }
    let pid: i32 = std::fs::read_to_string(&pid_file).unwrap().parse().unwrap();
    assert_eq!(
        unsafe { libc::kill(pid, 0) },
        0,
        "child must be alive before close"
    );
    drop(std::mem::take(&mut rig.service));
    let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(3));
    while unsafe { libc::kill(pid, 0) } == 0 {
        assert!(
            Instant::now() < deadline,
            "grandchild {pid} survived app close"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn an_unanswered_approval_times_out_without_launching() {
    let mut rig = Rig::new();
    rig.approval_timeout = Duration::from_millis(50);
    assert!(rig
        .submit("slow", "fixture.echo", json!({"word": "x"}))
        .is_empty());
    std::thread::sleep(Duration::from_millis(80));
    let reply = rig.replies(1).remove(0);
    assert_eq!(code_of(&reply), "approval_timeout");
    rig.assert_not_launched();
}

// ---- Done-When 2: zero-launch negatives ----

#[test]
fn a_first_ask_does_not_claim_a_change() {
    let mut rig = Rig::new();
    assert!(rig
        .submit("f", "fixture.echo", json!({"word": "x"}))
        .is_empty());
    assert!(!rig.pending()[0].input_summary.contains("CHANGED"));
}

#[test]
fn a_program_that_needs_its_own_directory_fails_with_a_readable_typed_error() {
    let mut rig = Rig::new();
    assert!(rig.submit("d", "fixture.dyld", json!({})).is_empty());
    rig.decide(ApprovalChoice::Once);
    let reply = rig.replies(1).remove(0);
    assert_eq!(code_of(&reply), "executable_not_self_contained", "{reply}");
    assert!(reply["error"].as_str().unwrap().contains("self-contained"));
}

#[test]
fn nonzero_exit_carries_structured_stdout_and_a_bounded_stderr_tail() {
    let mut rig = Rig::new();
    assert!(rig.submit("f", "fixture.failjson", json!({})).is_empty());
    rig.decide(ApprovalChoice::Once);
    let reply = rig.replies(1).remove(0);
    assert_eq!(code_of(&reply), "nonzero_exit", "{reply}");
    assert_eq!(reply["exit_code"], 1);
    assert_eq!(reply["data_available"], true);
    assert_eq!(reply["data"]["error"], "priority_conflict");
    assert_eq!(reply["data"]["actual_priority"], "p2");
    assert_eq!(reply["stderr"], "conflict");
}

#[test]
fn nonzero_exit_never_surfaces_unparseable_stdout_as_data() {
    let mut rig = Rig::new();
    assert!(rig.submit("f", "fixture.failtext", json!({})).is_empty());
    rig.decide(ApprovalChoice::Once);
    let reply = rig.replies(1).remove(0);
    assert_eq!(code_of(&reply), "nonzero_exit");
    assert_eq!(reply["data_available"], false);
    assert!(reply.get("data").is_none(), "{reply}");
    assert_eq!(reply["stderr"], "boom");
}

#[test]
fn nonzero_exit_text_operations_return_stdout_as_text() {
    let mut rig = Rig::new();
    assert!(rig.submit("f", "fixture.failplain", json!({})).is_empty());
    rig.decide(ApprovalChoice::Once);
    let reply = rig.replies(1).remove(0);
    assert_eq!(code_of(&reply), "nonzero_exit");
    assert_eq!(reply["data_available"], true);
    assert_eq!(reply["data"], "this is not json\n");
}

#[test]
fn nonzero_exit_with_oversized_stdout_is_rejected_by_the_success_path_rule() {
    let mut rig = Rig::new();
    assert!(rig.submit("f", "fixture.failspray", json!({})).is_empty());
    rig.decide(ApprovalChoice::Once);
    let reply = rig.replies(1).remove(0);
    assert_eq!(code_of(&reply), "output_too_large", "{reply}");
    assert!(reply.get("data").is_none());
}

#[test]
fn denial_launches_nothing() {
    let mut rig = Rig::new();
    assert!(rig
        .submit("d", "fixture.echo", json!({"word": "x"}))
        .is_empty());
    rig.decide(ApprovalChoice::Deny);
    let reply = rig.replies(1).remove(0);
    assert_eq!(code_of(&reply), "denied");
    rig.assert_not_launched();
}

#[test]
fn deny_always_blocks_later_calls_without_asking() {
    let mut rig = Rig::new();
    assert!(rig
        .submit("d", "fixture.echo", json!({"word": "x"}))
        .is_empty());
    rig.decide(ApprovalChoice::DenyAlways);
    assert_eq!(code_of(&rig.replies(1).remove(0)), "denied");
    let again = rig.submit("d2", "fixture.echo", json!({"word": "y"}));
    assert_eq!(code_of(&again[0]), "denied");
    assert!(rig.pending().is_empty());
    rig.assert_not_launched();
}

#[test]
fn malformed_and_flag_like_values_launch_nothing_and_ask_nothing() {
    let mut rig = Rig::new();
    for (op, values, expected) in [
        ("fixture.echo", json!({"word": ""}), "invalid_argument"),
        ("fixture.echo", json!({"word": 5}), "invalid_argument"),
        (
            "fixture.echo",
            json!({"word": "x".repeat(33)}),
            "invalid_argument",
        ),
        ("fixture.echo", json!({}), "invalid_argument"),
        (
            "fixture.echo",
            json!({"word": "x", "extra": "y"}),
            "invalid_argument",
        ),
        (
            "fixture.echo",
            json!({"word": "--version"}),
            "flag_like_argument",
        ),
        ("fixture.echo", json!({"word": "-rf"}), "flag_like_argument"),
        (
            "fixture.set",
            json!({"priority": "p9", "task": "1"}),
            "invalid_argument",
        ),
        ("fixture.slow", json!({"ms": 61000}), "invalid_argument"),
        ("fixture.slow", json!({"ms": "10"}), "invalid_argument"),
        ("fixture.missing", json!({}), "unknown_operation"),
    ] {
        let reply = rig.submit("m", op, values.clone());
        assert_eq!(reply.len(), 1, "{op} {values}");
        assert_eq!(code_of(&reply[0]), expected, "{op} {values}: {}", reply[0]);
    }
    assert!(
        rig.pending().is_empty(),
        "invalid input must never reach a human"
    );
    rig.assert_not_launched();
}

#[test]
fn flag_like_text_is_a_plain_operand_after_the_delimiter() {
    let mut rig = Rig::new();
    assert!(rig
        .submit(
            "s",
            "fixture.set",
            json!({"priority": "p0", "task": "--delete-all"})
        )
        .is_empty());
    rig.decide(ApprovalChoice::Once);
    let reply = rig.replies(1).remove(0);
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(
        reply["data"]["argv"],
        json!(["--priority=p0", "--", "--delete-all"])
    );
}

#[test]
fn guest_supplied_executable_environment_directory_or_shell_is_refused() {
    let mut rig = Rig::new();
    for extra in [
        "executable",
        "env",
        "cwd",
        "argv",
        "shell",
        "command",
        "workspace",
    ] {
        let mut message = json!({
            "type": "cli_operation",
            "request_id": "g",
            "operation_id": "fixture.echo",
            "values": {"word": "x"},
        });
        message[extra] = json!("/bin/sh");
        let reply = rig.submit_raw(message);
        assert_eq!(
            code_of(&reply[0]),
            "invalid_request",
            "{extra}: {}",
            reply[0]
        );
    }
    let reply = rig
        .submit_raw(json!({"type": "cli_operation", "operation_id": "fixture.echo", "values": {}}));
    assert_eq!(code_of(&reply[0]), "invalid_request");
    let reply = rig.submit_raw(json!({"type": "cli_operation", "request_id": "v", "operation_id": "fixture.echo", "values": []}));
    assert_eq!(code_of(&reply[0]), "invalid_request");
    assert!(rig.pending().is_empty());
    rig.assert_not_launched();
}

#[test]
fn experiment_off_and_missing_capability_launch_nothing_and_probe_nothing() {
    let mut rig = Rig::new();
    rig.enabled = false;
    let reply = rig.submit("e", "fixture.echo", json!({"word": "x"}));
    assert_eq!(code_of(&reply[0]), "disabled");
    rig.enabled = true;
    rig.who.has_capability = false;
    let reply = rig.submit("c", "fixture.echo", json!({"word": "x"}));
    assert_eq!(code_of(&reply[0]), "capability_missing");
    assert!(rig.pending().is_empty());
    assert!(
        !rig.seal_base.exists(),
        "no copy may be made before admission"
    );
    rig.assert_not_launched();
}

#[test]
fn duplicate_request_ids_are_refused() {
    let mut rig = Rig::new();
    assert!(rig
        .submit("same", "fixture.echo", json!({"word": "x"}))
        .is_empty());
    let reply = rig.submit("same", "fixture.echo", json!({"word": "y"}));
    assert_eq!(code_of(&reply[0]), "invalid_request");
}

#[test]
fn script_executables_and_missing_programs_never_reach_a_human_or_launch() {
    let mut rig = Rig::new();
    let script = rig.bin.parent().unwrap().join("scriptcli");
    std::fs::write(&script, "#!/bin/sh\necho hi > marker-from-script\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let reply = rig.submit("s", "fixture.script", json!({}));
    assert_eq!(code_of(&reply[0]), "executable_mismatch", "{}", reply[0]);
    std::fs::remove_file(&rig.bin).unwrap();
    let reply = rig.submit("m", "fixture.echo", json!({"word": "x"}));
    assert_eq!(code_of(&reply[0]), "executable_missing", "{}", reply[0]);
    assert!(rig.pending().is_empty());
    assert!(!rig.workspace.join("marker-from-script").exists());
    rig.assert_not_launched();
}

#[test]
fn once_authorizes_exactly_one_invocation() {
    let mut rig = Rig::new();
    let first = rig.run_once_pair();
    assert_eq!(first["ok"], true);
    assert_eq!(rig.launches().len(), 1);
    // The same call again must ask again and must not run.
    assert!(rig
        .submit("second", "fixture.echo", json!({"word": "hi"}))
        .is_empty());
    assert_eq!(rig.pending().len(), 1, "a spent one-shot grant asks again");
    assert!(rig.poll().is_empty());
    assert_eq!(rig.launches().len(), 1);
}

impl Rig {
    fn run_once_pair(&mut self) -> Value {
        assert!(self
            .submit("first", "fixture.echo", json!({"word": "hi"}))
            .is_empty());
        self.decide(ApprovalChoice::Once);
        self.replies(1).remove(0)
    }
}

#[test]
fn two_requests_sharing_one_once_approval_run_only_one() {
    let mut rig = Rig::new();
    assert!(rig
        .submit("one", "fixture.echo", json!({"word": "hi"}))
        .is_empty());
    assert!(rig
        .submit("two", "fixture.echo", json!({"word": "hi"}))
        .is_empty());
    assert_eq!(
        rig.pending().len(),
        1,
        "identical requests share one prompt"
    );
    rig.decide(ApprovalChoice::Once);
    let replies = rig.replies(1);
    assert_eq!(replies[0]["ok"], true);
    // The other request is asked again rather than riding the spent grant.
    let _ = rig.poll();
    assert_eq!(rig.launches().len(), 1);
    assert_eq!(rig.pending().len(), 1);
}

#[test]
fn remembered_consent_is_rechecked_on_every_call_and_revocation_blocks() {
    let mut rig = Rig::new();
    rig.remember(ApprovalChoice::Session);
    let launched = rig.launches().len();
    assert_eq!(rig.run_ok("a", "one")["ok"], true);
    assert_eq!(rig.run_ok("b", "two")["ok"], true);
    assert!(rig.pending().is_empty(), "no prompt while remembered");
    assert_eq!(rig.launches().len(), launched + 2);

    let entry = rig
        .monitor
        .list_entries()
        .into_iter()
        .find(|e| e.tool == "cli.op.read" && e.kind == "allow")
        .expect("remembered grant is listed in the Permissions inventory");
    assert!(rig.monitor.revoke_grant_id(&entry.id));
    assert!(rig
        .submit("c", "fixture.echo", json!({"word": "three"}))
        .is_empty());
    assert_eq!(rig.pending().len(), 1, "revocation asks again");
    assert_eq!(
        rig.launches().len(),
        launched + 2,
        "revoked grant launched nothing"
    );
}

#[test]
fn read_consent_does_not_authorize_a_mutation() {
    let mut rig = Rig::new();
    rig.remember(ApprovalChoice::Always);
    assert!(rig
        .submit("m", "fixture.set", json!({"priority": "p0", "task": "1"}))
        .is_empty());
    let pending = rig.pending();
    assert_eq!(pending[0].tool, "cli.op.mutate");
    let launched = rig.launches().len();
    assert!(rig.poll().is_empty());
    assert_eq!(rig.launches().len(), launched);
}

#[test]
fn workspace_substitution_asks_again_and_launches_nothing_there() {
    let mut rig = Rig::new();
    rig.remember(ApprovalChoice::Always);
    let other = rig.root.path().join("other-ws");
    std::fs::create_dir_all(&other).unwrap();
    rig.who.workspace_root = other.clone();
    assert!(rig
        .submit("w", "fixture.echo", json!({"word": "x"}))
        .is_empty());
    assert_eq!(rig.pending().len(), 1);
    assert!(rig.poll().is_empty());
    assert!(fixture::launches(&other).is_empty());
}

#[test]
fn package_content_change_requires_reapproval() {
    let mut rig = Rig::new();
    rig.remember(ApprovalChoice::Always);
    let launched = rig.launches().len();
    std::fs::write(rig.app_dir.join("main.py"), "# entry\nprint('changed')\n").unwrap();
    assert!(rig
        .submit("p", "fixture.echo", json!({"word": "x"}))
        .is_empty());
    assert_eq!(rig.pending().len(), 1, "changed package must ask again");
    assert!(rig.poll().is_empty());
    assert_eq!(rig.launches().len(), launched);
}

#[test]
fn operation_definition_change_requires_reapproval() {
    let mut rig = Rig::new();
    rig.remember(ApprovalChoice::Always);
    let launched = rig.launches().len();
    // Widen only the declared timeout of the approved operation.
    let widened = MANIFEST.replacen("timeout_ms = 10000", "timeout_ms = 20000", 1);
    std::fs::write(rig.app_dir.join("manifest.toml"), widened).unwrap();
    assert!(rig
        .submit("d", "fixture.echo", json!({"word": "x"}))
        .is_empty());
    assert_eq!(rig.pending().len(), 1, "changed definition must ask again");
    assert_eq!(rig.launches().len(), launched);
}

#[test]
fn executable_replacement_requires_reapproval_and_runs_neither_binary_unasked() {
    let mut rig = Rig::new();
    rig.remember(ApprovalChoice::Always);
    let launched = rig.launches().len();
    fixture::install("b", &rig.bin);
    assert!(rig
        .submit("x", "fixture.echo", json!({"word": "x"}))
        .is_empty());
    assert_eq!(rig.pending().len(), 1, "a replaced program must ask again");
    assert!(rig.poll().is_empty());
    assert_eq!(rig.launches().len(), launched);
    assert!(
        rig.launches().iter().all(|l| l.starts_with("a ")),
        "{:?}",
        rig.launches()
    );
}

#[test]
fn replacement_while_an_approval_is_open_cannot_inherit_that_approval() {
    let mut rig = Rig::new();
    assert!(rig
        .submit("race", "fixture.echo", json!({"word": "x"}))
        .is_empty());
    // The user is looking at the approval for program "a" when it is swapped.
    fixture::install("b", &rig.bin);
    rig.decide(ApprovalChoice::Once);
    assert!(
        rig.poll().is_empty(),
        "the decision was about a different program"
    );
    rig.assert_not_launched();
    let pending = rig.pending();
    assert_eq!(pending.len(), 1, "the new program needs its own decision");
    // Approving the new program runs the new program, by explicit consent.
    rig.decide(ApprovalChoice::Once);
    let reply = rig.replies(1).remove(0);
    assert_eq!(reply["data"]["version"], "b");
}

#[test]
fn tampered_grant_file_does_not_authorize_after_restart() {
    let _exclusive = crate::broker::host_key::ExclusiveHostKey::acquire();
    let mut rig = Rig::new();
    rig.remember(ApprovalChoice::Always);
    let launched = rig.launches().len();
    let binding_line = std::fs::read_to_string(rig.profile.path().join("grants.toml")).unwrap();
    assert!(binding_line.contains("cli.op.read"), "{binding_line}");
    // Widen the remembered grant to the mutate tool by editing the file.
    std::fs::write(
        rig.profile.path().join("grants.toml"),
        binding_line.replace("cli.op.read", "cli.op.mutate"),
    )
    .unwrap();
    rig.service = CliOpService::default();
    rig.monitor = PermissionMonitor::reload_for_test(rig.profile.path());
    assert!(rig
        .submit("t", "fixture.set", json!({"priority": "p0", "task": "1"}))
        .is_empty());
    assert!(
        !rig.pending().is_empty(),
        "an edited grant must not authorize; the mutate call must ask"
    );
    assert!(rig.poll().is_empty());
    assert_eq!(rig.launches().len(), launched);
}

#[test]
fn failed_grant_persistence_launches_nothing() {
    let _exclusive = crate::broker::host_key::ExclusiveHostKey::acquire();
    let profile = tempfile::tempdir().unwrap();
    {
        // A sealed profile, so an unreadable key makes the next save fail.
        let monitor = PermissionMonitor::open_profile(profile.path());
        monitor.store().try_save().expect("seal profile");
        monitor.note_denial("actor", "c1", "res", "op", "denied");
        crate::broker::integrity::mark_clean_shutdown(profile.path());
    }
    let mut rig = Rig::new();
    rig.monitor = PermissionMonitor::open_profile(profile.path());
    let _denied = crate::broker::host_key::UnreadableMacKey::acquire().delete_needs_prompt();
    assert!(rig
        .submit("p", "fixture.echo", json!({"word": "x"}))
        .is_empty());
    let id = rig.pending()[0].pending_request_id.clone();
    let error = rig
        .monitor
        .approve_pending(&id, ApprovalChoice::Always)
        .expect_err("an unreadable key cannot seal the grant");
    assert!(error.contains("grant was not saved"), "{error}");
    assert!(
        rig.poll().is_empty(),
        "no decision was recorded, so nothing is released"
    );
    rig.assert_not_launched();
}

#[test]
fn failed_audit_launches_nothing_and_does_not_spend_the_grant() {
    let mut rig = Rig::new();
    assert!(rig
        .submit("a", "fixture.echo", json!({"word": "x"}))
        .is_empty());
    rig.decide(ApprovalChoice::Once);
    rig.monitor.fail_audit(true);
    let reply = rig.replies(1).remove(0);
    rig.monitor.fail_audit(false);
    assert_eq!(code_of(&reply), "audit_failed", "{reply}");
    rig.assert_not_launched();
}

#[test]
fn always_survives_restart_and_session_does_not() {
    let mut rig = Rig::new();
    rig.remember(ApprovalChoice::Always);
    rig.service = CliOpService::default();
    rig.monitor = PermissionMonitor::reload_for_test(rig.profile.path());
    assert_eq!(rig.run_ok("after", "restart")["ok"], true);
    assert!(rig.pending().is_empty());

    let mut rig = Rig::new();
    rig.remember(ApprovalChoice::Session);
    rig.service = CliOpService::default();
    rig.monitor = PermissionMonitor::reload_for_test(rig.profile.path());
    assert!(rig
        .submit("after", "fixture.echo", json!({"word": "x"}))
        .is_empty());
    assert_eq!(
        rig.pending().len(),
        1,
        "a session grant does not survive restart"
    );
}

#[test]
fn consent_is_not_confinement() {
    // The grant authorizes an operation; it does not sandbox the program. A
    // read operation can read any file the user can, including outside the
    // workspace, and the approval says so.
    let mut rig = Rig::new();
    let outside = rig.root.path().join("outside-secret.txt");
    std::fs::write(&outside, "outside the workspace").unwrap();
    assert!(rig
        .submit(
            "r",
            "fixture.read",
            json!({"path": outside.to_str().unwrap()})
        )
        .is_empty());
    assert!(rig.pending()[0]
        .input_summary
        .contains("does not confine it"));
    rig.decide(ApprovalChoice::Once);
    let reply = rig.replies(1).remove(0);
    assert_eq!(reply["data"]["content"], "outside the workspace", "{reply}");
}

// ---- Installer lifecycle: replacement and interruption fail closed ----

#[test]
fn an_installed_package_runs_and_a_missing_install_marker_fails_closed() {
    let mut rig = Rig::installed();
    assert!(rig
        .app_dir
        .join(crate::app::package::INSTALL_COMPLETE_MARKER)
        .is_file());
    assert_eq!(rig.run_once_pair()["ok"], true);
    let launched = rig.launches().len();
    // An interrupted or pre-marker install has no completion marker.
    std::fs::remove_file(
        rig.app_dir
            .join(crate::app::package::INSTALL_COMPLETE_MARKER),
    )
    .unwrap();
    let reply = rig.submit("m", "fixture.echo", json!({"word": "x"}));
    assert_eq!(code_of(&reply[0]), "package_invalid", "{}", reply[0]);
    assert!(reply[0]["error"].as_str().unwrap().contains("reinstall"));
    assert!(rig.pending().is_empty());
    assert_eq!(rig.launches().len(), launched);
}

#[test]
fn an_updated_package_asks_again_and_restored_content_matches_the_old_grant() {
    let mut rig = Rig::installed();
    rig.remember(ApprovalChoice::Always);
    let launched = rig.launches().len();

    // A new version arrives through the installer with changed content.
    let v2 = rig.root.path().join("v2");
    std::fs::create_dir_all(&v2).unwrap();
    std::fs::write(v2.join("manifest.toml"), MANIFEST).unwrap();
    std::fs::write(v2.join("main.py"), "# entry\nprint('v2')\n").unwrap();
    assert_eq!(rig.install_from(&v2), 0);
    assert!(rig
        .submit("u", "fixture.echo", json!({"word": "x"}))
        .is_empty());
    assert_eq!(rig.pending().len(), 1, "an updated package must ask again");
    assert_eq!(
        rig.launches().len(),
        launched,
        "the update launched nothing unasked"
    );

    // Reinstalling the original bytes restores the identity the grant bound.
    let v1 = rig.root.path().join("v1");
    std::fs::create_dir_all(&v1).unwrap();
    std::fs::write(v1.join("manifest.toml"), MANIFEST).unwrap();
    std::fs::write(v1.join("main.py"), "# entry\n").unwrap();
    assert_eq!(rig.install_from(&v1), 0);
    rig.service = CliOpService::default();
    assert_eq!(rig.run_ok("again", "ok")["ok"], true);
}

#[test]
fn an_interrupted_update_keeps_the_previous_install_and_its_grant() {
    use std::os::unix::fs::PermissionsExt;
    let mut rig = Rig::installed();
    rig.remember(ApprovalChoice::Always);
    let before = crate::app::package::content_digest(&rig.app_dir).unwrap();

    let broken = rig.root.path().join("broken");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("manifest.toml"), MANIFEST).unwrap();
    std::fs::write(broken.join("main.py"), "# entry\nprint('new')\n").unwrap();
    let unreadable = broken.join("asset.bin");
    std::fs::write(&unreadable, "x").unwrap();
    std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o000)).unwrap();
    let code = rig.install_from(&broken);
    std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(code, 1, "a copy that fails part way must fail the install");

    assert_eq!(
        crate::app::package::content_digest(&rig.app_dir).unwrap(),
        before,
        "the previous install is untouched"
    );
    assert!(rig
        .app_dir
        .join(crate::app::package::INSTALL_COMPLETE_MARKER)
        .is_file());
    let staging_root = crate::app::install_swap::staging_root();
    let leftovers = std::fs::read_dir(&staging_root)
        .map(|d| d.flatten().count())
        .unwrap_or(0);
    assert_eq!(leftovers, 0, "no half-copied tree may remain");
    // The previous install still serves its remembered grant.
    assert_eq!(rig.run_ok("still", "ok")["ok"], true);
}
