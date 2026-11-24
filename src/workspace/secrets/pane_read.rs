//! Workspace and global secret reads asked for from a terminal pane.
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

/// Where `plexi secret get` looks: a workspace (then the global fallback), or global only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretScope {
    Workspace { id: String, root: PathBuf },
    Global,
}

impl SecretScope {
    /// Arguments covered by a standing grant. The broker binds Once to the call ID.
    pub(crate) fn input(&self, name: &str, outside: Option<&str>) -> serde_json::Value {
        let mut input = serde_json::json!({"name": name, "scope": self.label()});
        if let Some(caller) = outside {
            input["caller"] = caller.into();
        }
        input
    }

    /// The resource label in the grant binding: the workspace root, or `global`.
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Workspace { root, .. } => root.to_string_lossy().into_owned(),
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
        SecretScope::Global => vec![super::keychain_user_name(name)],
    };
    let label = scope.label();
    let input = scope.input(name, outside);
    let workspace_root = match scope {
        SecretScope::Workspace { root, .. } => root.as_path(),
        SecretScope::Global => global_root,
    };
    super::folder::admitted_read_with(
        monitor,
        actor,
        SECRET_GET_TOOL,
        &input.to_string(),
        workspace_root,
        &format!("name={name} scope={label}"),
        admit,
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
