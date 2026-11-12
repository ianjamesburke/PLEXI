//! Grant semantics for typed CLI operations (stint 1175). A grant binds package
//! content and the operation binding digest; it never binds the pane that asked.

use std::path::{Path, PathBuf};

use super::gate::{Admission, AdmitRequest, ApprovalChoice, PermissionMonitor};
use super::{ActorScope, TargetType};

struct Call {
    tool: &'static str,
    workspace: PathBuf,
    package: &'static str,
    binding: &'static str,
    values: &'static str,
    pane: u64,
    context: u64,
}

impl Call {
    fn new(workspace: &Path) -> Self {
        Self {
            tool: "cli.op.mutate",
            workspace: workspace.to_path_buf(),
            package: "pkg-a",
            binding: "task.set#bind-1",
            values: r#"{"priority":"p1"}"#,
            pane: 4,
            context: 9,
        }
    }

    fn input(&self) -> String {
        format!(
            r#"{{"cli_operation_id":"task.set","binding":"{}","values":{}}}"#,
            self.binding, self.values
        )
    }

    fn admit(&self, monitor: &PermissionMonitor) -> Admission {
        monitor.admit_cli_operation(
            AdmitRequest {
                call_id: "call-1",
                tool: self.tool,
                input_json: &self.input(),
                principal: &crate::broker::principal::Principal::plugin("board", Some(self.pane)),
                actor_scope: ActorScope::User,
                trust_origin: "host",
                workspace_root: &self.workspace,
                context_id: self.context,
                package_id: self.package,
                instance_id: self.pane,
                target_type: TargetType::CliOperation,
            },
            "Board wants to run task.set",
        )
    }
}

fn pending(adm: Admission) -> String {
    match adm {
        Admission::Required { pending_request_id } => pending_request_id,
        Admission::Proceed { .. } => panic!("expected an approval request, got Proceed"),
        Admission::Denied { code } => panic!("expected an approval request, got Denied({code})"),
    }
}

fn proceeds(adm: Admission) -> String {
    match adm {
        Admission::Proceed { grant_id, .. } => grant_id,
        Admission::Required { .. } => panic!("expected Proceed, got an approval request"),
        Admission::Denied { code } => panic!("expected Proceed, got Denied({code})"),
    }
}

fn monitor() -> (
    tempfile::TempDir,
    tempfile::TempDir,
    std::sync::Arc<PermissionMonitor>,
) {
    let profile = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();
    let monitor = PermissionMonitor::for_profile(profile.path()).unwrap();
    (profile, ws, monitor)
}

#[test]
fn unapproved_operation_asks_and_shows_the_disclosure() {
    let (_p, ws, monitor) = monitor();
    let call = Call::new(ws.path());
    let id = pending(call.admit(&monitor));
    let view = monitor.show_pending(&id).expect("pending is listed");
    assert_eq!(view.tool, "cli.op.mutate");
    assert_eq!(view.input_summary, "Board wants to run task.set");
    assert_eq!(view.resource_id.as_deref(), Some("task.set#bind-1"));
}

#[test]
fn once_authorizes_exactly_one_invocation() {
    let (_p, ws, monitor) = monitor();
    let call = Call::new(ws.path());
    let id = pending(call.admit(&monitor));
    monitor.approve_pending(&id, ApprovalChoice::Once).unwrap();
    let grant = proceeds(call.admit(&monitor));
    monitor.consume_once(&grant, None);
    pending(call.admit(&monitor));
}

#[test]
fn once_does_not_cover_different_values() {
    let (_p, ws, monitor) = monitor();
    let mut call = Call::new(ws.path());
    let id = pending(call.admit(&monitor));
    monitor.approve_pending(&id, ApprovalChoice::Once).unwrap();
    call.values = r#"{"priority":"p0"}"#;
    pending(call.admit(&monitor));
}

#[test]
fn remembered_grant_covers_other_values_but_only_the_same_binding() {
    let (_p, ws, monitor) = monitor();
    let mut call = Call::new(ws.path());
    let id = pending(call.admit(&monitor));
    monitor
        .approve_pending(&id, ApprovalChoice::Session)
        .unwrap();
    proceeds(call.admit(&monitor));

    call.values = r#"{"priority":"p0"}"#;
    proceeds(call.admit(&monitor));

    // A pane or context id change must not matter.
    call.pane = 77;
    call.context = 12;
    proceeds(call.admit(&monitor));

    let base = Call::new(ws.path());
    for (label, changed) in [
        (
            "operation binding",
            Call {
                binding: "task.set#bind-2",
                ..Call::new(ws.path())
            },
        ),
        (
            "package content",
            Call {
                package: "pkg-b",
                ..Call::new(ws.path())
            },
        ),
        (
            "mode",
            Call {
                tool: "cli.op.read",
                ..Call::new(ws.path())
            },
        ),
        ("workspace", Call::new(tempfile::tempdir().unwrap().path())),
        (
            "missing binding",
            Call {
                binding: "",
                ..Call::new(ws.path())
            },
        ),
    ] {
        let _ = &base;
        match changed.admit(&monitor) {
            Admission::Required { .. } => {}
            Admission::Proceed { .. } => panic!("changed {label} must not inherit the grant"),
            Admission::Denied { code } => panic!("changed {label} denied {code}"),
        }
    }
}

#[test]
fn remembered_grant_is_not_arbitrary_argv_authority() {
    // Values are not part of the broker match, so the host must have validated
    // them against the definition first. The broker still refuses any request
    // whose binding differs, even with otherwise identical values.
    let (_p, ws, monitor) = monitor();
    let mut call = Call::new(ws.path());
    let id = pending(call.admit(&monitor));
    monitor
        .approve_pending(&id, ApprovalChoice::Always)
        .unwrap();
    call.binding = "task.set#bind-with-widened-definition";
    pending(call.admit(&monitor));
}

#[test]
fn revoked_grant_asks_again() {
    let (_p, ws, monitor) = monitor();
    let call = Call::new(ws.path());
    let id = pending(call.admit(&monitor));
    monitor
        .approve_pending(&id, ApprovalChoice::Session)
        .unwrap();
    let grant = proceeds(call.admit(&monitor));
    assert!(monitor.revoke_grant_id(&grant));
    pending(call.admit(&monitor));
}

#[test]
fn denial_is_not_a_grant_and_deny_always_is_remembered() {
    let (_p, ws, monitor) = monitor();
    let call = Call::new(ws.path());
    let id = pending(call.admit(&monitor));
    monitor.approve_pending(&id, ApprovalChoice::Deny).unwrap();
    let id = pending(call.admit(&monitor));
    monitor
        .approve_pending(&id, ApprovalChoice::DenyAlways)
        .unwrap();
    assert!(matches!(call.admit(&monitor), Admission::Denied { .. }));
}

#[test]
fn always_survives_restart_and_session_does_not() {
    let profile = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();
    let monitor = PermissionMonitor::for_profile(profile.path()).unwrap();
    let always = Call::new(ws.path());
    let session = Call {
        binding: "task.show#bind-s",
        tool: "cli.op.read",
        ..Call::new(ws.path())
    };
    let id = pending(always.admit(&monitor));
    monitor
        .approve_pending(&id, ApprovalChoice::Always)
        .unwrap();
    let id = pending(session.admit(&monitor));
    monitor
        .approve_pending(&id, ApprovalChoice::Session)
        .unwrap();
    proceeds(always.admit(&monitor));
    proceeds(session.admit(&monitor));
    drop(monitor);

    let restarted = PermissionMonitor::reload_for_test(profile.path());
    proceeds(always.admit(&restarted));
    pending(session.admit(&restarted));
}

#[test]
fn tampered_grant_store_does_not_authorize() {
    let _exclusive = crate::broker::host_key::ExclusiveHostKey::acquire();
    let profile = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();
    let monitor = PermissionMonitor::for_profile(profile.path()).unwrap();
    let call = Call::new(ws.path());
    let id = pending(call.admit(&monitor));
    monitor
        .approve_pending(&id, ApprovalChoice::Always)
        .unwrap();
    proceeds(call.admit(&monitor));
    drop(monitor);

    let path = profile.path().join("grants.toml");
    let body = std::fs::read_to_string(&path).unwrap();
    assert!(body.contains("cli.op.mutate"), "{body}");
    std::fs::write(
        &path,
        body.replace("task.set#bind-1", "task.set#bind-forged"),
    )
    .unwrap();
    let restarted = PermissionMonitor::reload_for_test(profile.path());
    let forged = Call {
        binding: "task.set#bind-forged",
        ..Call::new(ws.path())
    };
    assert!(
        !matches!(forged.admit(&restarted), Admission::Proceed { .. }),
        "an edited grant file must never authorize"
    );
    assert!(
        !matches!(call.admit(&restarted), Admission::Proceed { .. }),
        "the sealed row no longer verifies"
    );
}
