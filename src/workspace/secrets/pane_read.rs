//! Workspace and global secret reads asked for from a terminal pane.
//!
//! Any process in a pane may be an agent, so `plexi secret get` from a pane
//! does not read the store itself. It asks the host, which admits tool
//! `secret.get` for the host-stamped pane identity. Without a grant the read
//! files a click approval and returns its pending id; the store is read only
//! after a desktop click. Logs and audit rows carry the name and scope, never
//! the value.

use std::path::{Path, PathBuf};

use super::folder::{ReadResult, SecretActor};
use super::store::NonDestructiveStore;
use crate::broker::{gate::PermissionMonitor, PermissionPosture};

/// Tool name the permission monitor admits for a pane's workspace or global read.
pub const SECRET_GET_TOOL: &str = "secret.get";

/// Where `plexi secret get` looks: a workspace (then the global fallback), or global only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretScope {
    Workspace { id: String, root: PathBuf },
    Global,
}

impl SecretScope {
    /// The resource label in the grant binding: the workspace root, or `global`.
    fn label(&self) -> String {
        match self {
            Self::Workspace { root, .. } => root.to_string_lossy().into_owned(),
            Self::Global => "global".to_string(),
        }
    }
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
) -> Result<ReadResult, String> {
    read_pane_secret_with_posture(monitor, store, actor, name, scope, global_root, None)
}

/// As [`read_pane_secret`], evaluating the immutable posture carried by the
/// requesting worker pane.
pub fn read_pane_secret_with_posture(
    monitor: &PermissionMonitor,
    store: &dyn NonDestructiveStore,
    actor: &SecretActor,
    name: &str,
    scope: &SecretScope,
    global_root: &Path,
    posture: Option<&PermissionPosture>,
) -> Result<ReadResult, String> {
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
    let input = serde_json::json!({"name": name, "scope": label}).to_string();
    let workspace_root = match scope {
        SecretScope::Workspace { root, .. } => root.as_path(),
        SecretScope::Global => global_root,
    };
    super::folder::admitted_read_with_posture(
        monitor,
        actor,
        SECRET_GET_TOOL,
        &input,
        workspace_root,
        &format!("name={name} scope={label}"),
        posture,
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
    use crate::workspace::secrets::{InMemoryKeychain, SecretStore};

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
        let first = read_pane_secret(&monitor, &store, &actor, "SHARED_0788", &scope, root.path())
            .unwrap();
        let ReadResult::PermissionRequired { pending_id } = first else {
            panic!("expected a pending approval, got {first:?}");
        };
        monitor.approve_pending(&pending_id, ApprovalChoice::Once).unwrap();
        let read = read_pane_secret(&monitor, &store, &actor, "SHARED_0788", &scope, root.path())
            .unwrap();
        assert!(matches!(read, ReadResult::Value(ref value) if value.as_str() == "global-canary-0788"));
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
