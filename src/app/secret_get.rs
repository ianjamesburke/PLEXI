//! Host side of `plexi secret get` from a terminal pane.
//!
//! The caller is the pane named by the socket peer's ancestry, never a claim
//! in the request. The read goes through the permission monitor, so without
//! a grant the desktop banner shows the ask and the reply carries only its
//! pending id. Contract: `src/cli/AGENTS.md`.

use std::path::{Path, PathBuf};

use super::PlexiApp;
use crate::workspace::secrets::{ReadResult, SecretActor, SecretScope};

impl PlexiApp {
    pub(crate) fn serve_secret_list(
        &mut self,
        context: &str,
        ancestry: &[u32],
        response_file: &str,
    ) {
        let result = self.context_secret_inventory(context, ancestry);
        let body = match result {
            Ok(inventory) => serde_json::to_value(inventory)
                .unwrap_or_else(|error| serde_json::json!({"error": error.to_string()})),
            Err(error) => {
                log::info!("secret_list: refused context={context:?}: {error}");
                serde_json::json!({"error": error, "error_code": "permission_denied"})
            }
        };
        crate::rpc::write_json_response(response_file, body);
    }

    fn context_secret_inventory(
        &self,
        context: &str,
        ancestry: &[u32],
    ) -> Result<crate::workspace::secrets::manager::SecretInventory, String> {
        use crate::broker::gate::{Admission, AdmitRequest, PermissionMonitor};
        use crate::broker::{ActorScope, ActorType, TargetType};
        let candidates: Vec<_> = self
            .router
            .iter()
            .filter(|ctx| {
                context.parse::<u64>().ok() == Some(ctx.context_id)
                    || ctx.name.displayed() == context
                    || crate::platform::path::canonical_or_self(Path::new(context))
                        == crate::platform::path::canonical_or_self(&ctx.root)
            })
            .collect();
        let [ctx] = candidates.as_slice() else {
            return Err(format!("Context '{context}' is missing or ambiguous. Use its id or root from `plexi context list`."));
        };
        let root = crate::platform::path::canonical_or_self(&ctx.root);
        let Some((pane, _, _)) = self.resolve_socket_peer_pane(ancestry) else {
            return Err("permission_denied: caller identity could not be verified; run --context from a human-owned Plexi pane.".into());
        };
        log::info!(
            "secret_list: explicit context={} pane={pane} human={}",
            ctx.context_id,
            self.is_human_owned_pane(pane)
        );
        if self.is_human_owned_pane(pane) {
            return crate::workspace::secrets::manager::inventory_for_context(&root);
        }
        let monitor = PermissionMonitor::for_profile(&crate::config::config_dir());
        let actor = format!("pane:{pane}");
        let call = format!("secret-list-{}", uuid::Uuid::new_v4());
        let input = serde_json::json!({"path": root}).to_string();
        let posture = self.pane_permission_posture(pane);
        match monitor.admit_with_posture(AdmitRequest {
            call_id: &call, tool: "secret.list", input_json: &input,
            actor_type: ActorType::Agent, actor_id: &actor, actor_scope: ActorScope::User,
            trust_origin: "plexi-cli", workspace_root: &root, context_id: ctx.context_id,
            package_id: "", instance_id: 0, target_type: TargetType::Secret,
        }, posture.as_ref()) {
            Admission::Proceed { grant_id, fingerprint, resource_id, .. } => {
                let resource = resource_id.unwrap_or_default();
                monitor.note_use(&actor, &call, &grant_id, &fingerprint, &resource, "")?;
                let result = crate::workspace::secrets::manager::inventory_for_context(&root);
                monitor.note_outcome(&actor, &call, &grant_id, &fingerprint, &resource, "", if result.is_ok() { "ok" } else { "failed" }, "", "")?;
                monitor.consume_once(&grant_id, None);
                result
            }
            Admission::Required { pending_request_id } => Err(format!("permission_denied: missing secret.list grant for {} (actor {actor}). Ask the human to approve request {pending_request_id} in Needs you, then rerun this command.", root.display())),
            Admission::Denied { .. } => Err(format!("permission_denied: missing secret.list grant for {} (actor {actor}). Ask the human to review the denial in Permissions and approve a new request in Needs you.", root.display())),
        }
    }

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
        if !global {
            if let Some(root) = workspace_root {
                let root_canon = crate::platform::path::canonical_or_self(root);
                let pane_root = self.pane_context_authority_root(pane_id);
                if pane_root.as_ref() != Some(&root_canon)
                    || !self.pane_may_access_context_root(pane_id, root)
                {
                    let actor_id = format!("pane:{pane_id}");
                    let resource = format!("{name}@{}", root_canon.display());
                    monitor.note_denial(
                        &actor_id,
                        "secret-get",
                        &resource,
                        "",
                        "context_scope_violation",
                    );
                    log::info!(
                        "secret_get: denied pane={pane_id} name={name} target={} outside pane context root {pane_root:?}",
                        root_canon.display()
                    );
                    write_reply(
                        &reply_path,
                        serde_json::json!({"status": "denied", "error_code": "permission_denied"}),
                        false,
                    );
                    return;
                }
            }
        }
        let scope = match scope_for(global, workspace_root) {
            Ok(scope) => scope,
            Err(error) => {
                log::warn!("secret_get: workspace config unreadable: {error}");
                write_reply(
                    &reply_path,
                    serde_json::json!({"status": "error", "error": error}),
                    false,
                );
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
        // Keychain authentication can wait for a human. Keep servicing the
        // host while the exact admitted request finishes; no re-admission loop.
        let posture = self.pane_permission_posture(pane_id);
        let name = name.to_string();
        let profile = crate::config::config_dir();
        let failure_path = reply_path.clone();
        let repaint = self.ctx.clone();
        let read = move || {
            let result = crate::workspace::secrets::read_pane_secret_with_posture(
                &monitor,
                crate::workspace::secrets::system_store(),
                &actor,
                &name,
                &scope,
                &profile,
                posture.as_ref(),
            );
            let (body, holds_value) = match result {
                Ok(ReadResult::Value(value)) => (
                    serde_json::json!({"status": "value", "value": value.as_str()}),
                    true,
                ),
                Ok(ReadResult::PermissionRequired { pending_id }) => (
                    serde_json::json!({"status": "permission_required", "pending_request_id": pending_id}),
                    false,
                ),
                Ok(ReadResult::Denied) => (serde_json::json!({"status": "denied"}), false),
                Ok(ReadResult::NotFound) => (serde_json::json!({"status": "not_found"}), false),
                Err(error) => (
                    serde_json::json!({"status": "error", "error": error}),
                    false,
                ),
            };
            write_reply(&reply_path, body, holds_value);
            // Admission ran off-thread: wake an idle host to display a newly
            // pending banner (or remove a consumed one) without another input.
            repaint.request_repaint();
        };
        #[cfg(not(test))]
        if let Err(error) = std::thread::Builder::new()
            .name("secret-get".into())
            .spawn(read)
        {
            log::error!("secret_get: could not start read worker: {error}");
            write_reply(
                &failure_path,
                serde_json::json!({"status": "error", "error": "could not start secret read"}),
                false,
            );
        }
        #[cfg(test)]
        {
            let _ = failure_path;
            read();
        }
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
    let token = path.to_string_lossy();
    let wrote = if holds_value {
        crate::rpc::write_stream_only_json_response(&token, body)
    } else {
        crate::rpc::write_json_response(&token, body)
    };
    if !wrote {
        log::error!("secret_get: reply write for {} failed", path.display());
    }
}

#[cfg(test)]
mod list_tests {
    use crate::testing::HostHarness;
    use crate::workspace::secrets;

    fn request(h: &mut HostHarness, context: &str, claimed_pid: u32, peer: &[u32]) -> serde_json::Value {
        let response_file = crate::rpc::response_file("secret-list-test", "json");
        let request = crate::protocol::AppRequest::PaneSecretList {
            context: context.into(), peer_ancestry: vec![claimed_pid], response_file: response_file.clone(),
        };
        let line = serde_json::to_string(&request).unwrap();
        crate::app::handle_socket_line(&line, &h.ipc_tx, Some(peer), &mut Vec::new());
        h.hidden_frame();
        let body = std::fs::read_to_string(response_file).unwrap();
        assert!(!body.contains("not-for-output"));
        serde_json::from_str(&body).unwrap()
    }

    #[test]
    fn context_inventory_human_names_agent_requires_exact_grant() {
        let mut h = HostHarness::new();
        let pane = h.add_focused_terminal();
        let pid = h.terminal_backend(pane).child_pid();
        let other = tempfile::tempdir().unwrap();
        let root = other.path().to_path_buf();
        let mut target = h.app.router.active().clone();
        target.context_id = crate::testing::reserve_test_context_id();
        target.root = root.clone();
        target.name = crate::host::context::ContextName::custom("inventory-target");
        let context = target.context_id.to_string();
        h.app.router.push(target);
        let cfg = secrets::init_workspace(&root, &crate::config::workspace_channel_dir()).unwrap();
        let name = format!("CONTEXT_{}", uuid::Uuid::new_v4().simple());
        let account = secrets::keychain_workspace_name(&cfg.id, &name);
        secrets::system_store().set(&account, "not-for-output").unwrap();
        for target in [&context, "inventory-target", root.to_str().unwrap()] {
            let human = request(&mut h, target, pid, &[pid]);
            assert!(human["entries"].as_array().unwrap().iter().any(|entry| entry["name"] == name));
        }
        // The listener overwrites forged ancestry before dispatch, including while hidden.
        assert_eq!(request(&mut h, &context, pid, &[])["error_code"], "permission_denied");
        h.app.windows[0].panes.get_mut(&pane).unwrap().as_terminal_mut().unwrap().agent_spawned = true;
        let denied = request(&mut h, &context, pid, &[pid]);
        let error = denied["error"].as_str().unwrap();
        assert!(error.contains("secret.list") && error.contains("Needs you"));
        let monitor = crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir());
        let pending = monitor.list_entries().into_iter().find(|row| row.tool == "secret.list").unwrap();
        monitor.approve_pending(&pending.id, crate::broker::gate::ApprovalChoice::Once).unwrap();
        let allowed = request(&mut h, &context, pid, &[pid]);
        assert!(allowed["entries"].as_array().unwrap().iter().any(|entry| entry["name"] == name));
        assert_eq!(request(&mut h, &context, pid, &[pid])["error_code"], "permission_denied");
        secrets::system_store().delete(&account).unwrap();
    }

    fn write_agent_role(workspace: &std::path::Path, role: &str, permissions_toml: &str) {
        let dir = workspace.join(".plexi/agents").join(role);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("AGENT.md"), format!("You are {role}.")).unwrap();
        std::fs::write(
            dir.join("settings.toml"),
            format!(
                "[agent]\nid = \"{role}\"\ndisplay_name = \"{role}\"\n\n[harness]\ncli = \"pi\"\n\n{permissions_toml}\n"
            ),
        )
        .unwrap();
    }

    fn secret_get_request(h: &mut HostHarness, name: &str, peer: &[u32]) -> serde_json::Value {
        let response_file = crate::rpc::response_file("secret-get", "json");
        let request = crate::protocol::AppRequest::PaneSecretGet {
            name: name.into(),
            global: false,
            workspace_root: Some(h.workspace_root()),
            peer_ancestry: vec![],
            response_file: response_file.clone(),
        };
        let line = serde_json::to_string(&request).unwrap();
        let (mut server_stream, client_stream) = std::os::unix::net::UnixStream::pair().unwrap();
        crate::rpc::register_client_stream(response_file.clone(), client_stream);
        crate::app::handle_socket_line(&line, &h.ipc_tx, Some(peer), &mut server_stream);
        drop(server_stream);
        h.hidden_frame();
        let body = crate::rpc::poll_string(
            &response_file,
            Some(std::time::Duration::from_secs(2)),
        )
        .unwrap();
        assert!(
            !std::path::Path::new(&response_file).exists(),
            "secret_get must never create a response file on disk"
        );
        serde_json::from_str(&body).unwrap()
    }

    #[test]
    fn secret_get_returns_value_over_socket_stream_without_creating_rpc_file() {
        let mut h = HostHarness::new();
        let ws = h.workspace_root();
        let cfg = secrets::init_workspace(&ws, &crate::config::workspace_channel_dir()).unwrap();
        let secret_name = "STREAM_ONLY_SECRET";
        let secret_val = "plaintext-never-hits-disk-0970";
        let account = secrets::keychain_workspace_name(&cfg.id, secret_name);
        secrets::system_store().set(&account, secret_val).unwrap();

        write_agent_role(
            &ws,
            "worker-allow-secret",
            "[permissions]\ndefault_posture = \"allow\"\nallow = [\"secret.get\"]",
        );
        let (pane, _) = h
            .app
            .spawn_agent_worker_pane(
                "worker-allow-secret",
                "pi",
                "sleep 30",
                "Read secret",
                None,
                true,
                None,
                Some(ws.clone()),
                None,
                None,
                false,
                None,
            )
            .unwrap();
        let pid = h.terminal_backend(pane).child_pid();

        let rpc_dir = crate::config::config_dir().join("rpc");
        let ask = secret_get_request(&mut h, secret_name, &[pid]);
        assert_eq!(ask["status"], "permission_required", "{ask}");
        let pending_id = ask["pending_request_id"].as_str().unwrap();
        let monitor =
            crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir());
        monitor
            .approve_pending(pending_id, crate::broker::gate::ApprovalChoice::Once)
            .unwrap();

        let reply = secret_get_request(&mut h, secret_name, &[pid]);
        assert_eq!(reply["status"], "value", "{reply}");
        assert_eq!(reply["value"], secret_val);

        if let Ok(entries) = std::fs::read_dir(&rpc_dir) {
            for entry in entries.flatten() {
                let fname = entry.file_name();
                let name_str = fname.to_string_lossy();
                assert!(
                    !name_str.starts_with("secret-get-"),
                    "found unexpected secret-get file on disk: {}",
                    entry.path().display()
                );
            }
        }

        // Without a registered socket stream, value replies are refused rather than written to disk.
        let unbacked = crate::rpc::response_file("secret-get-unbacked", "json");
        super::write_reply(
            std::path::Path::new(&unbacked),
            serde_json::json!({"status": "value", "value": secret_val}),
            true,
        );
        assert!(
            !std::path::Path::new(&unbacked).exists(),
            "value reply without a socket stream must never fall back to disk"
        );

        secrets::system_store().delete(&account).unwrap();
    }

    #[test]
    fn worker_posture_deny_and_ask_apply_to_secret_get_and_pane_send_and_resist_mid_run_edits() {
        let mut h = HostHarness::new();
        let target_pane = h.add_focused_terminal();
        let ws = h.workspace_root();
        let cfg = secrets::init_workspace(&ws, &crate::config::workspace_channel_dir()).unwrap();
        let account = secrets::keychain_workspace_name(&cfg.id, "FOO");
        secrets::system_store().set(&account, "top-secret").unwrap();

        write_agent_role(
            &ws,
            "researcher-deny",
            "[permissions]\ndefault_posture = \"ask\"\ndeny = [\"secret.get\", \"pane.send\"]",
        );
        write_agent_role(
            &ws,
            "researcher-ask",
            "[permissions]\ndefault_posture = \"ask\"\nask = [\"secret.get\"]",
        );

        let (denied_pane, _) = h
            .app
            .spawn_agent_worker_pane(
                "researcher-deny",
                "pi",
                "sleep 30",
                "Check deny",
                None,
                true,
                None,
                Some(ws.clone()),
                None,
                None,
                false,
                None,
            )
            .unwrap();
        let denied_pid = h.terminal_backend(denied_pane).child_pid();

        // Editing settings.toml after spawn must not widen the live worker's snapshotted posture.
        write_agent_role(
            &ws,
            "researcher-deny",
            "[permissions]\ndefault_posture = \"allow\"\nallow = [\"secret.get\", \"pane.send\"]",
        );

        let monitor =
            crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir());
        let reply = secret_get_request(&mut h, "FOO", &[denied_pid]);
        assert_eq!(reply["status"], "denied", "{reply}");
        assert!(
            monitor.list_pending().is_empty(),
            "posture deny must not file an approval banner"
        );
        assert!(
            monitor.audit_records().iter().any(|row| {
                row.kind == "deny"
                    && row.actor == format!("pane:{denied_pane}")
                    && row.decision == "deny"
            }),
            "posture deny must record a denial audit row"
        );

        // `pane send` from a denied worker is refused before reaching the target pane.
        let send_resp = crate::rpc::response_file("pane-send-denied", "json");
        let send_req = crate::protocol::AppRequest::SendToPane {
            pane_id: target_pane,
            text: "echo hi".into(),
            submit: false,
            paste: false,
            response_file: Some(send_resp.clone()),
            peer_ancestry: None,
        };
        let send_line = serde_json::to_string(&send_req).unwrap();
        crate::app::handle_socket_line(&send_line, &h.ipc_tx, Some(&[denied_pid]), &mut Vec::new());
        h.hidden_frame();
        let send_reply: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(send_resp).unwrap()).unwrap();
        assert!(
            send_reply["error"]
                .as_str()
                .is_some_and(|e| e.contains("permission_denied")),
            "{send_reply}"
        );

        // `ask` posture shows one banner.
        let (ask_pane, _) = h
            .app
            .spawn_agent_worker_pane(
                "researcher-ask",
                "pi",
                "sleep 30",
                "Check ask",
                None,
                true,
                None,
                Some(ws.clone()),
                None,
                None,
                false,
                None,
            )
            .unwrap();
        let ask_pid = h.terminal_backend(ask_pane).child_pid();
        let ask_reply = secret_get_request(&mut h, "FOO", &[ask_pid]);
        assert_eq!(ask_reply["status"], "permission_required", "{ask_reply}");
        assert_eq!(monitor.list_pending().len(), 1);

        secrets::system_store().delete(&account).unwrap();
    }

    #[test]
    fn child_worker_inherits_parent_posture_restrictions_and_subcontext_reuse_keeps_grants_distinct() {
        let mut h = HostHarness::new();
        let ws = h.workspace_root();
        let pane_pid = |h: &HostHarness, pane_id: u64| {
            h.app
                .windows
                .iter()
                .find_map(|w| w.panes.get(&pane_id))
                .and_then(|p| p.as_terminal())
                .expect("terminal pane")
                .backend
                .child_pid()
        };
        let cfg = secrets::init_workspace(&ws, &crate::config::workspace_channel_dir()).unwrap();
        let account = secrets::keychain_workspace_name(&cfg.id, "BAR");
        secrets::system_store().set(&account, "bar-secret").unwrap();

        write_agent_role(
            &ws,
            "parent-restricted",
            "[permissions]\ndefault_posture = \"ask\"\ndeny = [\"secret.get\"]",
        );
        write_agent_role(
            &ws,
            "child-permissive",
            "[permissions]\ndefault_posture = \"allow\"\nallow = [\"secret.get\"]",
        );
        write_agent_role(
            &ws,
            "peer-ask",
            "[permissions]\ndefault_posture = \"ask\"",
        );

        let (parent_pane, _) = h
            .app
            .spawn_agent_worker_pane(
                "parent-restricted",
                "pi",
                "sleep 30",
                "Parent",
                None,
                true,
                None,
                Some(ws.clone()),
                None,
                None,
                false,
                None,
            )
            .unwrap();
        // Child's definition allows secret.get, but parent denies it: intersection at spawn must deny.
        let (child_pane, child_ctx) = h
            .app
            .spawn_agent_worker_pane(
                "child-permissive",
                "pi",
                "sleep 30",
                "Child",
                Some("shared-squad"),
                true,
                Some(parent_pane),
                Some(ws.clone()),
                None,
                None,
                false,
                None,
            )
            .unwrap();
        let child_pid = pane_pid(&h, child_pane);
        let child_reply = secret_get_request(&mut h, "BAR", &[child_pid]);
        assert_eq!(child_reply["status"], "denied", "{child_reply}");

        // Two independent workers joining the same named sub-context keep distinct postures and grants.
        let (peer_a, ctx_a) = h
            .app
            .spawn_agent_worker_pane(
                "peer-ask",
                "pi",
                "sleep 30",
                "Peer A",
                Some("shared-squad"),
                true,
                None,
                Some(ws.clone()),
                None,
                None,
                false,
                None,
            )
            .unwrap();
        let (peer_b, ctx_b) = h
            .app
            .spawn_agent_worker_pane(
                "peer-ask",
                "pi",
                "sleep 30",
                "Peer B",
                Some("shared-squad"),
                true,
                None,
                Some(ws.clone()),
                None,
                None,
                false,
                None,
            )
            .unwrap();
        assert_eq!(ctx_a, child_ctx);
        assert_eq!(ctx_a, ctx_b);

        let pid_a = pane_pid(&h, peer_a);
        let pid_b = pane_pid(&h, peer_b);
        let req_a = secret_get_request(&mut h, "BAR", &[pid_a]);
        assert_eq!(req_a["status"], "permission_required", "{req_a}");
        let pending_id = req_a["pending_request_id"].as_str().unwrap();
        let monitor =
            crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir());
        monitor
            .approve_pending(pending_id, crate::broker::gate::ApprovalChoice::Always)
            .unwrap();

        let ok_a = secret_get_request(&mut h, "BAR", &[pid_a]);
        assert_eq!(ok_a["status"], "value", "{ok_a}");

        // Peer B in the same sub-context must NOT inherit Peer A's grant.
        let req_b = secret_get_request(&mut h, "BAR", &[pid_b]);
        assert_eq!(req_b["status"], "permission_required", "{req_b}");

        secrets::system_store().delete(&account).unwrap();
    }
}
