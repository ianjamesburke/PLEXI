//! Host-side admission and release of command-socket secrets.

use std::path::{Path, PathBuf};

use super::PlexiApp;
use crate::broker::caller::{RequestOrigin, SocketCaller};
use crate::broker::gate::PermissionMonitor;
use crate::broker::permit::{mint_pane, Coverage, Permit, SecretRelease};
use crate::broker::principal::Principal;
use crate::host::plugin_timeline::AdmittedScope;
use crate::workspace::secrets::{ReadResult, SecretActor, SecretScope};
use std::sync::Arc;

#[path = "secret_mutation.rs"]
mod mutation;

impl PlexiApp {
    pub(crate) fn serve_secret_list(
        &mut self,
        context: &str,
        origin: &crate::broker::caller::RequestOrigin,
        response_file: &str,
    ) {
        let result = self.context_secret_inventory(context, origin);
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
        origin: &crate::broker::caller::RequestOrigin,
    ) -> Result<crate::workspace::secrets::manager::SecretInventory, String> {
        use crate::broker::gate::{Admission, AdmitRequest, PermissionMonitor};
        use crate::broker::{ActorScope, TargetType};
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
        // An outside process is admitted as itself, per exact context.
        if matches!(
            origin,
            crate::broker::caller::RequestOrigin::Socket(
                crate::broker::caller::SocketCaller::External(_)
            )
        ) {
            self.admit_outside_use(
                origin,
                "secret.list",
                &serde_json::json!({"path": root}),
                &root,
                ctx.context_id,
            )?;
            return crate::workspace::secrets::manager::inventory_for_context(&root);
        }
        // Every pane is admitted as its own principal; a pane the person
        // opened has no inventory shortcut (stint 1285).
        let Some(pane) = origin
            .socket_caller()
            .and_then(crate::broker::caller::SocketCaller::pane)
            .map(|pane| pane.terminal_pane)
        else {
            return Err(format!(
                "permission_denied: a {} caller holds no secret inventory; run --context from a Plexi pane.",
                origin.class()
            ));
        };
        log::info!(
            "secret_list: explicit context={} pane={pane}",
            ctx.context_id
        );
        let monitor = PermissionMonitor::for_profile(&crate::config::config_dir())
            .map_err(|error| error.to_string())?;
        let actor = format!("pane:{pane}");
        let call = format!("secret-list-{}", uuid::Uuid::new_v4());
        let input = serde_json::json!({"path": root}).to_string();
        let posture = self.pane_ceiling(pane)?;
        match monitor.admit_with_posture(AdmitRequest {
            call_id: &call, tool: "secret.list", input_json: &input,
            principal: &crate::broker::principal::Principal::agent(&actor), actor_scope: ActorScope::User,
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
        origin: &crate::broker::caller::RequestOrigin,
        response_file: &str,
    ) {
        self.run_secret_request(
            "secret.get",
            name,
            global,
            workspace_root,
            origin,
            response_file,
            |request, monitor| {
                request.read_and_release(&monitor, crate::workspace::secrets::system_store());
            },
        );
    }

    fn run_secret_request(
        &self,
        tool: &'static str,
        name: &str,
        global: bool,
        workspace_root: Option<&Path>,
        origin: &RequestOrigin,
        response_file: &str,
        work: impl FnOnce(SecretRequest, Arc<PermissionMonitor>) + Send + 'static,
    ) {
        let reply_path = PathBuf::from(response_file);
        let profile = crate::config::config_dir();
        if reply_path.parent() != Some(profile.join("rpc").as_path()) {
            log::warn!("{tool}: refused reply outside the profile rpc directory");
            return;
        }
        log::info!(
            "{tool}: caller={} name={name} global={global} workspace={workspace_root:?}",
            origin.class()
        );
        let monitor = match PermissionMonitor::for_profile(&profile) {
            Ok(monitor) => monitor,
            Err(error) => {
                log::error!("{tool}: cannot open host monitor: {error}");
                write_reply(&reply_path, serde_json::json!({"status":"denied"}));
                return;
            }
        };
        let (subject, posture) = match origin.socket_caller() {
            Some(SocketCaller::Pane(pane)) => {
                let id = pane.terminal_pane;
                let scope = AdmittedScope::of(&pane.authority);
                if let Some(root) = workspace_root.filter(|_| !global) {
                    let root = crate::platform::path::canonical_or_self(root);
                    if self.pane_context_authority_root(id).as_ref() != Some(&root)
                        || !self.pane_may_access_context_root(id, &root)
                    {
                        monitor.note_denial(
                            &format!("pane:{id}"),
                            tool,
                            &format!("{name}@{}", root.display()),
                            "",
                            "context_scope_violation",
                        );
                        write_reply(
                            &reply_path,
                            serde_json::json!({"status":"denied","error_code":"permission_denied"}),
                        );
                        return;
                    }
                }
                match self.pane_ceiling(id) {
                    Ok(posture) => (Subject::Pane(scope), posture),
                    Err(error) => {
                        write_reply(
                            &reply_path,
                            serde_json::json!({"status":"error","error":error}),
                        );
                        return;
                    }
                }
            }
            Some(SocketCaller::External(_)) => (Subject::Outside("a terminal outside Plexi"), None),
            Some(SocketCaller::Unidentified(_)) => {
                (Subject::Outside("a detached caller (unidentified)"), None)
            }
            None => {
                write_reply(&reply_path, serde_json::json!({"status":"denied"}));
                return;
            }
        };
        let scope = match scope_for(global, workspace_root) {
            Ok(scope) => scope,
            Err(error) => {
                log::warn!("{tool}: workspace config unreadable: {error}");
                write_reply(
                    &reply_path,
                    serde_json::json!({"status": "error", "error": error}),
                );
                return;
            }
        };
        if tool != "secret.get" && !global && matches!(scope, SecretScope::Global) {
            write_reply(
                &reply_path,
                serde_json::json!({"status":"error","error":"No workspace found. Run plexi workspace init or use --global."}),
            );
            return;
        }
        let request = SecretRequest {
            subject,
            name: name.into(),
            scope,
            profile,
            posture,
            reply_path: reply_path.clone(),
        };
        let repaint = self.ctx.clone();
        let run = move || {
            match request.subject.current() {
                Ok(()) => work(request, monitor),
                Err(error) => {
                    log::info!("{tool}: withheld before admission: {error}");
                    write_reply(&request.reply_path, serde_json::json!({"status":"denied"}));
                }
            }
            repaint.request_repaint();
        };
        #[cfg(not(test))]
        if let Err(error) = std::thread::Builder::new().name(tool.into()).spawn(run) {
            log::error!("{tool}: worker failed: {error}");
            write_reply(
                &reply_path,
                serde_json::json!({"status":"error","error":"Could not start the secret worker."}),
            );
        }
        #[cfg(test)]
        run();
    }
}

enum Subject {
    Pane(AdmittedScope),
    Outside(&'static str),
}

impl Subject {
    fn principal(&self) -> Principal {
        match self {
            Self::Pane(scope) => Principal::agent(format!("pane:{}", scope.authority.pane_id())),
            Self::Outside(_) => Principal::External,
        }
    }

    fn current(&self) -> Result<(), String> {
        match self {
            Self::Pane(scope) => scope.current().map(|_| ()),
            Self::Outside(_) => Ok(()),
        }
    }

    fn outside(&self) -> Option<&'static str> {
        match self {
            Self::Pane(_) => None,
            Self::Outside(label) => Some(label),
        }
    }
}

/// Host-classified request carried into the secret worker.
struct SecretRequest {
    subject: Subject,
    name: String,
    scope: SecretScope,
    profile: PathBuf,
    posture: Option<crate::broker::PermissionPosture>,
    reply_path: PathBuf,
}

impl SecretRequest {
    /// Read under the broker gate and reply. A value leaves only through
    /// [`release_secret`], under the permit minted from the broker's approval
    /// of this read.
    fn read_and_release(
        &self,
        monitor: &std::sync::Arc<crate::broker::gate::PermissionMonitor>,
        store: &dyn crate::workspace::secrets::NonDestructiveStore,
    ) {
        let principal = self.subject.principal();
        let actor = SecretActor {
            kind: principal.actor_type(),
            id: principal.actor_id().into(),
        };
        let result = crate::workspace::secrets::read_pane_secret_with_posture(
            monitor,
            store,
            &actor,
            &self.name,
            &self.scope,
            &self.profile,
            self.posture.as_ref(),
            self.subject.outside(),
        );
        let body = match result.map(|read| (read.result, read.approved)) {
            Ok((ReadResult::Value(value), Some(approved))) => {
                let target = SecretRelease::target(&self.name, &self.scope);
                let permit = match &self.subject {
                    Subject::Pane(scope) => {
                        let coverage =
                            Coverage::new(monitor.clone(), vec![(target.clone(), approved)]);
                        mint_pane::<SecretRelease>(scope.clone(), target.clone(), coverage)
                    }
                    Subject::Outside(class) => crate::broker::permit::mint_external_secret(
                        class,
                        target.clone(),
                        monitor.clone(),
                        approved,
                    ),
                };
                match permit {
                    Ok(permit) => return release_secret(permit, &target, &self.reply_path, &value),
                    Err(error) => {
                        log::info!("secret_get: {} withheld after read: {error}", actor.id);
                        serde_json::json!({"status": "denied"})
                    }
                }
            }
            Ok((ReadResult::Value(_), None)) | Ok((ReadResult::Denied, _)) => {
                serde_json::json!({"status": "denied"})
            }
            Ok((ReadResult::PermissionRequired { pending_id }, _)) => {
                serde_json::json!({"status": "permission_required", "pending_request_id": pending_id})
            }
            Ok((ReadResult::NotFound, _)) => serde_json::json!({"status": "not_found"}),
            Err(error) => serde_json::json!({"status": "error", "error": error}),
        };
        write_reply(&self.reply_path, body);
    }
}

/// Commit the approved read before staging a value on the waiting stream.
/// Pane releases also check the admitted incarnation and scope revision.
/// The blocking write runs after the commit guard has been dropped.
fn release_secret(permit: Permit<SecretRelease>, target: &str, reply_path: &Path, value: &str) {
    let token = reply_path.to_string_lossy();
    let staged = match permit.commit(target, target) {
        Ok(_commit) => crate::rpc::stage_stream_only_json_response(
            &token,
            serde_json::json!({"status": "value", "value": value}),
        ),
        Err(error) => {
            log::info!("secret_get: release withheld: {error}");
            write_reply(reply_path, serde_json::json!({"status": "denied"}));
            return;
        }
    };
    log::info!("secret_get: released {target}");
    if !staged.is_some_and(crate::rpc::StagedReply::send) {
        log::error!(
            "secret_get: value reply for {} failed",
            reply_path.display()
        );
    }
}

/// The workspace whose `workspace.toml` sits at `root`, else global only, the
/// same order `plexi secret get` uses outside a pane.
fn scope_for(global: bool, workspace_root: Option<&Path>) -> Result<SecretScope, String> {
    let Some(root) = workspace_root.filter(|_| !global) else {
        return Ok(SecretScope::Global);
    };
    Ok(
        match crate::workspace::secrets::WorkspaceConfig::load(root)? {
            Some(cfg) => SecretScope::Workspace {
                id: cfg.id,
                root: crate::platform::path::canonical_or_self(root),
            },
            None => SecretScope::Global,
        },
    )
}

/// A reply that holds no secret value.
fn write_reply(path: &Path, body: serde_json::Value) {
    if !crate::rpc::write_json_response(&path.to_string_lossy(), body) {
        log::error!("secret_get: reply write for {} failed", path.display());
    }
}

#[cfg(test)]
mod list_tests {
    use crate::testing::HostHarness;
    use crate::workspace::secrets;

    fn request(h: &mut HostHarness, context: &str, peer: &[u32]) -> serde_json::Value {
        let response_file = crate::rpc::response_file("secret-list-test", "json");
        let request = crate::protocol::HostRequest::PaneSecretList {
            context: context.into(),
            response_file: response_file.clone(),
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
        secrets::system_store()
            .set(&account, "not-for-output")
            .unwrap();
        // A pane the person opened is brokered like any other (stint 1285).
        for target in [&context, "inventory-target", root.to_str().unwrap()] {
            let refused = request(&mut h, target, &[pid]);
            assert!(refused.get("entries").is_none(), "{refused}");
            let error = refused["error"].as_str().unwrap();
            assert!(
                error.contains("secret.list") && error.contains("Needs you"),
                "{error}"
            );
        }
        // The listener overwrites forged ancestry before dispatch, including while hidden.
        assert_eq!(
            request(&mut h, &context, &[])["error_code"],
            "permission_denied"
        );
        let restricted = crate::broker::pane_authority::PaneAuthority::for_test(
            pane,
            crate::broker::pane_authority::Ceiling::Baseline,
            h.app.windows[0].context_id,
            h.workspace_root(),
        );
        h.app.windows[0]
            .panes
            .get_mut(&pane)
            .unwrap()
            .as_terminal_mut()
            .unwrap()
            .replace_grant_for_test(restricted);
        let denied = request(&mut h, &context, &[pid]);
        let error = denied["error"].as_str().unwrap();
        assert!(error.contains("secret.list") && error.contains("Needs you"));
        let monitor =
            crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir())
                .unwrap();
        let pending = monitor
            .list_entries()
            .into_iter()
            .find(|row| row.tool == "secret.list")
            .unwrap();
        monitor
            .approve_pending(&pending.id, crate::broker::gate::ApprovalChoice::Once)
            .unwrap();
        let allowed = request(&mut h, &context, &[pid]);
        assert!(allowed["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["name"] == name));
        assert_eq!(
            request(&mut h, &context, &[pid])["error_code"],
            "permission_denied"
        );
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
        let request = crate::protocol::HostRequest::SecretGet {
            name: name.into(),
            global: false,
            workspace_root: Some(h.workspace_root()),
            response_file: response_file.clone(),
        };
        let line = serde_json::to_string(&request).unwrap();
        let (mut server_stream, client_stream) = std::os::unix::net::UnixStream::pair().unwrap();
        crate::rpc::register_client_stream(response_file.clone(), client_stream);
        crate::app::handle_socket_line(&line, &h.ipc_tx, Some(peer), &mut server_stream);
        drop(server_stream);
        h.hidden_frame();
        let body = crate::rpc::poll_string(&response_file, Some(std::time::Duration::from_secs(2)))
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
            crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir())
                .unwrap();
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
        let live = h
            .app
            .terminal_incarnation(pane)
            .unwrap()
            .authority()
            .unwrap()
            .clone();
        // A standing approval, so only the missing stream withholds the value.
        let actor = format!("pane:{pane}");
        monitor
            .store()
            .record(crate::broker::GrantRecord::event_stream_allow(
                crate::broker::ActorType::Agent,
                &actor,
                crate::broker::ActorScope::User,
                "unbacked::release",
                &ws,
                crate::broker::GrantDuration::Always,
                crate::broker::GrantSource::Workspace,
                None,
            ));
        let question =
            crate::broker::PermissionRequest::exact(&crate::broker::ExactBinding::event_stream(
                crate::broker::ActorType::Agent,
                &actor,
                crate::broker::ActorScope::User,
                "unbacked::release",
                &ws,
                None,
            ));
        let (approved, _) = monitor
            .admit_questions(vec![question], None)
            .unwrap()
            .remove(0);
        let permit = super::mint_pane::<super::SecretRelease>(
            crate::host::plugin_timeline::AdmittedScope::of(&live),
            "unbacked".into(),
            super::Coverage::new(monitor.clone(), vec![("unbacked".into(), approved)]),
        )
        .unwrap();
        super::release_secret(
            permit,
            "unbacked",
            std::path::Path::new(&unbacked),
            secret_val,
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
            crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir())
                .unwrap();
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
        let send_req = crate::protocol::HostRequest::SendToPane {
            pane_id: target_pane,
            text: "echo hi".into(),
            submit: false,
            paste: false,
            response_file: Some(send_resp.clone()),
            operation_id: None,
            generated_operation_id: false,
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

        let pending_id = ask_reply["pending_request_id"].as_str().unwrap();
        monitor
            .approve_pending(pending_id, crate::broker::gate::ApprovalChoice::Once)
            .unwrap();
        let val_reply = secret_get_request(&mut h, "FOO", &[ask_pid]);
        assert_eq!(val_reply["status"], "value", "{val_reply}");
        assert_eq!(val_reply["value"], "top-secret");

        let audit_json = serde_json::to_string(&monitor.audit_records()).unwrap();
        assert!(
            !audit_json.contains("top-secret"),
            "audit records must never retain real secret values: {audit_json}"
        );
        assert!(
            audit_json.contains("FOO"),
            "audit records must retain resource identifier FOO: {audit_json}"
        );

        secrets::system_store().delete(&account).unwrap();
    }

    #[test]
    fn worker_missing_snapshot_fallback_and_boundary_qualification() {
        let mut h = HostHarness::new();
        let target_pane = h.add_focused_terminal();
        let ws = h.workspace_root();
        let cfg = secrets::init_workspace(&ws, &crate::config::workspace_channel_dir()).unwrap();
        let account = secrets::keychain_workspace_name(&cfg.id, "QUAL");
        secrets::system_store()
            .set(&account, "qual-secret")
            .unwrap();

        // Spawn a worker without a settings.toml or valid role definition.
        let (worker_pane, _) = h
            .app
            .spawn_agent_worker_pane(
                "unconfigured-worker",
                "pi",
                "sleep 30",
                "Fallback check",
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
        let worker_pid = h.terminal_backend(worker_pane).child_pid();

        // Missing posture snapshot: pane_permission_posture returns None.
        assert!(
            h.app.pane_ceiling_or_none(worker_pane).is_none(),
            "unconfigured worker must have no posture snapshot"
        );

        // 1. Missing-snapshot fallback on secret.get enters broker and requires approval (Decision::Ask).
        let secret_reply = secret_get_request(&mut h, "QUAL", &[worker_pid]);
        assert_eq!(
            secret_reply["status"], "permission_required",
            "missing-snapshot fallback on secret.get must enter broker gate and ask, never bypass: {secret_reply}"
        );

        let monitor =
            crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir())
                .unwrap();
        assert_eq!(
            monitor.list_pending().len(),
            1,
            "missing-snapshot secret.get must file an approval request"
        );

        // 2. Missing-snapshot fallback on SendToPane must also enter the broker
        //    (Decision::Ask) instead of bypassing the gate: a worker with no
        //    snapshotted posture is still agent-owned, not human-owned.
        let send_resp = crate::rpc::response_file("pane-send-fallback", "json");
        let send_req = crate::protocol::HostRequest::SendToPane {
            pane_id: target_pane,
            text: "fallback message".into(),
            submit: false,
            paste: false,
            response_file: Some(send_resp.clone()),
            operation_id: None,
            generated_operation_id: false,
        };
        let send_line = serde_json::to_string(&send_req).unwrap();
        crate::app::handle_socket_line(&send_line, &h.ipc_tx, Some(&[worker_pid]), &mut Vec::new());
        h.hidden_frame();
        assert!(
            !std::path::Path::new(&send_resp).exists(),
            "missing-snapshot SendToPane must park on an approval instead of proceeding un-gated"
        );
        let send_pending = monitor
            .list_pending()
            .into_iter()
            .find(|row| row.tool == "pane.send")
            .expect("missing-snapshot pane.send must file an approval request");
        assert_eq!(send_pending.actor_id, format!("pane:{worker_pane}"));

        // Approving releases the parked send and writes the deferred reply.
        monitor
            .approve_pending(
                &send_pending.pending_request_id,
                crate::broker::gate::ApprovalChoice::Once,
            )
            .unwrap();
        h.hidden_frame();
        let send_reply: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&send_resp).unwrap()).unwrap();
        assert_eq!(
            send_reply["ok"], true,
            "approved missing-snapshot send must deliver: {send_reply}"
        );

        secrets::system_store().delete(&account).unwrap();
    }

    /// A pane the person opened has no posture snapshot either, and that buys
    /// it nothing: its send asks the broker like any pane's (stint 1285).
    #[test]
    fn a_desktop_pane_with_no_posture_is_brokered_like_any_pane() {
        let mut h = HostHarness::new();
        let human_pane = h.add_focused_terminal();
        let human_pid = h.terminal_backend(human_pane).child_pid();
        assert!(
            h.app
                .terminal_incarnation(human_pane)
                .is_some_and(|t| t.is_desktop_granted()),
            "fixture must be a human-owned terminal pane"
        );
        assert!(
            h.app.pane_ceiling_or_none(human_pane).is_none(),
            "human-owned pane must have no posture snapshot"
        );

        let send_resp = crate::rpc::response_file("pane-send-human", "json");
        let send_req = crate::protocol::HostRequest::SendToPane {
            pane_id: human_pane,
            text: "human message".into(),
            submit: false,
            paste: false,
            response_file: Some(send_resp.clone()),
            operation_id: None,
            generated_operation_id: false,
        };
        let send_line = serde_json::to_string(&send_req).unwrap();
        crate::app::handle_socket_line(&send_line, &h.ipc_tx, Some(&[human_pid]), &mut Vec::new());
        h.hidden_frame();
        assert!(
            !std::path::Path::new(&send_resp).exists(),
            "the send waits for a decision instead of being written"
        );
        let monitor =
            crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir())
                .unwrap();
        assert!(
            monitor
                .list_pending()
                .iter()
                .any(|row| row.tool == "pane.send"),
            "the send files a pane.send approval request"
        );
    }

    #[test]
    fn child_worker_inherits_parent_posture_restrictions_and_subcontext_reuse_keeps_grants_distinct(
    ) {
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
        write_agent_role(&ws, "peer-ask", "[permissions]\ndefault_posture = \"ask\"");

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
            crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir())
                .unwrap();
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

/// Stint 1716: the value is released only under a permit whose pane
/// incarnation and revision, and the grant that admitted the read, still
/// hold at the write.
#[cfg(test)]
mod release_tests {
    use super::*;
    use crate::broker::gate::{ApprovalChoice, PermissionMonitor};
    use crate::broker::pane_authority::{Ceiling, LivePaneAuthority, PaneAuthority};
    use crate::workspace::secrets::{
        InMemoryKeychain, NonDestructiveStore, SecretError, SecretStore,
    };
    use std::sync::Arc;
    use zeroize::Zeroizing;

    /// What happens while the store read is in progress.
    #[derive(Clone, Copy, Debug)]
    enum DuringRead {
        Nothing,
        ClosePane,
        RescopePane,
        RevokeGrant,
        /// A second reader with the same approval arrives mid-read.
        SecondReader,
    }

    /// A store that acts on the asking pane or its grant mid-read.
    struct ActsDuringRead {
        inner: InMemoryKeychain,
        pane: Arc<LivePaneAuthority>,
        monitor: Arc<PermissionMonitor>,
        during: DuringRead,
        profile: tempfile::TempDir,
        gets: std::sync::atomic::AtomicUsize,
        second: std::sync::Mutex<Option<serde_json::Value>>,
    }

    impl NonDestructiveStore for ActsDuringRead {
        fn get(&self, account: &str) -> Option<Zeroizing<String>> {
            let first = self.gets.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0;
            match self.during {
                DuringRead::Nothing => {}
                DuringRead::SecondReader if first => {
                    *self.second.lock().unwrap() = Some(read(self));
                }
                DuringRead::SecondReader => {}
                DuringRead::ClosePane => self.pane.revoke(),
                DuringRead::RescopePane => {
                    self.pane.publish_scope(PathBuf::from("/tmp/elsewhere"));
                }
                DuringRead::RevokeGrant => {
                    let ids: Vec<String> = self
                        .monitor
                        .store()
                        .records()
                        .iter()
                        .filter(|record| record.actor_id == "pane:7")
                        .map(|record| record.grant_id.clone())
                        .collect();
                    assert!(!ids.is_empty(), "the read was admitted by a grant");
                    for id in ids {
                        assert!(self.monitor.revoke_grant_id(&id).unwrap());
                    }
                }
            }
            self.inner.get(account)
        }
        fn add_new(&self, account: &str, value: &str) -> Result<(), SecretError> {
            self.inner.add_new(account, value)
        }
        fn delete_if_value(&self, account: &str, expected: &str) -> Result<(), SecretError> {
            self.inner.delete_if_value(account, expected)
        }
        fn list_with_prefix(&self, prefix: &str) -> Vec<String> {
            self.inner.list_with_prefix(prefix)
        }
        fn scan_accounts(&self) -> Result<Vec<String>, SecretError> {
            self.inner.scan_accounts()
        }
    }

    fn read(store: &ActsDuringRead) -> serde_json::Value {
        let scope = SecretScope::Global;
        let reply = crate::rpc::response_file("secret-release-1716", "json");
        // As the socket listener wires a reply: the host writes one end, the
        // waiting client reads the other.
        let (server, client) = std::os::unix::net::UnixStream::pair().unwrap();
        crate::rpc::register_client_stream(reply.clone(), client);
        crate::rpc::ReplyRegistry::register(reply.clone(), server.try_clone().unwrap());
        SecretRequest {
            subject: Subject::Pane(AdmittedScope::of(&store.pane)),
            name: "TOKEN_1716".into(),
            scope,
            profile: store.profile.path().to_path_buf(),
            posture: None,
            reply_path: PathBuf::from(&reply),
        }
        .read_and_release(&store.monitor, store);
        drop(server);
        let body =
            crate::rpc::poll_string(&reply, Some(std::time::Duration::from_secs(2))).unwrap();
        serde_json::from_str(&body).unwrap()
    }

    /// Approve the read with `choice`, then read once while `during` happens.
    fn read_while(
        during: DuringRead,
        choice: ApprovalChoice,
    ) -> (serde_json::Value, ActsDuringRead) {
        let pane = LivePaneAuthority::admitted(PaneAuthority::for_test(
            7,
            Ceiling::Baseline,
            1,
            PathBuf::from("/tmp/ws"),
        ));
        pane.mark_live();
        let store = ActsDuringRead {
            inner: InMemoryKeychain::new(),
            pane,
            monitor: PermissionMonitor::ephemeral(),
            during,
            profile: tempfile::tempdir().unwrap(),
            gets: Default::default(),
            second: Default::default(),
        };
        store
            .inner
            .set(
                &crate::workspace::secrets::keychain_user_name("TOKEN_1716"),
                "canary-1716",
            )
            .unwrap();
        // Admission comes before the store is touched: this read only asks.
        let ask = read(&store);
        assert_eq!(ask["status"], "permission_required", "{ask}");
        store
            .monitor
            .approve_pending(ask["pending_request_id"].as_str().unwrap(), choice)
            .unwrap();
        (read(&store), store)
    }

    #[test]
    fn the_value_is_released_only_while_pane_and_grant_still_hold() {
        for choice in [ApprovalChoice::Once, ApprovalChoice::Always] {
            let (released, _) = read_while(DuringRead::Nothing, choice);
            assert_eq!(released["status"], "value", "{choice:?}: {released}");
            assert_eq!(released["value"], "canary-1716");
            for during in [
                DuringRead::ClosePane,
                DuringRead::RescopePane,
                DuringRead::RevokeGrant,
            ] {
                let (reply, _) = read_while(during, choice);
                assert_eq!(reply["status"], "denied", "{during:?} {choice:?}: {reply}");
                assert!(!reply.to_string().contains("canary-1716"));
            }
        }
    }

    /// Stint 1716 (design-3306 once): one approval admits one read. A
    /// second reader arriving mid-read is asked again and never reaches the
    /// store; once the first read has run, the next read asks again too.
    #[test]
    fn a_once_approval_admits_exactly_one_read() {
        let (first, store) = read_while(DuringRead::SecondReader, ApprovalChoice::Once);
        assert_eq!(first["status"], "value", "{first}");
        let second = store.second.lock().unwrap().take().unwrap();
        assert_eq!(second["status"], "permission_required", "{second}");
        assert_eq!(store.gets.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(
            read(&store)["status"],
            "permission_required",
            "the claim was spent"
        );
    }
}

#[cfg(all(test, unix))]
mod cli_tests {
    use crate::testing::HostHarness;
    use crate::workspace::secrets;
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixListener;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    // Each invocation has its own CLI environment and process-local mock store.
    #[test]
    fn secret_cli_child() {
        let Ok(op) = std::env::var("SECRET_TEST_OP") else {
            return;
        };
        let _profile = crate::config::set_test_profile_dir(
            std::env::var_os("SECRET_TEST_PROFILE").unwrap().into(),
        );
        if let Some(socket) = std::env::var_os("SECRET_TEST_SOCKET") {
            crate::cli::set_command_socket_override(socket.into());
        }
        let name = "SOCKET_1280";
        secrets::take_test_store_calls();
        let code = match op.as_str() {
            "get" => crate::cli::workspace::workspace_secret_get(name, true),
            "set" => crate::cli::workspace::workspace_secret_set(name, true, true, None),
            "delete" => crate::cli::workspace::workspace_secret_delete(name, true),
            _ => unreachable!(),
        };
        assert_eq!(
            secrets::take_test_store_calls(),
            0,
            "the CLI opened its store"
        );
        std::process::exit(code);
    }

    fn cli(op: &str, profile: &std::path::Path) -> Command {
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        cmd.args([
            "app::secret_get::cli_tests::secret_cli_child",
            "--exact",
            "--nocapture",
            "--test-threads=2",
        ])
        .env_clear()
        .env("SECRET_TEST_PROFILE", profile)
        .env("HOME", profile)
        .env("SECRET_TEST_OP", op)
        .env("SOCKET_1280", "fixture-value")
        .current_dir(profile)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
        cmd
    }

    #[test]
    fn workspace_secret_cli_fails_closed_without_a_host() {
        let profile = tempfile::tempdir().unwrap();
        for op in ["get", "set", "delete"] {
            let output = cli(op, profile.path()).output().unwrap();
            assert_eq!(output.status.code(), Some(1));
            assert_eq!(
                String::from_utf8_lossy(&output.stdout).trim(),
                "running 1 test"
            );
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                stderr.contains("Plexi host did not answer") && stderr.contains("start Plexi"),
                "{stderr}"
            );
        }
    }

    #[test]
    fn outside_cli_secret_get_over_host_socket() {
        let mut h = HostHarness::new();
        let profile = crate::config::config_dir();
        let socket_dir = tempfile::tempdir_in("/tmp").unwrap();
        let socket = socket_dir.path().join("host.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let store = secrets::system_store();
        let account = secrets::keychain_user_name("SOCKET_1280");
        store.set(&account, "fixture-value").unwrap();
        for expected in [2, 0, 2] {
            let mut child = cli("get", &profile)
                .env("SECRET_TEST_SOCKET", &socket)
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(child.try_wait().unwrap().is_none() && Instant::now() < deadline);
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) => panic!("accept: {e}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let peer = crate::broker::caller::PeerWitness::capture(
                super::super::resolve_socket_peer_pid(&stream).unwrap(),
            );
            assert!(matches!(
                h.app.classify_socket_peer(Some(&peer)),
                crate::broker::caller::SocketCaller::External(_)
            ));
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            crate::app::handle_socket_line_in(
                &profile,
                line.trim(),
                &h.ipc_tx,
                Some(&peer),
                &mut stream,
            );
            drop(stream);
            h.hidden_frame();
            let output = child.wait_with_output().unwrap();
            assert_eq!(
                output.status.code(),
                Some(expected),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let stdout = String::from_utf8(output.stdout).unwrap();
            if expected == 2 {
                assert!(
                    stdout.contains("permission_required") && !stdout.contains("fixture-value")
                );
                let id = stdout
                    .lines()
                    .find_map(|line| line.strip_prefix("pending_request_id="))
                    .unwrap();
                crate::broker::gate::PermissionMonitor::for_profile(&profile)
                    .unwrap()
                    .approve_pending(id, crate::broker::gate::ApprovalChoice::Once)
                    .unwrap();
            } else {
                assert!(stdout.lines().any(|line| line == "fixture-value"));
            }
        }
        store.delete(&account).unwrap();
    }
}
