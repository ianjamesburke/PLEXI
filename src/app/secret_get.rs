//! Host side of `plexi secret get` from a terminal pane.
//!
//! The caller is the pane named by the socket peer's ancestry, never a claim
//! in the request. The read goes through the permission monitor, so without
//! a grant the desktop banner shows the ask and the reply carries only its
//! pending id. Contract: `src/cli/AGENTS.md`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::PlexiApp;
use crate::workspace::secrets::{ReadResult, SecretActor, SecretScope};

/// A reply that carries a value is removed after this long even when the
/// caller could not delete it (a sandboxed agent cannot write the profile).
const VALUE_REPLY_TTL: Duration = Duration::from_secs(15);

impl PlexiApp {
    pub(crate) fn serve_secret_get(
        &mut self,
        name: &str,
        global: bool,
        workspace_root: Option<&Path>,
        peer_ancestry: &[u32],
        response_file: &str,
    ) {
        let reply_path = PathBuf::from(response_file);
        let rpc_dir = crate::config::config_dir().join("rpc");
        if reply_path.parent() != Some(rpc_dir.as_path()) {
            log::warn!("secret_get: refused reply path outside {}", rpc_dir.display());
            return;
        }
        let monitor =
            crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir());
        let Some((pane_id, _, _)) = self.resolve_socket_peer_pane(peer_ancestry) else {
            monitor.note_denial("session:unscoped", "secret-get", name, "", "identity");
            log::info!("secret_get: denied name={name} caller resolves to no pane");
            write_reply(&reply_path, serde_json::json!({"status": "denied"}), false);
            return;
        };
        let scope = match scope_for(global, workspace_root) {
            Ok(scope) => scope,
            Err(error) => {
                log::warn!("secret_get: workspace config unreadable: {error}");
                write_reply(&reply_path, serde_json::json!({"status": "error", "error": error}), false);
                return;
            }
        };
        let actor = SecretActor {
            kind: crate::broker::ActorType::Agent,
            id: format!("pane:{pane_id}"),
        };
        log::info!(
            "secret_get: pane={pane_id} name={name} scope={}",
            match &scope {
                SecretScope::Workspace { root, .. } => root.display().to_string(),
                SecretScope::Global => "global".to_string(),
            }
        );
        let result = crate::workspace::secrets::read_pane_secret(
            &monitor,
            crate::workspace::secrets::system_store(),
            &actor,
            name,
            &scope,
            &crate::config::config_dir(),
        );
        let (body, holds_value) = match result {
            Ok(ReadResult::Value(value)) => {
                (serde_json::json!({"status": "value", "value": value.as_str()}), true)
            }
            Ok(ReadResult::PermissionRequired { pending_id }) => (
                serde_json::json!({"status": "permission_required", "pending_request_id": pending_id}),
                false,
            ),
            Ok(ReadResult::Denied) => (serde_json::json!({"status": "denied"}), false),
            Ok(ReadResult::NotFound) => (serde_json::json!({"status": "not_found"}), false),
            Err(error) => (serde_json::json!({"status": "error", "error": error}), false),
        };
        write_reply(&reply_path, body, holds_value);
    }
}

/// The workspace whose `workspace.toml` sits at `root`, else global only, the
/// same order `plexi secret get` uses outside a pane.
fn scope_for(global: bool, workspace_root: Option<&Path>) -> Result<SecretScope, String> {
    let Some(root) = workspace_root.filter(|_| !global) else {
        return Ok(SecretScope::Global);
    };
    Ok(match crate::workspace::secrets::WorkspaceConfig::load(root)? {
        Some(cfg) => SecretScope::Workspace {
            id: cfg.id,
            root: crate::platform::path::canonical_or_self(root),
        },
        None => SecretScope::Global,
    })
}

fn write_reply(path: &Path, body: serde_json::Value, holds_value: bool) {
    if let Err(error) =
        crate::platform::fs::atomic_write_with_mode(path, body.to_string().as_bytes(), 0o600)
    {
        log::error!("secret_get: reply write to {} failed: {error}", path.display());
        return;
    }
    if holds_value {
        let path = path.to_path_buf();
        let spawned = std::thread::Builder::new()
            .name("secret-get-reply-expiry".into())
            .spawn(move || {
                std::thread::sleep(VALUE_REPLY_TTL);
                match std::fs::remove_file(&path) {
                    Ok(()) => log::info!("secret_get: removed unread value reply"),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => log::warn!("secret_get: value reply cleanup failed: {error}"),
                }
            });
        if let Err(error) = spawned {
            log::warn!("secret_get: value reply expiry thread failed: {error}");
        }
    }
}
