//! Admitted workspace, global and folder reads on the host command socket.
//!
//! Any process in a pane may be an agent, so `plexi secret get` from a pane
//! does not read the store itself. It asks the host, which admits tool
//! `secret.get` for the host-stamped pane identity. Without a grant the read
//! waits for a desktop decision on the original connection. Logs and audit
//! rows carry the name and scope.

use std::path::{Path, PathBuf};

use super::folder::SecretActor;
use super::store::NonDestructiveStore;
use crate::broker::gate::{AdmitRequest, PermissionMonitor, UseDecision};

/// Tool name the permission monitor admits for a pane's workspace or global read.
pub const SECRET_GET_TOOL: &str = "secret.get";

/// Account selection and grant scope for a command-socket read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretScope {
    Workspace { id: String, root: PathBuf },
    Folder { path: PathBuf },
    Global,
}

impl SecretScope {
    fn read_tool(&self) -> &'static str {
        match self {
            Self::Folder { .. } => super::SECRET_READ_TOOL,
            _ => SECRET_GET_TOOL,
        }
    }

    pub(crate) fn store(&self) -> &'static dyn super::SecretStore {
        match self {
            Self::Folder { .. } => super::folder_store(),
            _ => super::system_store(),
        }
    }

    /// Arguments covered by a standing grant. The broker binds Once to the call ID.
    pub(crate) fn input(&self, tool: &str, name: &str, outside: Option<&str>) -> serde_json::Value {
        // A saved folder-read decision is keyed `NAME@<path>` since before the
        // host owned this verb; keep that key so it still applies. Set and
        // delete share the workspace key space, so a folder is prefixed there.
        let (place, label) = match (tool == super::SECRET_READ_TOOL, self) {
            (true, Self::Folder { path }) => ("folder", path.to_string_lossy().into_owned()),
            (true, _) => ("folder", self.label()),
            (false, _) => ("scope", self.label()),
        };
        let mut input = serde_json::json!({"name": name, (place): label});
        if let Some(caller) = outside {
            input["caller"] = caller.into();
        }
        input
    }

    /// The resource label in the grant binding.
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Workspace { root, .. } => root.to_string_lossy().into_owned(),
            Self::Folder { path } => format!("folder:{}", path.display()),
            Self::Global => "global".to_string(),
        }
    }
}

/// The secret a read names: its name, scope, and the profile root used when
/// the scope is global.
#[derive(Clone, Copy)]
pub struct ReadTarget<'a> {
    pub name: &'a str,
    pub scope: &'a SecretScope,
    pub global_root: &'a Path,
}

/// Admit `secret.get` for `actor`, then read `name` the way `plexi secret get`
/// does: the workspace account first, then `plexi:user:<name>`.
#[cfg(test)]
pub fn read_pane_secret(
    monitor: &PermissionMonitor,
    store: &dyn NonDestructiveStore,
    actor: &SecretActor,
    name: &str,
    scope: &SecretScope,
    global_root: &Path,
) -> Result<super::folder::ReadResult, String> {
    read_pane_secret_with(
        monitor,
        store,
        actor,
        ReadTarget {
            name,
            scope,
            global_root,
        },
        None,
        |req| Some(monitor.admit_for_use(req, None)),
    )
    .map(|read| read.expect("synchronous admission").result)
}

/// As [`read_pane_secret`], evaluating the immutable posture carried by the
/// requesting worker pane, and returning the grant that admitted the read.
pub fn read_pane_secret_with(
    monitor: &PermissionMonitor,
    store: &dyn NonDestructiveStore,
    actor: &SecretActor,
    target: ReadTarget<'_>,
    outside: Option<&str>,
    admit: impl FnOnce(AdmitRequest) -> Option<UseDecision>,
) -> Result<Option<super::folder::GrantedRead>, String> {
    let ReadTarget {
        name,
        scope,
        global_root,
    } = target;
    let accounts = match scope {
        SecretScope::Workspace { id, .. } => {
            let account = super::keychain_workspace_name(id, name);
            if super::workspace_id_is_reserved(id) || super::secret_account_is_reserved(&account) {
                return Err(super::reserved_secret_error(id));
            }
            vec![account, super::keychain_user_name(name)]
        }
        SecretScope::Folder { path } => vec![super::folder::account_name(path, name)],
        SecretScope::Global => vec![super::keychain_user_name(name)],
    };
    let label = scope.label();
    let tool = scope.read_tool();
    let input = scope.input(tool, name, outside);
    let workspace_root = match scope {
        SecretScope::Workspace { root, .. } | SecretScope::Folder { path: root } => root.as_path(),
        SecretScope::Global => global_root,
    };
    super::folder::admitted_read_with(
        monitor,
        actor,
        tool,
        &input.to_string(),
        workspace_root,
        &format!("name={name} scope={label}"),
        |mut req| {
            if matches!(scope, SecretScope::Folder { .. }) {
                req.trust_origin = crate::broker::SECRET_SOCKET_ORIGIN;
            }
            admit(req)
        },
        || {
            for account in &accounts {
                if let Some(value) = store.get_interactive(account)? {
                    return Ok(Some(value));
                }
            }
            Ok(None)
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::gate::ApprovalChoice;
    use crate::workspace::secrets::{InMemoryKeychain, ReadResult, SecretStore};

    #[test]
    fn global_fallback_needs_the_same_click() {
        let store = InMemoryKeychain::new();
        store.set("plexi:user:SHARED_0788", "global-canary-0788").unwrap();
        let monitor = PermissionMonitor::ephemeral();
        let actor = SecretActor {
            kind: crate::broker::ActorType::Agent,
            id: "pane:3".to_string(),
        };
        let root = tempfile::tempdir().unwrap();
        let scope = SecretScope::Workspace {
            id: "ws-0788".to_string(),
            root: root.path().to_path_buf(),
        };
        let read = read_pane_secret_with(
            &monitor,
            &store,
            &actor,
            ReadTarget {
                name: "SHARED_0788",
                scope: &scope,
                global_root: root.path(),
            },
            None,
            |req| {
                let UseDecision::Required { pending_request_id } = monitor.admit_for_use(req, None)
                else {
                    panic!("expected a pending approval")
                };
                monitor
                    .approve_pending(&pending_request_id, ApprovalChoice::Once)
                    .unwrap();
                Some(monitor.admit_for_use_if(req, None, Some(&|| true)))
            },
        )
        .unwrap()
        .unwrap();
        assert!(
            matches!(read.result, ReadResult::Value(ref value) if value.as_str() == "global-canary-0788")
        );
    }

    /// A deny saved by the desktop folder reader is keyed `NAME@<path>` with
    /// the `host` origin. The same reader must still be refused by it.
    #[test]
    fn a_saved_always_deny_on_the_old_folder_key_still_refuses() {
        use crate::broker::gate::Admission;
        use crate::workspace::secrets::{read_folder_secret, take_test_store_calls};
        let store = InMemoryKeychain::new();
        let monitor = PermissionMonitor::ephemeral();
        let actor = SecretActor::agent("me");
        let root = tempfile::tempdir().unwrap();
        let path = root.path().canonicalize().unwrap();
        // The key alpha wrote, spelled out so a format change cannot follow it.
        let old_input =
            serde_json::json!({"name": "OLD_DENY", "folder": path.to_string_lossy()}).to_string();
        let principal =
            crate::broker::principal::Principal::from_actor(actor.kind, &actor.id, None).unwrap();
        let saved = AdmitRequest {
            call_id: "before-upgrade",
            tool: super::super::SECRET_READ_TOOL,
            input_json: &old_input,
            principal: &principal,
            actor_scope: crate::broker::ActorScope::User,
            trust_origin: "host",
            workspace_root: &path,
            context_id: 0,
            package_id: "",
            instance_id: 0,
            target_type: crate::broker::TargetType::Secret,
        };
        let Admission::Required { pending_request_id } = monitor.admit_with_posture(saved, None)
        else {
            panic!("expected a pending approval");
        };
        monitor
            .approve_pending(&pending_request_id, ApprovalChoice::DenyAlways)
            .unwrap();
        take_test_store_calls();
        let read = read_folder_secret(&monitor, &store, &actor, "OLD_DENY", &path).unwrap();
        assert!(matches!(read, ReadResult::Denied), "{read:?}");
        assert_eq!(take_test_store_calls(), 0);
        assert!(monitor.open_needs_you().is_empty());
    }

    #[test]
    fn folder_read_reserves_its_click_and_reads_one_account() {
        use crate::broker::gate::Admission;
        use crate::workspace::secrets::take_test_store_calls;
        let store = InMemoryKeychain::new();
        store.set("plexi:user:SHARED", "global-canary").unwrap();
        let monitor = PermissionMonitor::ephemeral();
        let actor = SecretActor {
            kind: crate::broker::ActorType::External,
            id: crate::broker::principal::EXTERNAL_ID.into(),
        };
        let root = tempfile::tempdir().unwrap();
        let scope = SecretScope::Folder {
            path: root.path().canonicalize().unwrap(),
        };
        take_test_store_calls();
        let read = read_pane_secret_with(
            &monitor,
            &store,
            &actor,
            ReadTarget {
                name: "SHARED",
                scope: &scope,
                global_root: root.path(),
            },
            Some("a terminal outside Plexi"),
            |req| {
                let other = AdmitRequest {
                    call_id: "other-live-read",
                    ..req
                };
                let pending = |ask| match monitor.admit_with_posture(ask, None) {
                    Admission::Required { pending_request_id } => pending_request_id,
                    _ => panic!("expected an independent card"),
                };
                let first = pending(req);
                let second = pending(other);
                assert_ne!(first, second);
                assert_eq!(take_test_store_calls(), 0);
                monitor
                    .approve_pending(&first, ApprovalChoice::Once)
                    .unwrap();
                assert_eq!(pending(other), second);
                monitor
                    .approve_pending(&second, ApprovalChoice::Deny)
                    .unwrap();
                Some(monitor.admit_for_use_if(req, None, Some(&|| true)))
            },
        )
        .unwrap()
        .unwrap();
        assert!(matches!(read.result, ReadResult::NotFound));
        assert_eq!(take_test_store_calls(), 1);
        assert!(read.approved.is_some());
        assert!(monitor.open_needs_you().is_empty());
    }

    #[test]
    fn reserved_workspace_id_is_refused_before_the_gate() {
        let store = InMemoryKeychain::new();
        let monitor = PermissionMonitor::ephemeral();
        let actor = SecretActor {
            kind: crate::broker::ActorType::Agent,
            id: "pane:3".to_string(),
        };
        let scope = SecretScope::Workspace {
            id: "host".to_string(),
            root: PathBuf::from("/tmp"),
        };
        assert!(read_pane_secret(&monitor, &store, &actor, "permission-mac", &scope, Path::new("/tmp")).is_err());
        assert!(monitor.list_pending().is_empty());
    }
}
