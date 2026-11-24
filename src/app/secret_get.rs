//! Host-side admission and release of command-socket secrets.

use std::path::{Path, PathBuf};

use super::PlexiApp;
use crate::broker::caller::{PeerHop, RequestOrigin, SocketCaller};
use crate::broker::gate::{
    Admission, AdmitRequest, ApprovalChoice, PermissionMonitor, UseDecision,
};
use crate::broker::permit::{mint_pane, Coverage, Permit, SecretRelease};
use crate::broker::principal::Principal;
use crate::host::plugin_timeline::AdmittedScope;
use crate::workspace::secrets::{
    NonDestructiveStore, ReadResult, SecretActor, SecretError, SecretScope,
};
use std::sync::Arc;
use zeroize::Zeroizing;

mod bundle;
#[path = "secret_mutation.rs"]
mod mutation;
use bundle::Bundle;

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
        self.serve_secret_read(
            name,
            SecretAsk::workspace(global, workspace_root),
            origin,
            response_file,
        );
    }

    pub(super) fn serve_secret_read(
        &self,
        name: &str,
        ask: SecretAsk,
        origin: &RequestOrigin,
        response_file: &str,
    ) {
        let tool = if ask.is_folder() {
            crate::workspace::secrets::SECRET_READ_TOOL
        } else {
            "secret.get"
        };
        self.run_secret_request(
            tool,
            name,
            ask,
            origin,
            response_file,
            |request, monitor| {
                if let SecretTarget::One { scope, .. } = &request.target {
                    request.read_and_release(&monitor, scope.store());
                }
            },
        );
    }

    fn run_secret_request(
        &self,
        tool: &'static str,
        name: &str,
        mut ask: SecretAsk,
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
            "{tool}: caller={} name={name} scope={ask:?}",
            origin.class()
        );
        if let Err(error) = ask.validate(name) {
            stream_reply(
                &reply_path,
                serde_json::json!({"status":"error","error":error}),
            );
            return;
        }
        let monitor = match PermissionMonitor::for_profile(&profile) {
            Ok(monitor) => monitor,
            Err(error) => {
                log::error!("{tool}: cannot open host monitor: {error}");
                stream_reply(&reply_path, serde_json::json!({"status":"denied"}));
                return;
            }
        };
        let (subject, posture, discovery_root) = match origin.socket_caller() {
            Some(SocketCaller::Pane(pane)) => {
                let id = pane.terminal_pane;
                let scope = AdmittedScope::of(&pane.authority);
                let authority = self.pane_context_authority_root(id);
                let target = match &ask {
                    SecretAsk::Folder(path)
                    | SecretAsk::Workspace(Some(path))
                    | SecretAsk::Bundle { cwd: path, .. } => Some(path),
                    SecretAsk::DiscoverFolder(_) => authority.as_ref(),
                    SecretAsk::Global | SecretAsk::Workspace(None) => None,
                };
                if target.is_some_and(|root| {
                    authority.as_ref() != Some(root) || !self.pane_may_access_context_root(id, root)
                }) || matches!(&ask, SecretAsk::Bundle { command: Some((_, Some(root))), .. } if Some(root) != authority.as_ref())
                    || (matches!(ask, SecretAsk::DiscoverFolder(_)) && authority.is_none())
                {
                    let prefix = if ask.is_folder() && tool != crate::workspace::secrets::SECRET_READ_TOOL {
                        "folder:"
                    } else {
                        ""
                    };
                    let path = target.map(|p| p.display().to_string()).unwrap_or_default();
                    monitor.note_denial(
                        &format!("pane:{id}"),
                        tool,
                        &format!("{name}@{prefix}{path}"),
                        "",
                        "context_scope_violation",
                    );
                    stream_reply(
                        &reply_path,
                        serde_json::json!({"status":"denied","error_code":"permission_denied"}),
                    );
                    return;
                }
                match self.pane_ceiling(id) {
                    Ok(posture) => (Subject::Pane(scope), posture, authority),
                    Err(error) => {
                        stream_reply(
                            &reply_path,
                            serde_json::json!({"status":"error","error":error}),
                        );
                        return;
                    }
                }
            }
            Some(SocketCaller::External(_)) => {
                (Subject::Outside("a terminal outside Plexi"), None, None)
            }
            Some(SocketCaller::Unidentified(_)) => (
                Subject::Outside("a detached caller (unidentified)"),
                None,
                None,
            ),
            None => {
                stream_reply(&reply_path, serde_json::json!({"status":"denied"}));
                return;
            }
        };
        let name = name.to_owned();
        let peer = origin
            .socket_caller()
            .and_then(SocketCaller::peer)
            .and_then(|peer| peer.hops.first())
            .copied();
        let repaint = self.ctx.clone();
        let reply = reply_path.clone();
        let run = move || {
            if let Err(error) = subject.current() {
                log::info!("{tool}: withheld before admission: {error}");
                stream_reply(
                    &reply,
                    serde_json::json!({"status":"denied","reason":"revoked"}),
                );
            } else {
                match ask.target_for(tool, name, discovery_root.as_deref()) {
                    Ok(target) => work(
                        SecretRequest {
                            subject,
                            peer,
                            repaint: repaint.clone(),
                            target,
                            profile,
                            posture,
                            reply_path: reply,
                        },
                        monitor,
                    ),
                    Err(error) => {
                        log::info!("{tool}: scope unavailable: {error}");
                        stream_reply(&reply, serde_json::json!({"status":"error","error":error}));
                    }
                }
            }
            repaint.request_repaint();
        };
        #[cfg(test)]
        let spawn = worker_test::spawn(tool, crate::config::config_dir(), run);
        #[cfg(not(test))]
        let spawn = std::thread::Builder::new().name(tool.into()).spawn(run);
        if let Err(error) = spawn {
            log::error!("{tool}: worker failed: {error}");
            stream_reply(
                &reply_path,
                serde_json::json!({"status":"error","error":"Could not start the secret worker."}),
            );
        }
    }
}

/// Scope requested on the wire. Folder discovery runs on the secret worker.
#[derive(Debug)]
pub(super) enum SecretAsk {
    Global,
    Workspace(Option<PathBuf>),
    Folder(PathBuf),
    DiscoverFolder(PathBuf),
    Bundle {
        cwd: PathBuf,
        command: Option<(String, Option<PathBuf>)>,
    },
}

impl SecretAsk {
    pub(super) fn workspace(global: bool, root: Option<&Path>) -> Self {
        if global {
            Self::Global
        } else {
            Self::Workspace(root.map(crate::platform::path::canonical_or_self))
        }
    }

    fn is_folder(&self) -> bool {
        matches!(self, Self::Folder(_) | Self::DiscoverFolder(_))
    }

    fn validate(&mut self, name: &str) -> Result<(), String> {
        if let Self::Bundle { cwd, command } = self {
            let feature = crate::release::ReleaseFeature::FolderSecrets;
            if command.is_none() && !crate::release::feature_enabled(feature) {
                return Err(crate::release::feature_unavailable_message(feature));
            }
            *cwd = crate::workspace::secrets::canonical_folder(cwd)?;
            if let Some((_, Some(root))) = command {
                *root = crate::workspace::secrets::canonical_folder(root)?;
            }
            if cwd.to_str().is_none() {
                return Err("The canonical working directory must be valid UTF-8.".into());
            }
            return Ok(());
        }
        if self.is_folder() {
            let feature = crate::release::ReleaseFeature::FolderSecrets;
            if !crate::release::feature_enabled(feature) {
                return Err(crate::release::feature_unavailable_message(feature));
            }
            if !crate::workspace::secrets::manager::valid_env_name(name) {
                return Err(format!(
                    "secret name '{name}' is not an environment variable name"
                ));
            }
            if let Self::Folder(path) = self {
                *path = crate::workspace::secrets::canonical_folder(path)?;
                if path.to_str().is_none() {
                    return Err("The canonical folder path must be valid UTF-8.".into());
                }
            }
        }
        Ok(())
    }

    fn target_for(
        self,
        tool: &str,
        name: String,
        discovery_root: Option<&Path>,
    ) -> Result<SecretTarget, String> {
        let scope: Result<SecretScope, String> = match self {
            Self::Bundle { cwd, command } => {
                return Bundle::discover(cwd, command, discovery_root).map(|bundle| SecretTarget::Bundle(Box::new(bundle)));
            }
            Self::Global => Ok(SecretScope::Global),
            Self::Workspace(root) => {
                if let Some(root) = root {
                    if let Some(cfg) = crate::workspace::secrets::WorkspaceConfig::load(&root)? {
                        return Ok(SecretTarget::One {
                            name,
                            scope: SecretScope::Workspace { id: cfg.id, root },
                        });
                    }
                }
                if tool == "secret.get" {
                    Ok(SecretScope::Global)
                } else {
                    Err("No workspace found. Run plexi workspace init or use --global.".into())
                }
            }
            Self::Folder(path) => Ok(SecretScope::Folder { path }),
            Self::DiscoverFolder(cwd) => {
                let cwd = crate::workspace::secrets::canonical_folder(&cwd)?;
                let folders: Vec<_> = crate::workspace::secrets::list_folder_secrets(
                    crate::workspace::secrets::folder_store(),
                )
                .into_iter()
                .filter(|meta| {
                    meta.name == name && discovery_root.is_none_or(|root| root == meta.folder)
                })
                .map(|meta| meta.folder)
                .collect();
                let nearest = folders
                    .iter()
                    .filter(|path| cwd.starts_with(path))
                    .max_by_key(|path| path.components().count());
                let path = match (nearest, folders.as_slice()) {
                    (Some(path), _) | (None, [path]) => path.clone(),
                    (_, []) => return Err(format!("secret '{name}' not found")),
                    _ => {
                        return Err(format!(
                            "secret '{name}' is set on more than one folder; pass --folder"
                        ))
                    }
                };
                if path.to_str().is_none()
                    || crate::workspace::secrets::canonical_folder(&path)? != path
                {
                    return Err("The stored folder no longer has a canonical UTF-8 path.".into());
                }
                Ok(SecretScope::Folder { path })
            }
        };
        Ok(SecretTarget::One {
            name,
            scope: scope?,
        })
    }
}

enum SecretTarget {
    One { name: String, scope: SecretScope },
    Bundle(Box<Bundle>),
}

enum Subject {
    Pane(AdmittedScope),
    Outside(&'static str),
}

impl Subject {
    fn principal(&self) -> Principal {
        match self {
            Self::Pane(scope) => Principal::agent(format!("pane:{}", scope.authority.pane_id())),
            Self::Outside(..) => Principal::External,
        }
    }

    fn current(&self) -> Result<(), String> {
        match self {
            Self::Pane(scope) => scope.current().map(|_| ()),
            Self::Outside(..) => Ok(()),
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
    peer: Option<PeerHop>,
    repaint: egui::Context,
    target: SecretTarget,
    profile: PathBuf,
    posture: Option<crate::broker::PermissionPosture>,
    reply_path: PathBuf,
}

struct LiveStore<'a> {
    inner: &'a dyn NonDestructiveStore,
    token: &'a str,
}

impl LiveStore<'_> {
    fn check_client(&self) -> bool {
        let live = crate::rpc::ReplyRegistry::connected(self.token) == Some(true);
        if !live {
            log::info!("secret_get: client gone before the store read");
        }
        live
    }
}

impl NonDestructiveStore for LiveStore<'_> {
    fn get(&self, account: &str) -> Option<Zeroizing<String>> {
        if !self.check_client() {
            return None;
        }
        self.inner.get(account)
    }

    fn get_interactive(&self, account: &str) -> Result<Option<Zeroizing<String>>, SecretError> {
        self.inner.get_interactive_while(account, &|| self.check_client())
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

impl SecretRequest {
    fn admit(&self, monitor: &PermissionMonitor, req: AdmitRequest<'_>) -> Option<UseDecision> {
        let tool = req.tool;
        let deadline = std::time::Instant::now() + crate::workspace::secrets::APPROVAL_TIMEOUT;
        let posture = self.posture.as_ref();
        // Keep the same host call ID from filing through claim. The click
        // reserves its Once until this worker checks the connection and deadline.
        let pending = match monitor.admit_with_posture(req, posture) {
            Admission::Denied { .. } => return Some(UseDecision::Denied),
            Admission::Proceed { .. } => None,
            Admission::Required { pending_request_id } => {
                log::info!(
                    "{tool}: parked pending={pending_request_id} {}",
                    self.bundle_label()
                );
                self.repaint.request_repaint();
                Some(pending_request_id)
            }
        };
        let token = self.reply_path.to_string_lossy();
        let stopped = |_claim: bool| {
            #[cfg(test)]
            let deadline = worker_test::deadline(deadline, _claim);
            if crate::rpc::ReplyRegistry::connected(&token) != Some(true) {
                Some("client gone")
            } else if self.subject.current().is_err()
                || self.peer.is_some_and(|peer| !peer.is_live())
            {
                Some("revoked")
            } else if std::time::Instant::now() >= deadline {
                Some("expired")
            } else {
                None
            }
        };
        let reason = loop {
            if let Some(reason) = stopped(false) {
                break reason;
            }
            if let Some(id) = &pending {
                match monitor.approval_resolution(id) {
                    Some(
                        ApprovalChoice::Once | ApprovalChoice::Session | ApprovalChoice::Always,
                    ) => (),
                    Some(_) => break "denied",
                    None => {
                        std::thread::sleep(std::time::Duration::from_millis(20));
                        continue;
                    }
                }
            }
            let stop = std::cell::Cell::new(None);
            let current = || {
                stop.set(stopped(true));
                #[cfg(test)]
                if stop.get().is_none() {
                    worker_test::after_claim_check(&token);
                }
                stop.get().is_none()
            };
            let claim = monitor.admit_for_use_if(req, posture, Some(&current));
            if matches!(claim, UseDecision::Allowed { .. }) {
                log::info!(
                    "{tool}: claimed pending={pending:?} {}",
                    self.bundle_label()
                );
                return Some(claim);
            }
            break stop.get().unwrap_or("denied");
        };
        log::info!("{tool}: {reason} pending={pending:?}");
        if let Some(id) = pending {
            if let Err(error) = monitor.withdraw_live_ask(&id) {
                log::error!("{tool}: withdraw {id}: {error}");
            }
        }
        if reason == "client gone" {
            crate::rpc::ReplyRegistry::remove(&token);
        } else {
            stream_reply(
                &self.reply_path,
                serde_json::json!({"status":"denied","reason":reason}),
            );
        }
        None
    }

    fn release_permit(
        &self,
        monitor: &Arc<PermissionMonitor>,
        target: &str,
        approved: crate::broker::gate::Approved,
    ) -> Result<Permit<SecretRelease>, String> {
        match &self.subject {
            Subject::Pane(scope) => {
                let coverage = Coverage::new(monitor.clone(), vec![(target.to_owned(), approved)]);
                mint_pane::<SecretRelease>(scope.clone(), target.to_owned(), coverage)
            }
            Subject::Outside(class) => crate::broker::permit::mint_external_secret(
                class,
                target.to_owned(),
                monitor.clone(),
                approved,
            ),
        }
    }

    /// Read under the broker gate and reply. A value leaves only through
    /// [`release_secret`], under the permit minted from the broker's approval
    /// of this read.
    fn read_and_release(
        &self,
        monitor: &std::sync::Arc<crate::broker::gate::PermissionMonitor>,
        store: &dyn NonDestructiveStore,
    ) {
        let SecretTarget::One { name, scope } = &self.target else {
            return;
        };
        let principal = self.subject.principal();
        let actor = SecretActor {
            kind: principal.actor_type(),
            id: principal.actor_id().into(),
        };
        let Some(result) = crate::workspace::secrets::read_pane_secret_with(
            monitor,
            &LiveStore {
                inner: store,
                token: &self.reply_path.to_string_lossy(),
            },
            &actor,
            crate::workspace::secrets::ReadTarget {
                name,
                scope,
                global_root: &self.profile,
            },
            self.subject.outside(),
            |req| self.admit(monitor, req),
        )
        .transpose() else {
            return;
        };
        let body = match result.map(|read| (read.result, read.approved)) {
            Ok((ReadResult::Value(value), Some(approved))) => {
                let target = SecretRelease::target(name, scope);
                let permit = self.release_permit(monitor, &target, approved);
                match permit {
                    Ok(permit) => {
                        return release_secret(
                            permit,
                            &target,
                            &self.reply_path,
                            "secret.get",
                            || serde_json::json!({"status":"value","value":value.as_str()}),
                        )
                    }
                    Err(error) => {
                        log::info!("secret_get: {} withheld after read: {error}", actor.id);
                        serde_json::json!({"status": "denied"})
                    }
                }
            }
            Ok((ReadResult::NotFound, _)) => serde_json::json!({"status": "not_found"}),
            Ok(_) => serde_json::json!({"status": "denied"}),
            Err(error) => serde_json::json!({"status": "error", "error": error}),
        };
        stream_reply(&self.reply_path, body);
    }
}

/// Commit the approved read before staging a value on the waiting stream.
/// Pane releases also check the admitted incarnation and scope revision.
/// The blocking write runs after the commit guard has been dropped.
fn release_secret<T: serde::Serialize>(
    permit: Permit<SecretRelease>,
    target: &str,
    reply_path: &Path,
    label: &str,
    body: impl FnOnce() -> T,
) {
    let token = reply_path.to_string_lossy();
    let staged = match permit.commit(target, target) {
        Ok(_commit) => crate::rpc::stage_stream_only_json_response(&token, body()),
        Err(error) => {
            log::info!("{label}: release withheld: {error}");
            stream_reply(reply_path, serde_json::json!({"status": "denied"}));
            return;
        }
    };
    if staged.is_some_and(crate::rpc::StagedReply::send) {
        log::info!("{label}: released {target}");
    } else {
        log::error!("{label}: reply on original connection failed for {target}");
    }
}

fn stream_reply(path: &Path, body: serde_json::Value) {
    let token = path.to_string_lossy();
    #[cfg(test)]
    if worker_test::TIMING.with_borrow(|mode| matches!(mode, worker_test::Timing::DropReply))
        && body["status"] == "ok"
    {
        crate::rpc::ReplyRegistry::remove(&token);
    }
    let sent = crate::rpc::stage_stream_only_json_response(&token, body)
        .is_some_and(crate::rpc::StagedReply::send);
    log::info!(
        "secret: reply {} token={token}",
        if sent { "sent" } else { "lost" }
    );
}

#[cfg(test)]
mod worker_test {
    use std::{cell::RefCell, path::PathBuf, thread::JoinHandle};

    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    type Evidence = (usize, Vec<String>);
    #[derive(Default)]
    pub(super) enum Timing {
        #[default]
        Normal,
        Deadline(Arc<Mutex<Instant>>),
        ExpireAtClaim,
        DropClientAfterClaimCheck(Option<crate::platform::ipc::IpcStream>),
        PauseAfterAsk(std::sync::mpsc::Receiver<()>),
        ObserveStoreBeforeClick(std::sync::mpsc::Sender<usize>),
        DropReply,
    }
    thread_local! {
        pub(super) static TIMING: RefCell<Timing> = const { RefCell::new(Timing::Normal) };
        static WORKERS: RefCell<Vec<JoinHandle<Evidence>>> = const { RefCell::new(Vec::new()) };
    }

    pub(super) fn after_claim_check(token: &str) {
        TIMING.with_borrow_mut(|mode| {
            if let Timing::DropClientAfterClaimCheck(client) = mode {
                drop(client.take());
                wait_for_disconnect(token);
                *mode = Timing::Normal;
            }
        });
    }

    pub(super) fn wait_for_disconnect(token: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while crate::rpc::ReplyRegistry::connected(token) != Some(false) {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for the reply registry to observe the client disconnect"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    pub(super) fn deadline(real: Instant, claim: bool) -> Instant {
        TIMING.with_borrow_mut(|mode| match mode {
            Timing::ObserveStoreBeforeClick(tx) if !claim => {
                tx.send(crate::workspace::secrets::take_test_store_calls())
                    .unwrap();
                *mode = Timing::Normal;
                real
            }
            Timing::PauseAfterAsk(resume) if !claim => {
                resume
                    .recv_timeout(std::time::Duration::from_secs(10))
                    .unwrap();
                *mode = Timing::Normal;
                real
            }
            Timing::Deadline(time) => real.min(*time.lock().unwrap()),
            Timing::ExpireAtClaim if claim => Instant::now(),
            _ => real,
        })
    }

    pub(super) fn spawn(
        tool: &str,
        profile: PathBuf,
        run: impl FnOnce() + Send + 'static,
    ) -> std::io::Result<()> {
        let timing = TIMING.take();
        let feature = crate::release::ReleaseFeature::FolderSecrets;
        let features: Vec<_> = crate::release::feature_enabled(feature)
            .then_some(feature)
            .into_iter()
            .collect();
        let worker = std::thread::Builder::new()
            .name(tool.into())
            .spawn(move || {
                let _profile = crate::config::set_test_profile_dir(profile);
                let _experiments = crate::release::set_test_experiments(&features);
                TIMING.replace(timing);
                let logs = crate::testing::test_log::capture_with_debug();
                run();
                (
                    crate::workspace::secrets::take_test_store_calls(),
                    logs.lines(),
                )
            })?;
        WORKERS.with_borrow_mut(|workers| workers.push(worker));
        Ok(())
    }

    #[cfg(unix)]
    pub(super) fn join() -> Evidence {
        WORKERS
            .with_borrow_mut(|workers| workers.pop().unwrap())
            .join()
            .unwrap()
    }
}

#[cfg(test)]
mod list_tests {
    use crate::broker::caller::PeerWitness;
    use crate::broker::gate::{ApprovalChoice, PermissionMonitor};
    use crate::testing::socket_policy_tests::{live_secret, read_client, send};
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

    #[test]
    fn folder_requests_check_feature_and_arguments_before_a_card() {
        let mut h = HostHarness::new();
        let root = h.workspace_root();
        let monitor = PermissionMonitor::for_profile(h.profile_dir()).unwrap();
        let feature = crate::release::ReleaseFeature::FolderSecrets;
        for op in ["read", "set", "rm"] {
            let mut request = serde_json::json!({"type": format!("folder_secret_{op}"), "name":"VALID", "folder":root});
            if op == "set" {
                request["value"] = "fixture".into();
            }
            if op == "read" {
                request["cwd"] = serde_json::json!(root);
            }
            let _off = crate::release::set_test_experiments(&[]);
            let reply = send(&mut h, request.clone(), None);
            assert_eq!(
                reply,
                serde_json::json!({"status":"error","error":crate::release::feature_unavailable_message(feature)})
            );
            let _on = crate::release::set_test_experiments(&[feature]);
            for (field, value, expected) in [
                (
                    "name",
                    serde_json::json!("bad=name"),
                    "environment variable name",
                ),
                ("folder", serde_json::json!(root.join("missing")), "missing"),
            ] {
                let mut bad = request.clone();
                bad[field] = value;
                let reply = send(&mut h, bad, None);
                assert_eq!(reply["status"], "error");
                assert!(
                    reply["error"].as_str().unwrap().contains(expected),
                    "{reply}"
                );
            }
            if op == "set" {
                request["value"] = "".into();
                let reply = send(&mut h, request, None);
                assert_eq!(reply["status"], "error");
                assert!(reply["error"].as_str().unwrap().contains("empty"));
            }
            assert!(monitor.list_needs_you().is_empty());
        }
    }

    #[test]
    fn exact_pane_folder_verbs_wait_for_their_own_decision() {
        let mut h = HostHarness::new();
        let pane = h.add_focused_terminal();
        let peer = PeerWitness::for_live_pids(&[h.shell_pid(pane)]);
        let _on =
            crate::release::set_test_experiments(&[crate::release::ReleaseFeature::FolderSecrets]);
        let root = h.workspace_root().canonicalize().unwrap();
        let monitor = PermissionMonitor::for_profile(h.profile_dir()).unwrap();
        let name = format!("PANE_{}", uuid::Uuid::new_v4().simple());
        let other = tempfile::tempdir().unwrap();
        secrets::set_folder_secret(secrets::folder_store(), &name, other.path(), "outside-root")
            .unwrap();
        for op in ["set", "read", "rm"] {
            let mut request = serde_json::json!({"type": format!("folder_secret_{op}"), "name":name, "folder":root});
            if op == "set" {
                request["value"] = "pane-folder-canary".into();
            }
            if op == "read" {
                request["folder"] = serde_json::Value::Null;
                request["cwd"] = serde_json::json!(other.path());
            }
            for choice in [ApprovalChoice::Deny, ApprovalChoice::Once] {
                let (client, ask) = live_secret(&mut h, request.clone(), &peer);
                assert_eq!(ask.actor_id, format!("pane:{pane}"));
                assert_eq!(monitor.open_needs_you().len(), 1);
                monitor
                    .approve_pending(&ask.pending_request_id, choice)
                    .unwrap();
                let reply = read_client(client, op);
                if choice == ApprovalChoice::Deny {
                    assert_eq!(reply["status"], "denied");
                } else if op == "read" {
                    assert_eq!(reply["value"], "pane-folder-canary");
                } else {
                    assert_eq!(reply["status"], "ok");
                }
                let discovery = usize::from(op == "read");
                assert_eq!(
                    super::worker_test::join().0,
                    discovery + usize::from(choice == ApprovalChoice::Once)
                );
                assert!(monitor.open_needs_you().is_empty());
            }
        }
    }

    #[test]
    fn bundle_pane_scope_and_missing_names() {
        let mut h = HostHarness::new();
        let ws = h.workspace_root();
        write_agent_role(
            &ws,
            "bundle-reader",
            "[permissions]\ndefault_posture = \"ask\"",
        );
        let (pane, _) = h
            .app
            .spawn_agent_worker_pane(
                "bundle-reader",
                "pi",
                "sleep 30",
                "Bundle reader",
                None,
                true,
                None,
                Some(ws),
                None,
                None,
                false,
                None,
            )
            .unwrap();
        let peer = PeerWitness::for_live_pids(&[h.terminal_backend(pane).child_pid()]);
        let _on =
            crate::release::set_test_experiments(&[crate::release::ReleaseFeature::FolderSecrets]);
        let root = h.workspace_root().canonicalize().unwrap();
        let config =
            secrets::init_workspace(&root, &crate::config::workspace_channel_dir()).unwrap();
        let ancestor = format!("ANCESTOR_{}", uuid::Uuid::new_v4().simple());
        secrets::set_folder_secret(
            secrets::folder_store(),
            &ancestor,
            root.parent().unwrap(),
            "ancestor-canary",
        )
        .unwrap();
        secrets::set_folder_secret(secrets::folder_store(), "LOCAL", &root, "local-canary")
            .unwrap();
        secrets::write_terminal_env_inject(&root, "REQUIRED", true).unwrap();
        let commands = crate::cli::ensure_commands_file(Some(&root));
        std::fs::write(
            commands,
            "[commands]\nbuild = { run = 'true', secrets = ['REQUIRED'] }",
        )
        .unwrap();
        let monitor = PermissionMonitor::for_profile(h.profile_dir()).unwrap();
        for kind in ["folder_secret_exec", "run_secrets"] {
            let mut wire = serde_json::json!({"type":kind, "cwd":root});
            if kind == "run_secrets" {
                wire["command"] = "build".into();
                wire["workspace_root"] = serde_json::json!(root);
            }
            let mut wrong = wire.clone();
            wrong["cwd"] = serde_json::json!(root.parent());
            let denied = send(&mut h, wrong, Some(peer.clone()));
            assert_eq!(denied["error_code"], "permission_denied");
            assert!(
                std::fs::read_to_string(h.profile_dir().join("permission-audit.jsonl"))
                    .unwrap()
                    .contains("context_scope_violation")
            );
            let (client, card) = live_secret(&mut h, wire.clone(), &peer);
            assert_eq!(card.actor_id, format!("pane:{pane}"));
            assert!(card.input_summary.contains("REQUIRED"));
            assert!(!card.input_summary.contains(&ancestor));
            assert!(card.input_summary.contains(if kind == "run_secrets" {
                "build"
            } else {
                "LOCAL"
            }));
            assert!(monitor
                .approve_pending(&card.pending_request_id, ApprovalChoice::Session)
                .is_err());
            monitor
                .approve_pending(&card.pending_request_id, ApprovalChoice::Once)
                .unwrap();
            assert_eq!(
                read_client(client, kind),
                serde_json::json!({"status":"missing","names":["REQUIRED"]})
            );
            super::worker_test::join();
            let account = secrets::keychain_workspace_name(&config.id, "REQUIRED");
            secrets::system_store()
                .set(&account, "terminal-canary")
                .unwrap();
            let outside = PeerWitness::for_live_pids(&[std::process::id()]);
            let (client, granted) = live_secret(&mut h, wire.clone(), &outside);
            assert!(granted.input_summary.contains("REQUIRED"));
            let (denied, other) = live_secret(&mut h, wire.clone(), &outside);
            assert_eq!(other.resource_id, granted.resource_id);
            assert_ne!(other.pending_request_id, granted.pending_request_id);
            monitor
                .approve_pending(&other.pending_request_id, ApprovalChoice::Deny)
                .unwrap();
            assert_eq!(read_client(denied, kind)["status"], "denied");
            assert_eq!(
                super::worker_test::join().0,
                usize::from(kind == "folder_secret_exec")
            );
            assert_eq!(monitor.open_needs_you().len(), 1);
            monitor
                .approve_pending(&granted.pending_request_id, ApprovalChoice::Once)
                .unwrap();
            let reply = read_client(client, kind);
            assert_eq!(reply["status"], "values");
            let values = reply["values"].as_array().unwrap();
            assert!(values
                .iter()
                .any(|item| item["name"] == "REQUIRED" && item["value"] == "terminal-canary"));
            if kind == "run_secrets" {
                assert_eq!(reply["run"], "true");
                assert_eq!(values.len(), 1);
            } else {
                assert!(values
                    .iter()
                    .any(|item| item["name"] == ancestor && item["value"] == "ancestor-canary"));
                assert!(values
                    .iter()
                    .any(|item| item["name"] == "LOCAL" && item["value"] == "local-canary"));
            }
            let (_, logs) = super::worker_test::join();
            for canary in ["terminal-canary", "ancestor-canary", "local-canary"] {
                assert!(!logs.join("\n").contains(canary));
                assert!(!granted.input_summary.contains(canary));
                assert!(
                    !std::fs::read_to_string(h.profile_dir().join("permission-audit.jsonl"))
                        .unwrap()
                        .contains(canary)
                );
            }
            secrets::system_store().delete(&account).unwrap();
            let (cancel, next) = live_secret(&mut h, wire, &peer);
            drop(cancel);
            super::worker_test::join();
            assert!(monitor.open_needs_you().is_empty());
            assert!(monitor
                .approve_pending(&next.pending_request_id, ApprovalChoice::Once)
                .is_err());
        }
        secrets::remove_folder_secret(secrets::folder_store(), &ancestor, root.parent().unwrap())
            .unwrap();
        let retired_gate = h.profile_dir().join("folder-secret-gate.sock");
        assert_eq!(
            std::os::unix::net::UnixStream::connect(retired_gate)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::NotFound,
        );
    }

    #[test]
    fn folder_discovery_selects_only_visible_bindings() {
        let base = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let root = base.path().canonicalize().unwrap();
        let nested = root.join("nested");
        std::fs::create_dir(&nested).unwrap();
        let name = format!("DISCOVERY_{}", uuid::Uuid::new_v4().simple());
        let discover = |cwd: &std::path::Path, authority: Option<&std::path::Path>| {
            super::SecretAsk::DiscoverFolder(cwd.into())
                .target_for("secret.read", name.clone(), authority)
                .map(|target| match target {
                    super::SecretTarget::One { scope, .. } => scope.label(),
                    _ => unreachable!(),
                })
        };
        let missing = format!("secret '{name}' not found");
        assert_eq!(discover(&root, None), Err(missing.clone()));
        secrets::set_folder_secret(secrets::folder_store(), &name, &nested, "nested").unwrap();
        let nested_label = format!("folder:{}", nested.display());
        assert_eq!(discover(other.path(), None), Ok(nested_label.clone()));
        assert_eq!(discover(&nested, Some(&root)), Err(missing));
        secrets::set_folder_secret(secrets::folder_store(), &name, &root, "root").unwrap();
        assert_eq!(discover(&nested, None), Ok(nested_label));
        assert_eq!(
            discover(&nested, Some(&root)),
            Ok(format!("folder:{}", root.display()))
        );
        assert_eq!(
            discover(other.path(), None),
            Err(format!(
                "secret '{name}' is set on more than one folder; pass --folder"
            ))
        );
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

    fn pane_get(
        h: &mut HostHarness,
        name: &str,
        pid: u32,
        choice: Option<ApprovalChoice>,
    ) -> serde_json::Value {
        let request = serde_json::json!({"type":"secret_get","name":name,"workspace_root":h.workspace_root()});
        let peer = PeerWitness::for_live_pids(&[pid]);
        if let Some(choice) = choice {
            let (client, ask) = live_secret(h, request, &peer);
            let monitor = PermissionMonitor::for_profile(h.profile_dir()).unwrap();
            monitor
                .approve_pending(&ask.pending_request_id, choice)
                .unwrap();
            read_client(client, name)
        } else {
            send(h, request, Some(peer))
        }
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
        let monitor = PermissionMonitor::for_profile(h.profile_dir()).unwrap();
        let reply = pane_get(&mut h, secret_name, pid, Some(ApprovalChoice::Once));
        assert_eq!(reply["status"], "value", "{reply}");
        assert_eq!(reply["value"], secret_val);

        if let Ok(entries) = std::fs::read_dir(&rpc_dir) {
            for entry in entries.flatten() {
                let fname = entry.file_name();
                let name_str = fname.to_string_lossy();
                assert!(
                    !name_str.starts_with("secret-get-")
                        && !name_str.starts_with("host-secret-stream"),
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
            "secret.get",
            || serde_json::json!({"status":"value","value":secret_val}),
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
        let reply = pane_get(&mut h, "FOO", denied_pid, None);
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
        let val_reply = pane_get(&mut h, "FOO", ask_pid, Some(ApprovalChoice::Once));
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
        let secret_reply = pane_get(&mut h, "QUAL", worker_pid, Some(ApprovalChoice::Deny));
        assert_eq!(secret_reply["reason"], "denied");
        let monitor = PermissionMonitor::for_profile(h.profile_dir()).unwrap();

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

    #[test]
    fn a_denied_pane_set_never_opens_the_store() {
        let mut h = HostHarness::new();
        let pane = h.add_focused_terminal();
        let peer = PeerWitness::for_live_pids(&[h.shell_pid(pane)]);
        let request = serde_json::json!({"type":"secret_set","global":true,"name":"DENIED_1280","value":"discard-me"});
        let (client, ask) = live_secret(&mut h, request, &peer);
        assert_eq!(ask.actor_id, format!("pane:{pane}"));
        let monitor = PermissionMonitor::for_profile(h.profile_dir()).unwrap();
        monitor
            .approve_pending(&ask.pending_request_id, ApprovalChoice::Deny)
            .unwrap();
        assert_eq!(
            read_client(client, "pane set"),
            serde_json::json!({"status":"denied","reason":"denied"})
        );
        assert_eq!(super::worker_test::join().0, 0);
        assert!(monitor.open_needs_you().is_empty());
    }

    fn pane_set(
        h: &mut HostHarness,
        peer: &PeerWitness,
        name: &str,
        value: &str,
    ) -> (std::os::unix::net::UnixStream, String) {
        let frame =
            serde_json::json!({"type":"secret_set","global":true,"name":name,"value":value});
        let (client, ask) = live_secret(h, frame, peer);
        assert!(ask.actor_id.starts_with("pane:"));
        (client, ask.pending_request_id)
    }

    #[test]
    fn concurrent_pane_sets_write_only_the_clicked_value() {
        let mut h = HostHarness::new();
        let pane = h.add_focused_terminal();
        let peer = PeerWitness::for_live_pids(&[h.shell_pid(pane)]);
        let monitor = PermissionMonitor::for_profile(h.profile_dir()).unwrap();
        let name = "PANE_CONCURRENT_1280";
        let account = secrets::keychain_user_name(name);
        let stored = || secrets::system_store().get(&account).unwrap();
        let (a, first) = pane_set(&mut h, &peer, name, "first-pane-canary");
        let (b, second) = pane_set(&mut h, &peer, name, "second-pane-canary");
        assert_ne!(first, second);
        assert_eq!(monitor.open_needs_you().len(), 2);
        monitor
            .approve_pending(&second, ApprovalChoice::Once)
            .unwrap();
        assert_eq!(read_client(b, name)["status"], "ok");
        assert_eq!(super::worker_test::join().0, 1);
        assert_eq!(stored().as_str(), "second-pane-canary");
        let open = monitor.open_needs_you();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].id, first);
        monitor
            .approve_pending(&first, ApprovalChoice::Deny)
            .unwrap();
        assert_eq!(read_client(a, name)["reason"], "denied");
        assert_eq!(super::worker_test::join().0, 0);
        assert_eq!(stored().as_str(), "second-pane-canary");
        assert!(monitor.open_needs_you().is_empty());
        assert!(monitor
            .store()
            .records()
            .iter()
            .filter(|r| r.target_id == "secret.set")
            .all(|r| r.consumed));
    }

    #[test]
    fn cancelling_a_pane_client_leaves_the_other_clients_card_live() {
        let mut h = HostHarness::new();
        let pane = h.add_focused_terminal();
        let peer = PeerWitness::for_live_pids(&[h.shell_pid(pane)]);
        let monitor = PermissionMonitor::for_profile(h.profile_dir()).unwrap();
        let name = "PANE_CANCEL_1280";
        let (survivor, card) = pane_set(&mut h, &peer, name, "surviving-canary");
        // Wait for both asks even if they were merged onto one card.
        let frame = serde_json::json!({"type":"secret_set","global":true,"name":name,
            "value":"cancelled-canary","response_file":crate::rpc::response_file("gone", "json")});
        let (mut server, client) = std::os::unix::net::UnixStream::pair().unwrap();
        crate::app::handle_socket_line_in(
            h.profile_dir(),
            &frame.to_string(),
            &h.ipc_tx,
            Some(&peer),
            &mut server,
        );
        drop(server);
        h.hidden_frame();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while monitor
            .audit_records()
            .iter()
            .filter(|row| row.kind == "ask")
            .count()
            < 2
        {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        drop(client);
        assert_eq!(super::worker_test::join().0, 0);
        let open = monitor.open_needs_you();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].id, card);
        monitor
            .approve_pending(&card, ApprovalChoice::Once)
            .unwrap();
        assert_eq!(read_client(survivor, name)["status"], "ok");
        assert_eq!(super::worker_test::join().0, 1);
        assert_eq!(
            secrets::system_store()
                .get(&secrets::keychain_user_name(name))
                .unwrap()
                .as_str(),
            "surviving-canary"
        );
    }

    #[test]
    fn later_pane_set_needs_its_own_click_while_the_first_click_is_unclaimed() {
        let mut h = HostHarness::new();
        let pane = h.add_focused_terminal();
        let peer = PeerWitness::for_live_pids(&[h.shell_pid(pane)]);
        let monitor = PermissionMonitor::for_profile(h.profile_dir()).unwrap();
        let name = "PANE_LATER_1280";
        let account = secrets::keychain_user_name(name);
        let (resume, paused) = std::sync::mpsc::channel();
        super::worker_test::TIMING.replace(super::worker_test::Timing::PauseAfterAsk(paused));
        let (a, first) = pane_set(&mut h, &peer, name, "original-canary");
        monitor
            .approve_pending(&first, ApprovalChoice::Once)
            .unwrap();
        let (b, second) = pane_set(&mut h, &peer, name, "later-canary");
        assert_ne!(first, second);
        assert!(secrets::system_store().get(&account).is_none());
        resume.send(()).unwrap();
        assert_eq!(read_client(a, name)["status"], "ok");
        assert_eq!(
            secrets::system_store().get(&account).unwrap().as_str(),
            "original-canary"
        );
        monitor
            .approve_pending(&second, ApprovalChoice::Once)
            .unwrap();
        assert_eq!(read_client(b, name)["status"], "ok");
        assert_eq!(
            secrets::system_store().get(&account).unwrap().as_str(),
            "later-canary"
        );
        assert_eq!(super::worker_test::join().0, 1);
        assert_eq!(super::worker_test::join().0, 1);
    }

    #[test]
    fn closing_a_parked_pane_withdraws_its_card() {
        let mut h = HostHarness::new();
        let pane = h.add_focused_terminal();
        let peer = PeerWitness::for_live_pids(&[std::process::id(), h.shell_pid(pane)]);
        let get = serde_json::json!({"type":"secret_get","name":"CLOSED_1280","global":true});
        let (client, ask) = live_secret(&mut h, get, &peer);
        assert_eq!(ask.actor_id, format!("pane:{pane}"));
        h.app.close_pane_by_id(pane);
        let monitor = PermissionMonitor::for_profile(h.profile_dir()).unwrap();
        // Closing the pane retires its asks and revokes its scope; the parked
        // worker may observe either first. Both end with no effect.
        let reason = read_client(client, "closed pane")["reason"].clone();
        assert!(reason == "revoked" || reason == "denied", "{reason}");
        assert_eq!(super::worker_test::join().0, 0);
        assert!(monitor.open_needs_you().is_empty());
        assert!(monitor
            .approve_pending(&ask.pending_request_id, ApprovalChoice::Once)
            .is_err());
        assert!(monitor
            .store()
            .records()
            .iter()
            .all(|r| r.target_id != "secret.get"));
    }

    #[test]
    fn a_pane_session_grant_answers_the_next_call_without_a_card() {
        let mut h = HostHarness::new();
        let pane = h.add_focused_terminal();
        let pid = h.shell_pid(pane);
        let monitor = PermissionMonitor::for_profile(h.profile_dir()).unwrap();
        assert_eq!(
            pane_get(
                &mut h,
                "SESSION_PANE_1280",
                pid,
                Some(ApprovalChoice::Session)
            )["status"],
            "not_found"
        );
        let asks = || {
            monitor
                .audit_records()
                .iter()
                .filter(|row| row.kind == "ask")
                .count()
        };
        assert_eq!(asks(), 1);
        for _ in 0..3 {
            assert_eq!(
                pane_get(&mut h, "SESSION_PANE_1280", pid, None)["status"],
                "not_found"
            );
            assert_eq!(asks(), 1);
            assert!(monitor.open_needs_you().is_empty());
        }
    }

    #[test]
    fn a_parked_pane_checks_the_connecting_process_even_with_an_open_stream() {
        let mut h = HostHarness::new();
        let pane = h.add_focused_terminal();
        let mut child = std::process::Command::new("sleep")
            .arg("5")
            .spawn()
            .unwrap();
        let peer = PeerWitness::for_live_pids(&[child.id(), h.shell_pid(pane)]);
        let get = serde_json::json!({"type":"secret_get","name":"DEAD_PEER_1280","global":true});
        let (client, ask) = live_secret(&mut h, get, &peer);
        assert_eq!(ask.actor_id, format!("pane:{pane}"));
        child.kill().unwrap();
        child.wait().unwrap();
        assert_eq!(read_client(client, "dead peer")["reason"], "revoked");
        assert_eq!(super::worker_test::join().0, 0);
        let monitor = PermissionMonitor::for_profile(h.profile_dir()).unwrap();
        assert!(monitor.open_needs_you().is_empty());
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
        let child_reply = pane_get(&mut h, "BAR", child_pid, None);
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
        let ok_a = pane_get(&mut h, "BAR", pid_a, Some(ApprovalChoice::Always));
        assert_eq!(ok_a["status"], "value", "{ok_a}");
        assert_eq!(pane_get(&mut h, "BAR", pid_a, None)["status"], "value");

        // Peer B in the same sub-context still has to ask.
        let req_b = pane_get(&mut h, "BAR", pid_b, Some(ApprovalChoice::Deny));
        assert_eq!(req_b["reason"], "denied", "{req_b}");

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
    }

    /// A store that acts on the asking pane or its grant mid-read.
    struct ActsDuringRead {
        inner: InMemoryKeychain,
        pane: Arc<LivePaneAuthority>,
        monitor: Arc<PermissionMonitor>,
        during: DuringRead,
        profile: tempfile::TempDir,
        gets: std::sync::atomic::AtomicUsize,
    }

    impl NonDestructiveStore for ActsDuringRead {
        fn get(&self, account: &str) -> Option<Zeroizing<String>> {
            self.gets.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            match self.during {
                DuringRead::Nothing => {}
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

    fn request(store: &ActsDuringRead, reply: &str) -> SecretRequest {
        SecretRequest {
            subject: Subject::Pane(AdmittedScope::of(&store.pane)),
            peer: None,
            repaint: Default::default(),
            target: super::SecretTarget::One {
                name: "TOKEN_1716".into(),
                scope: SecretScope::Global,
            },
            profile: store.profile.path().to_path_buf(),
            posture: None,
            reply_path: PathBuf::from(reply),
        }
    }

    fn read(store: &ActsDuringRead) -> serde_json::Value {
        let reply = crate::rpc::response_file_in(store.profile.path(), "secret-release", "json");
        let (server, client) = std::os::unix::net::UnixStream::pair().unwrap();
        crate::rpc::ReplyRegistry::register(reply.clone(), server);
        request(store, &reply).read_and_release(&store.monitor, store);
        crate::testing::socket_policy_tests::read_client(client, "pane read")
    }

    fn fixture(during: DuringRead) -> ActsDuringRead {
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
        };
        store
            .inner
            .set(
                &crate::workspace::secrets::keychain_user_name("TOKEN_1716"),
                "canary-1716",
            )
            .unwrap();
        store
    }

    fn decide(store: &ActsDuringRead, asks: usize, choice: ApprovalChoice) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while store
            .monitor
            .audit_records()
            .iter()
            .filter(|row| row.kind == "ask")
            .count()
            < asks
        {
            assert!(std::time::Instant::now() < deadline, "read did not ask");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let pending = store.monitor.list_pending();
        assert_eq!(pending.len(), 1);
        store
            .monitor
            .approve_pending(&pending[0].pending_request_id, choice)
            .unwrap();
    }

    fn read_while(during: DuringRead, choice: ApprovalChoice) -> serde_json::Value {
        let store = fixture(during);
        std::thread::scope(|s| {
            let worker = s.spawn(|| read(&store));
            decide(&store, 1, choice);
            worker.join().unwrap()
        })
    }

    #[test]
    fn a_pane_request_without_its_connection_withdraws() {
        let store = fixture(DuringRead::Nothing);
        let token = crate::rpc::response_file_in(store.profile.path(), "gone", "json");
        request(&store, &token).read_and_release(&store.monitor, &store);
        assert!(store.monitor.open_needs_you().is_empty());
        assert_eq!(store.gets.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(!Path::new(&token).exists());
    }

    #[test]
    fn a_disconnected_pane_cannot_claim_an_already_approved_read() {
        let store = fixture(DuringRead::Nothing);
        let token = crate::rpc::response_file_in(store.profile.path(), "gone", "json");
        let (server, client) = std::os::unix::net::UnixStream::pair().unwrap();
        crate::rpc::ReplyRegistry::register(token.clone(), server);
        let (resume, paused) = std::sync::mpsc::channel();
        std::thread::scope(|s| {
            let worker = s.spawn(|| {
                worker_test::TIMING.replace(worker_test::Timing::PauseAfterAsk(paused));
                request(&store, &token).read_and_release(&store.monitor, &store);
            });
            decide(&store, 1, ApprovalChoice::Once);
            drop(client);
            // A concurrently forked child keeps its inherited descriptor open until exec.
            worker_test::wait_for_disconnect(&token);
            resume.send(()).unwrap();
            worker.join().unwrap();
        });
        assert_eq!(store.gets.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(store.monitor.store().records().is_empty());
    }

    #[test]
    fn a_disconnect_after_the_claim_check_prevents_the_store_read() {
        let store = fixture(DuringRead::Nothing);
        let token = crate::rpc::response_file_in(store.profile.path(), "gone-after-check", "json");
        let (server, client) = std::os::unix::net::UnixStream::pair().unwrap();
        crate::rpc::ReplyRegistry::register(token.clone(), server);
        std::thread::scope(|s| {
            let worker = s.spawn(|| {
                worker_test::TIMING
                    .replace(worker_test::Timing::DropClientAfterClaimCheck(Some(client)));
                request(&store, &token).read_and_release(&store.monitor, &store);
            });
            decide(&store, 1, ApprovalChoice::Once);
            worker.join().unwrap();
        });
        assert_eq!(store.gets.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(store
            .monitor
            .audit_records()
            .iter()
            .any(|row| row.kind == "outcome" && row.decision == "backend_error"));
        assert_eq!(crate::rpc::ReplyRegistry::connected(&token), None);
        assert!(!Path::new(&token).exists());
    }

    #[test]
    fn the_value_is_released_only_while_pane_and_grant_still_hold() {
        for choice in [ApprovalChoice::Once, ApprovalChoice::Always] {
            let released = read_while(DuringRead::Nothing, choice);
            assert_eq!(released["status"], "value", "{choice:?}: {released}");
            assert_eq!(released["value"], "canary-1716");
            for during in [
                DuringRead::ClosePane,
                DuringRead::RescopePane,
                DuringRead::RevokeGrant,
            ] {
                let reply = read_while(during, choice);
                assert_eq!(reply["status"], "denied", "{during:?} {choice:?}: {reply}");
                assert!(!reply.to_string().contains("canary-1716"));
            }
        }
    }

    #[test]
    fn a_once_approval_admits_exactly_one_read() {
        let store = fixture(DuringRead::Nothing);
        std::thread::scope(|s| {
            let first = s.spawn(|| read(&store));
            decide(&store, 1, ApprovalChoice::Once);
            assert_eq!(first.join().unwrap()["value"], "canary-1716");

            let next = s.spawn(|| read(&store));
            decide(&store, 2, ApprovalChoice::Deny);
            assert_eq!(next.join().unwrap()["reason"], "denied");
        });
        assert_eq!(store.gets.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(store.monitor.open_needs_you().is_empty());
    }
}

#[cfg(all(test, unix))]
mod cli_tests {
    use crate::testing::HostHarness;
    use crate::workspace::secrets;
    use std::fs::{self, File};
    use std::io::{BufRead, BufReader, ErrorKind, Write};
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
        } else {
            // A clean shell resolves its own channel's host under its home.
            let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
            assert_eq!(
                crate::cli::resolve_secret_socket(),
                clean_shell_endpoint(&home)
            );
        }
        // A job-control shell starts a foreground CLI with the default SIGINT action.
        unsafe { libc::signal(libc::SIGINT, libc::SIG_DFL) };
        let name = &std::env::var("SECRET_TEST_NAME").unwrap_or_else(|_| "SOCKET_1280".into());
        secrets::take_test_store_calls();
        let _folders =
            crate::release::set_test_experiments(&[crate::release::ReleaseFeature::FolderSecrets]);
        let folder = std::path::Path::new(".");
        let code = match op.as_str() {
            "exec" => crate::cli::workspace::folder_secret_exec(
                folder,
                &[
                    "sh".into(),
                    "-c".into(),
                    "test \"$LOCAL\" = host-canary && test \"$EXTRA\" = terminal-canary".into(),
                ],
            ),
            "run" => crate::cli::run::run_command("build", &[], crate::cli::OutputMode::Text),
            "folder-read" => crate::cli::workspace::folder_secret_read(name, Some(folder)),
            "folder-discover" => crate::cli::workspace::folder_secret_read(name, None),
            "folder-set" => crate::cli::workspace::folder_secret_set(
                name,
                std::env::var_os("SECRET_PROMPT").is_none(),
                false,
                None,
                folder,
            ),
            "folder-rm" => crate::cli::workspace::folder_secret_rm(name, folder),
            "get" => crate::cli::workspace::workspace_secret_get(name, true),
            "set" => crate::cli::workspace::workspace_secret_set(
                name,
                std::env::var_os("SECRET_PROMPT").is_none(),
                true,
                None,
            ),
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

    /// Where a clean shell's secret command finds this binary's own host.
    fn clean_shell_endpoint(home: &std::path::Path) -> std::path::PathBuf {
        let channel = crate::config::build_channel();
        plexi_distribution::host_presence::endpoint_in(&home.join(
            plexi_distribution::host_presence::profile_dir_name(
                channel.as_deref().unwrap_or("stable"),
            ),
        ))
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
    fn secret_cli_fails_closed_without_a_host() {
        let home = tempfile::tempdir().unwrap();
        let endpoint = clean_shell_endpoint(home.path());
        let profile = endpoint.parent().unwrap().to_path_buf();
        fs::create_dir(&profile).unwrap();
        for op in [
            "get",
            "set",
            "delete",
            "folder-read",
            "folder-discover",
            "folder-set",
            "folder-rm",
        ] {
            let mut command = cli(op, &profile);
            command.env("HOME", home.path());
            let output = if matches!(op, "set" | "folder-set") {
                finish(prompt_on_tty(command), 1)
            } else {
                command.output().unwrap()
            };
            assert_eq!(output.status.code(), Some(1));
            assert_eq!(
                String::from_utf8_lossy(&output.stdout).trim(),
                "running 1 test"
            );
            let stderr = String::from_utf8_lossy(&output.stderr);
            if matches!(op, "set" | "folder-set") {
                assert_eq!(stderr.matches("Enter value for").count(), 1);
            }
            let attempted = format!("Plexi host did not answer at {}", endpoint.display());
            assert!(
                stderr.contains(&attempted) && stderr.contains("start Plexi"),
                "{stderr}"
            );
        }
    }

    #[test]
    fn bundle_cli_requires_a_host() {
        let dir = tempfile::tempdir().unwrap();
        let _profile = crate::config::set_test_profile_dir(dir.path().into());
        let root = dir.path().join("work");
        fs::create_dir(&root).unwrap();
        let path = crate::cli::ensure_commands_file(Some(&root));
        fs::write(
            path,
            "[commands]\nbuild = { run = 'touch unexpected', secrets = ['REQUIRED'] }",
        )
        .unwrap();
        for op in ["exec", "run"] {
            let result = cli(op, dir.path())
                .env("SECRET_TEST_SOCKET", dir.path().join("missing.sock"))
                .current_dir(&root)
                .output()
                .unwrap();
            assert_eq!(result.status.code(), Some(1));
            let stderr = String::from_utf8(result.stderr).unwrap();
            assert!(
                stderr.contains("start Plexi") && stderr.contains("missing.sock"),
                "{stderr}"
            );
        }
        assert!(!root.join("unexpected").exists());
    }

    #[test]
    fn outside_cli_secret_get_over_host_socket() {
        let socket_dir = tempfile::tempdir_in("/tmp").unwrap();
        drive_outside_get(Some(socket_dir.path().join("host.sock")));
    }

    /// A clean shell (no `PLEXI_SOCKET`, no override) reaches the channel's
    /// own host at its profile endpoint.
    #[test]
    fn clean_shell_secret_get_finds_the_channel_host() {
        drive_outside_get(None);
    }

    fn drive_outside_get(socket: Option<std::path::PathBuf>) {
        let mut f = Fixture::new(socket);
        let account = secrets::keychain_user_name(&f.name);
        secrets::system_store()
            .set(&account, "fixture-value")
            .unwrap();
        let path = crate::config::config_dir().join("cli-stdout");
        let child = f
            .command("get")
            .stdout(File::create(&path).unwrap())
            .spawn()
            .unwrap();
        let ask = f.accept(None);
        let stdout = || fs::read_to_string(&path).unwrap();
        assert_eq!(stdout().trim(), "running 1 test"); // libtest banner only
        f.decide(&ask, ApprovalChoice::Once);
        finish(child, 0);
        assert_eq!(stdout().lines().last(), Some("fixture-value"));
    }

    use super::worker_test::{join, Timing, TIMING};
    use crate::broker::gate::{ApprovalChoice, PermissionMonitor};
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::process::{Child, Output};

    struct Fixture {
        h: HostHarness,
        monitor: std::sync::Arc<PermissionMonitor>,
        listener: UnixListener,
        socket: Option<std::path::PathBuf>,
        name: String,
    }

    fn until(mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(8);
        while !condition() {
            assert!(Instant::now() < deadline, "timed out waiting for CLI/host");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn finish(mut child: Child, code: i32) -> Output {
        until(|| child.try_wait().unwrap().is_some());
        let output = child.wait_with_output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(code), "{stderr}");
        output
    }

    impl Fixture {
        fn new(socket: Option<std::path::PathBuf>) -> Self {
            let h = HostHarness::new();
            let monitor = PermissionMonitor::for_profile(h.profile_dir()).unwrap();
            let path = socket.clone().unwrap_or_else(|| {
                // The CLI child's HOME is the harness profile dir.
                let endpoint = clean_shell_endpoint(h.profile_dir());
                fs::create_dir_all(endpoint.parent().unwrap()).unwrap();
                endpoint
            });
            let listener = UnixListener::bind(path).unwrap();
            listener.set_nonblocking(true).unwrap();
            Self {
                h,
                monitor,
                listener,
                socket,
                name: format!("SECRET_{}", uuid::Uuid::new_v4().simple()),
            }
        }

        fn command(&self, op: &str) -> Command {
            let mut command = cli(op, self.h.profile_dir());
            command.env("SECRET_TEST_NAME", &self.name).env("PATH", "/usr/bin:/bin");
            if let Some(socket) = &self.socket {
                command.env("SECRET_TEST_SOCKET", socket);
            }
            command
        }

        fn stored(&self) -> zeroize::Zeroizing<String> {
            secrets::system_store()
                .get(&secrets::keychain_user_name(&self.name))
                .unwrap()
        }

        fn accept(&mut self, value: Option<&str>) -> String {
            self.accept_before_dispatch(value, || {})
        }

        fn accept_before_dispatch(&mut self, value: Option<&str>, before: impl FnOnce()) -> String {
            let mut stream = None;
            until(|| {
                stream = match self.listener.accept() {
                    Ok((stream, _)) => Some(stream),
                    Err(e) if e.kind() == ErrorKind::WouldBlock => None,
                    Err(e) => panic!("accept: {e}"),
                };
                stream.is_some()
            });
            let mut stream = stream.unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let peer = crate::broker::caller::PeerWitness::capture(
                super::super::resolve_socket_peer_pid(&stream).unwrap(),
            );
            assert!(matches!(
                self.h.app.classify_socket_peer(Some(&peer)),
                crate::broker::caller::SocketCaller::External(_)
            ));
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            let frame: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(frame["value"].as_str(), value);
            before();
            self.h
                .dispatch_live_secret(&line, &peer, &mut stream)
                .pending_request_id
        }

        fn decide(&self, ask: &str, choice: ApprovalChoice) {
            self.monitor.approve_pending(ask, choice).unwrap();
        }

        fn set(&mut self, op: &str, value: &str) -> (Child, String) {
            let child = self.command(op).env(&self.name, value).spawn().unwrap();
            (child, self.accept(Some(value)))
        }
    }

    #[test]
    fn live_set_prompts_once_without_echo_and_deny_ends_it() {
        let mut f = Fixture::new(None);
        let path = crate::config::config_dir().join("cli-stderr");
        let mut command = f.command("set");
        command.stderr(File::create(&path).unwrap());
        let child = prompt_on_tty(command);
        let ask = f.accept(Some("tty-canary"));
        assert_eq!(f.monitor.open_needs_you().len(), 1);
        let stderr = || fs::read_to_string(&path).unwrap();
        until(|| stderr().contains("Waiting for approval"));
        f.decide(&ask, ApprovalChoice::Deny);
        finish(child, 1);
        let stderr = stderr();
        assert_eq!(stderr.matches("Enter value for").count(), 1);
        let notice = "Waiting for approval in the Plexi window (Needs you)...";
        assert_eq!(stderr.matches(notice).count(), 1);
        assert!(stderr.contains("Request denied") && !stderr.contains("tty-canary"));
        assert_eq!(join().0, 0);
        assert!(f.monitor.open_needs_you().is_empty());
        assert_eq!(
            f.listener.accept().unwrap_err().kind(),
            ErrorKind::WouldBlock
        );
    }

    #[test]
    fn outside_folder_verbs_finish_on_their_own_click() {
        let mut f = Fixture::new(None);
        let _folders =
            crate::release::set_test_experiments(&[crate::release::ReleaseFeature::FolderSecrets]);
        let folder = f.h.profile_dir().canonicalize().unwrap();
        for op in ["folder-set", "folder-read", "folder-rm"] {
            for choice in [ApprovalChoice::Deny, ApprovalChoice::Once] {
                let allow = choice == ApprovalChoice::Once;
                let path = folder.join("folder-cli-stdout");
                let mut command = f.command(op);
                command.stdout(File::create(&path).unwrap());
                let (tx, rx) = std::sync::mpsc::channel();
                TIMING.replace(Timing::ObserveStoreBeforeClick(tx));
                let child = if op == "folder-set" {
                    prompt_on_tty(command)
                } else {
                    command.spawn().unwrap()
                };
                let ask = f.accept((op == "folder-set").then_some("tty-canary"));
                assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), 0);
                let cards = f.monitor.open_needs_you();
                assert_eq!(cards.len(), 1);
                assert_eq!(
                    cards[0].resource,
                    if op == "folder-read" {
                        format!("{}@{}", f.name, folder.display())
                    } else {
                        format!("{}@folder:{}", f.name, folder.display())
                    }
                );
                assert_eq!(fs::read_to_string(&path).unwrap().trim(), "running 1 test");
                f.decide(&ask, choice);
                let result = finish(child, if allow { 0 } else { 1 });
                let stderr = String::from_utf8(result.stderr).unwrap();
                let stdout = fs::read_to_string(&path).unwrap();
                if op == "folder-set" {
                    assert_eq!(stderr.matches("Enter value for").count(), 1);
                    if allow {
                        assert!(stdout.contains(&format!(
                            "Stored '{}' for {} (in-memory)",
                            f.name,
                            folder.display()
                        )));
                    }
                }
                if !allow {
                    assert!(stderr.contains("Request denied"), "{stderr}");
                }
                assert_eq!(
                    stdout.lines().last() == Some("tty-canary"),
                    allow && op == "folder-read"
                );
                assert!(!stderr.contains("tty-canary"));
                assert_eq!(join().0, usize::from(allow));
                assert!(f.monitor.open_needs_you().is_empty());
                assert_eq!(
                    f.listener.accept().unwrap_err().kind(),
                    ErrorKind::WouldBlock
                );
            }
        }
        assert!(!secrets::list_folder_secrets(secrets::folder_store())
            .iter()
            .any(|m| m.name == f.name));
    }

    #[test]
    fn bundle_cli_executes_only_the_approved_definition() {
        let mut f = Fixture::new(None);
        let _on =
            crate::release::set_test_experiments(&[crate::release::ReleaseFeature::FolderSecrets]);
        let root = f.h.workspace_root().canonicalize().unwrap();
        let cfg = secrets::init_workspace(&root, &crate::config::workspace_channel_dir()).unwrap();
        secrets::set_folder_secret(secrets::folder_store(), "LOCAL", &root, "host-canary").unwrap();
        secrets::write_terminal_env_inject(&root, "EXTRA", true).unwrap();
        secrets::system_store()
            .set(
                &secrets::keychain_workspace_name(&cfg.id, "EXTRA"),
                "terminal-canary",
            )
            .unwrap();
        let path = crate::cli::ensure_commands_file(Some(&root));
        let definition = "[commands]\nbuild = { run = 'test -n \"$EXTRA\" && touch approved', secrets = ['EXTRA'] }";
        fs::write(&path, definition).unwrap();
        let logs = crate::testing::test_log::capture_with_debug();
        let mut evidence = String::new();
        for op in ["exec", "run"] {
            if op == "run" {
                fs::write(&path, definition.replace("touch approved", "touch initial")).unwrap();
            }
            let a = f.command(op).current_dir(&root).spawn().unwrap();
            let first = f.accept_before_dispatch(None, || {
                if op == "run" {
                    fs::write(&path, definition).unwrap();
                }
            });
            let b = f.command(op).current_dir(&root).spawn().unwrap();
            let second = f.accept(None);
            assert_ne!(first, second);
            let cards = f.monitor.open_needs_you();
            assert_eq!(cards.len(), 2);
            for card in cards {
                assert!(
                    card.summary.contains("EXTRA") && card.summary.contains(root.to_str().unwrap())
                );
                assert!(card
                    .summary
                    .contains(if op == "exec" { "LOCAL" } else { "build" }));
                evidence.push_str(&card.summary);
            }
            if op == "run" {
                fs::write(&path, definition.replace("touch approved", "touch changed")).unwrap();
            }
            f.decide(&second, ApprovalChoice::Deny);
            finish(b, 1);
            let (count, lines) = join();
            assert_eq!(count, usize::from(op == "exec")); // exec enumerates account names only.
            evidence.push_str(&lines.join("\n"));
            f.decide(&first, ApprovalChoice::Once);
            finish(a, 0);
            evidence.push_str(&join().1.join("\n"));
            assert!(f.monitor.open_needs_you().is_empty());
            let interrupted = f.command(op).current_dir(&root).spawn().unwrap();
            let ask = f.accept(None);
            assert_eq!(
                unsafe { libc::kill(interrupted.id() as _, libc::SIGINT) },
                0
            );
            use std::os::unix::process::ExitStatusExt;
            assert_eq!(
                interrupted.wait_with_output().unwrap().status.signal(),
                Some(libc::SIGINT)
            );
            assert_eq!(join().0, usize::from(op == "exec"));
            assert!(f.monitor.open_needs_you().is_empty());
            assert!(f
                .monitor
                .approve_pending(&ask, ApprovalChoice::Once)
                .is_err());
        }
        assert!(!root.join("initial").exists());
        assert!(root.join("approved").is_file());
        assert!(!root.join("changed").exists());
        evidence.push_str(&logs.lines().join("\n"));
        evidence.push_str(
            &fs::read_to_string(f.h.profile_dir().join("permission-audit.jsonl")).unwrap(),
        );
        if let Ok(entries) = fs::read_dir(f.h.profile_dir().join("rpc")) {
            for entry in entries {
                evidence.push_str(&fs::read_to_string(entry.unwrap().path()).unwrap());
            }
        }
        for canary in ["host-canary", "terminal-canary"] {
            assert!(!evidence.contains(canary));
        }
    }

    fn prompt_on_tty(mut command: Command) -> Child {
        use std::ptr::null_mut;
        let (mut master, mut slave) = (-1, -1);
        // The shell cleanup fixture also uses openpty. Only stdin needs the slave here.
        let (mut tty, slave) = unsafe {
            let rc = libc::openpty(&mut master, &mut slave, null_mut(), null_mut(), null_mut());
            assert_eq!(rc, 0);
            (File::from_raw_fd(master), File::from_raw_fd(slave))
        };
        let echo = || unsafe {
            let mut attrs = std::mem::zeroed();
            assert_eq!(libc::tcgetattr(slave.as_raw_fd(), &mut attrs), 0);
            attrs.c_lflag & libc::ECHO != 0
        };
        assert!(echo());
        let child = command
            .env("SECRET_PROMPT", "1")
            .stdin(slave.try_clone().unwrap())
            .spawn()
            .unwrap();
        until(|| !echo());
        writeln!(tty, "tty-canary").unwrap();
        until(echo);
        child
    }

    #[test]
    fn simultaneous_sets_keep_values_private_and_each_click_owns_one_write() {
        check_simultaneous_sets("set");
    }

    #[test]
    fn concurrent_folder_sets_store_only_the_clicked_value() {
        check_simultaneous_sets("folder-set");
    }

    fn check_simultaneous_sets(op: &str) {
        let mut f = Fixture::new(None);
        let _folders =
            crate::release::set_test_experiments(&[crate::release::ReleaseFeature::FolderSecrets]);
        let logs = crate::testing::test_log::capture_with_debug();
        let (first, a) = f.set(op, "first-1280-canary");
        let (second, b) = f.set(op, "other-1280-canary");
        assert_ne!(a, b);
        f.h.app.sync_attention_queue();
        let mut evidence = format!("{:?}", f.h.app.attention_queue.items);
        assert_eq!(f.monitor.open_needs_you().len(), 2);
        f.decide(&a, ApprovalChoice::Once);
        finish(first, 0);
        let open = f.monitor.open_needs_you();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].id, b);
        f.decide(&b, ApprovalChoice::Deny);
        finish(second, 1);
        // Join in reverse spawn order: B made no calls and A wrote once.
        for calls in [0, 1] {
            let (actual, lines) = join();
            assert_eq!(actual, calls);
            evidence.push_str(&lines.join("\n"));
        }
        if op == "folder-set" {
            let child = f.command("folder-discover").spawn().unwrap();
            let ask = f.accept(None);
            f.decide(&ask, ApprovalChoice::Once);
            let stdout = String::from_utf8(finish(child, 0).stdout).unwrap();
            assert_eq!(stdout.lines().last(), Some("first-1280-canary"));
            let (calls, lines) = join();
            assert_eq!(calls, 2); // Names-only discovery, then the approved fetch.
            evidence.push_str(&lines.join("\n"));
        } else {
            assert_eq!(f.stored().as_str(), "first-1280-canary");
        }
        assert!(f.monitor.open_needs_you().is_empty());
        evidence.push_str(&logs.lines().join("\n"));
        let audit = f.h.profile_dir().join("permission-audit.jsonl");
        evidence.push_str(&fs::read_to_string(audit).unwrap());
        let rpc = f.h.profile_dir().join("rpc");
        if rpc.exists() {
            for entry in fs::read_dir(rpc).unwrap() {
                evidence.push_str(&fs::read_to_string(entry.unwrap().path()).unwrap());
            }
        }
        for canary in ["first-1280-canary", "other-1280-canary"] {
            assert!(!evidence.contains(canary));
        }
    }

    fn assert_no_secret_grant(monitor: &PermissionMonitor) {
        let store = monitor.store();
        assert!(store
            .records()
            .iter()
            .all(|r| !r.target_id.starts_with("secret.")));
    }

    #[test]
    fn deadline_wins_at_claim_and_lost_reply_reports_unknown_write() {
        let mut f = Fixture::new(None);
        let deadline = std::sync::Arc::new(std::sync::Mutex::new(
            Instant::now() + crate::workspace::secrets::APPROVAL_TIMEOUT,
        ));
        TIMING.replace(Timing::Deadline(deadline.clone()));
        let (child, a) = f.set("set", "expired-canary");
        *deadline.lock().unwrap() = Instant::now();
        // Cancellation can already have closed the card before this late click.
        let _late_click = f.monitor.approve_pending(&a, ApprovalChoice::Once);
        assert!(String::from_utf8(finish(child, 1).stderr)
            .unwrap()
            .contains("Approval expired"));
        assert_eq!(join().0, 0);
        assert_no_secret_grant(&f.monitor);
        TIMING.replace(Timing::DropReply);
        let (child, b) = f.set("set", "written-canary");
        f.decide(&b, ApprovalChoice::Once);
        assert!(String::from_utf8(finish(child, 1).stderr)
            .unwrap()
            .contains("Plexi did not confirm the outcome"));
        assert_eq!(join().0, 1);
        assert_eq!(f.stored().as_str(), "written-canary");
    }

    #[test]
    fn two_streams_with_one_label_withdraw_only_their_own_authority() {
        let mut h = HostHarness::new();
        let monitor = PermissionMonitor::for_profile(h.profile_dir()).unwrap();
        let peer = crate::broker::caller::PeerWitness::for_live_pids(&[std::process::id()]);
        let frame = serde_json::json!({"type":"secret_get","global":true,"name":"PAIR_1280",
            "response_file":crate::rpc::response_file("fixture-label","json")})
        .to_string();
        let mut start = || {
            let (mut server, client) = std::os::unix::net::UnixStream::pair().unwrap();
            let ask = h.dispatch_live_secret(&frame, &peer, &mut server);
            (client, ask.pending_request_id)
        };
        let (a, first) = start();
        TIMING.replace(Timing::ExpireAtClaim);
        let (b, second) = start();
        drop(a); // Process is still live. Only A's connection is gone.
        until(|| monitor.show_pending(&first).is_none());
        assert!(monitor
            .approve_pending(&first, ApprovalChoice::Once)
            .is_err());
        monitor
            .approve_pending(&second, ApprovalChoice::Once)
            .unwrap();
        let reply: serde_json::Value = serde_json::from_reader(b).unwrap();
        assert_eq!(reply["reason"], "expired");
        assert_no_secret_grant(&monitor);
        assert_eq!(join().0, 0);
        assert_eq!(join().0, 0);
    }

    #[test]
    fn interrupted_and_killed_clients_withdraw_their_own_asks() {
        use std::os::unix::process::ExitStatusExt;
        let mut f = Fixture::new(None);
        for op in ["set", "get", "delete"] {
            for signal in [libc::SIGINT, libc::SIGKILL] {
                let child = f.command(op).env(&f.name, "unused-canary").spawn().unwrap();
                let ask = f.accept((op == "set").then_some("unused-canary"));
                assert_eq!(unsafe { libc::kill(child.id() as _, signal) }, 0);
                let out = child.wait_with_output().unwrap();
                assert_eq!(out.status.signal(), Some(signal));
                assert_eq!(
                    String::from_utf8_lossy(&out.stdout).trim(),
                    "running 1 test"
                );
                until(|| f.monitor.open_needs_you().is_empty());
                assert!(f
                    .monitor
                    .approve_pending(&ask, ApprovalChoice::Once)
                    .is_err());
                assert_eq!(join().0, 0);
                assert_no_secret_grant(&f.monitor);
            }
        }
        let (child, ask) = f.set("set", "fresh-canary");
        f.decide(&ask, ApprovalChoice::Deny);
        finish(child, 1);
    }
}
