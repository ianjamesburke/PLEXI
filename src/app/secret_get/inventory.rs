//! Names-only host inventory for command-socket callers.

use super::*;
use crate::broker::principal::RecordOwner;
use crate::broker::{ActorScope, TargetType};
use crate::workspace::secrets::manager::{self, InventoryScope, SkippedSecrets};
use serde_json::{json, Value};

#[derive(Debug)]
pub(in crate::app) struct Inventory {
    cwd: PathBuf,
    root: PathBuf,
    pane_root: Option<PathBuf>,
    global: bool,
    context_id: u64,
    skipped: Vec<SkippedSecrets>,
}

impl PlexiApp {
    pub(in crate::app) fn serve_secret_list(
        &self,
        context: &str,
        origin: &RequestOrigin,
        response_file: &str,
    ) {
        let path = crate::platform::path::canonical_or_self(Path::new(context));
        let mut candidates = self.router.iter().filter(|ctx| {
            context.parse::<u64>().ok() == Some(ctx.context_id)
                || ctx.name.displayed() == context
                || path == crate::platform::path::canonical_or_self(&ctx.root)
        });
        let target = candidates.next().filter(|_| candidates.next().is_none());
        let Some(ctx) = target else {
            return inventory_reply(response_file, Err(format!(
                "Context '{context}' is missing or ambiguous. Use its id or root from `plexi context list`."
            )));
        };
        let root = crate::platform::path::canonical_or_self(&ctx.root);
        self.serve_inventory(
            Inventory {
                cwd: root.clone(),
                pane_root: Some(root.clone()),
                root,
                global: false,
                context_id: ctx.context_id,
                skipped: Vec::new(),
            },
            origin,
            response_file,
        );
    }

    pub(in crate::app) fn serve_cwd_secret_list(
        &self,
        cwd: &Path,
        global: bool,
        origin: &RequestOrigin,
        response_file: &str,
    ) {
        if !cwd.is_absolute() {
            return inventory_reply(
                response_file,
                Err("An absolute working directory is required.".into()),
            );
        }
        let cwd = crate::platform::path::canonical_or_self(cwd);
        let mut inventory = Inventory {
            root: if global {
                crate::config::config_dir()
            } else {
                cwd.clone()
            },
            cwd,
            pane_root: None,
            global,
            context_id: 0,
            skipped: Vec::new(),
        };
        if let Some(pane) = origin.socket_caller().and_then(SocketCaller::pane) {
            let Some(root) = self.pane_context_authority_root(pane.visible_pane) else {
                return inventory_reply(
                    response_file,
                    Err("This pane has no resolvable context root.".into()),
                );
            };
            if !global && !self.scope_may_access(&root, &inventory.cwd) {
                inventory.skipped.push(SkippedSecrets::OutsidePaneContext {
                    scope: "workspace".into(),
                    path: inventory.cwd.clone(),
                    pane_context_root: root.clone(),
                    pane_context_name: self
                        .router
                        .iter()
                        .find(|ctx| ctx.context_id == pane.context_id)
                        .map(|ctx| ctx.name.displayed().to_owned()),
                });
                inventory.global = true;
            }
            if !inventory.global {
                inventory.root.clone_from(&root);
            }
            inventory.pane_root = Some(root);
            inventory.context_id = pane.context_id;
        }
        self.serve_inventory(inventory, origin, response_file);
    }

    fn serve_inventory(&self, inventory: Inventory, origin: &RequestOrigin, response_file: &str) {
        if inventory.cwd.to_str().is_none() || inventory.root.to_str().is_none() {
            return inventory_reply(
                response_file,
                Err("The inventory scope must be a UTF-8 path.".into()),
            );
        }
        log::info!(
            "secret_list: caller={} scope={}",
            origin.class(),
            inventory.root.display()
        );
        match origin.socket_caller() {
            Some(SocketCaller::Pane(pane)) => {
                if let Err(error) = self.start_pane_inventory(inventory, pane, response_file) {
                    inventory_reply(response_file, Err(error));
                }
            }
            Some(SocketCaller::External(_) | SocketCaller::Unidentified(_)) => {
                self.run_secret_request(
                    "secret.list",
                    "",
                    SecretAsk::Inventory(Box::new(inventory)),
                    origin,
                    response_file,
                    |request, monitor| request.release_inventory(&monitor),
                );
            }
            None => inventory_reply(
                response_file,
                Err("Secret inventory requires a socket caller.".into()),
            ),
        }
    }

    fn start_pane_inventory(
        &self,
        inventory: Inventory,
        pane: &crate::broker::caller::PaneCaller,
        response_file: &str,
    ) -> Result<(), String> {
        let principal = pane.principal();
        let monitor = PermissionMonitor::for_profile(&crate::config::config_dir())?;
        let posture = self.pane_ceiling(pane.terminal_pane)?;
        let call = format!("secret-list-{}", uuid::Uuid::new_v4());
        let input = json!({"path": inventory.root}).to_string();
        let admission = monitor.admit_for_use(
            inventory.admit_request(&call, &input, &principal, "plexi-cli"),
            posture.as_ref(),
        );
        match admission {
            UseDecision::Allowed { .. } => {}
            UseDecision::Required { pending_request_id } => return Err(format!(
                "Missing secret.list grant. Approve request {pending_request_id} in Needs you, then rerun this command."
            )),
            UseDecision::Denied => return Err(
                "Missing secret.list grant. Review the denial in Permissions and approve a new request in Needs you.".into()
            ),
        }
        let pane_scope = AdmittedScope::of(&pane.authority);
        let response_file = response_file.to_owned();
        spawn_secret_worker("secret-list", move || {
            release_inventory_reply(
                &monitor,
                &call,
                &principal,
                admission,
                &response_file,
                || inventory.resolve_scope(),
                || pane_scope.current().map(|_| ()),
            );
        })
        .map_err(|error| format!("Could not start the secret list worker: {error}"))
    }

    pub(in crate::app) fn serve_openrouter_secret_status(
        &self,
        origin: &RequestOrigin,
        response_file: &str,
    ) {
        if origin.socket_caller().is_none() {
            stream_reply(
                Path::new(response_file),
                json!({"error": "A socket caller is required."}),
            );
            return;
        }
        let caller = origin.class();
        let reply_path = PathBuf::from(response_file);
        if let Err(error) = spawn_secret_worker("openrouter-secret-status", move || {
            #[cfg(any(target_os = "macos", target_os = "linux", windows))]
            let result = manager::global_secret_stored("OPENROUTER_API_KEY");
            #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
            let result: Result<bool, String> =
                Err("Secret storage is unavailable on this platform.".into());
            let body = match result {
                Ok(configured) => {
                    log::info!("openrouter_secret_status: caller={caller} configured={configured}");
                    json!({"configured": configured})
                }
                Err(error) => {
                    log::warn!("openrouter_secret_status: account scan failed: {error}");
                    json!({"error": error})
                }
            };
            stream_reply(&reply_path, body);
        }) {
            log::error!("openrouter_secret_status: worker failed: {error}");
            stream_reply(
                Path::new(response_file),
                json!({"error": "Could not start the OpenRouter status worker."}),
            );
        }
    }
}

fn release_inventory_reply(
    monitor: &PermissionMonitor,
    call: &str,
    principal: &Principal,
    admission: UseDecision,
    token: &str,
    resolve_scope: impl FnOnce() -> Result<InventoryScope, String>,
    check_authority: impl Fn() -> Result<(), String>,
) {
    let UseDecision::Allowed {
        approved,
        grant_id,
        fingerprint,
        resource_id,
    } = admission
    else {
        stream_reply(Path::new(token), json!({"status":"denied"}));
        return;
    };
    let connected = || crate::rpc::ReplyRegistry::connected(token) == Some(true);
    let result = (|| {
        let owner = RecordOwner::requester(principal);
        monitor.note_use(&owner, call, &grant_id, &fingerprint, &resource_id, "")?;
        {
            let _commit = monitor.commit(&approved)?;
            check_authority()?;
            if !connected() {
                return Err("The inventory requester disconnected.".into());
            }
        }
        let result = resolve_scope().and_then(|scope| read_inventory(scope, principal));
        #[cfg(test)]
        worker_test::after_inventory_scan();
        monitor.note_outcome(
            None,
            &owner,
            call,
            &grant_id,
            &fingerprint,
            &resource_id,
            "",
            if result.is_ok() { "ok" } else { "failed" },
            "",
            "",
        )?;
        result
    })();
    if !connected() {
        crate::rpc::ReplyRegistry::remove(token);
        log::info!("secret_list: requester disconnected before reply");
        return;
    }
    let staged = result.and_then(|body| {
        // Revocation can land while the store scan is blocked. Hold the
        // policy guard through handover of the reply, then write outside it.
        let _commit = monitor.commit(&approved)?;
        check_authority()?;
        Ok(crate::rpc::stage_stream_only_json_response(token, body))
    });
    match staged {
        Ok(reply) => {
            if reply.is_some_and(crate::rpc::StagedReply::send) {
                log::info!("secret_list: released {resource_id}");
            } else {
                log::warn!("secret_list: reply failed for {resource_id}");
            }
        }
        Err(error) => inventory_reply(token, Err(error)),
    }
}

impl Inventory {
    fn outside_input(&self, caller: &str, scope: &InventoryScope) -> String {
        json!({
            "path": self.root,
            "name": "secret names and usage metadata",
            "scope": scope.disclosure(),
            "caller": caller,
        })
        .to_string()
    }

    fn admit_request<'a>(
        &'a self,
        call: &'a str,
        input: &'a str,
        principal: &'a Principal,
        trust_origin: &'a str,
    ) -> AdmitRequest<'a> {
        AdmitRequest {
            call_id: call,
            tool: "secret.list",
            input_json: input,
            principal,
            actor_scope: ActorScope::User,
            trust_origin,
            workspace_root: &self.root,
            context_id: self.context_id,
            package_id: "",
            instance_id: 0,
            target_type: TargetType::Secret,
        }
    }

    fn resolve_scope(&self) -> Result<InventoryScope, String> {
        InventoryScope::resolve(
            &self.cwd,
            self.global,
            self.pane_root.as_deref(),
            self.skipped.clone(),
        )
    }
}

fn read_inventory(scope: InventoryScope, principal: &Principal) -> Result<Value, String> {
    let inventory = manager::inventory_for_caller(scope, principal)?;
    let has_folders = inventory
        .entries
        .iter()
        .any(|entry| matches!(entry.target, manager::SecretTarget::Folder(_)));
    let mut body = serde_json::to_value(inventory).map_err(|error| error.to_string())?;
    if has_folders {
        body["folder_backend"] = crate::workspace::secrets::backend_label().into();
    }
    Ok(body)
}

impl SecretRequest {
    fn release_inventory(&self, monitor: &PermissionMonitor) {
        let SecretTarget::Inventory(inventory) = &self.target else {
            return;
        };
        let Some(caller) = self.subject.outside() else {
            return;
        };
        let token = self.reply_path.to_string_lossy();
        let scope = match inventory.resolve_scope() {
            Ok(scope) => scope,
            Err(error) => return inventory_reply(&token, Err(error)),
        };
        let principal = self.subject.principal();
        let call = format!("secret-list-{}", uuid::Uuid::new_v4());
        let input = inventory.outside_input(caller, &scope);
        let req = inventory.admit_request(
            &call,
            &input,
            &principal,
            crate::broker::SECRET_SOCKET_ORIGIN,
        );
        let Some(admission) = self.admit(monitor, req) else {
            return;
        };
        release_inventory_reply(
            monitor,
            &call,
            &principal,
            admission,
            &token,
            || Ok(scope),
            || {
                if self.peer.is_none_or(|peer| peer.is_live()) {
                    Ok(())
                } else {
                    Err("The inventory requester exited.".into())
                }
            },
        );
    }
}

fn inventory_reply(response_file: &str, result: Result<Value, String>) {
    let body = result.unwrap_or_else(|error| {
        log::info!("secret_list: refused: {error}");
        json!({"error":error,"error_code":"permission_denied"})
    });
    #[cfg(test)]
    if crate::rpc::ReplyRegistry::connected(response_file).is_none() {
        crate::rpc::write_json_response(response_file, body);
        return;
    }
    stream_reply(Path::new(response_file), body);
}
