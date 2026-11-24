//! Folder-scoped secrets.
//!
//! A secret is bound to a canonical directory. Eligible human terminals can
//! receive it at spawn in that directory or a descendant. Changing cwd later
//! leaves the spawned environment intact.
//!
//! The CLI sends folder reads and mutations to the host command socket.
//! The host checks the peer's authority and admits the exact folder account.
//! The independent folder sheet serves the Unix gate listener.

use std::collections::HashMap;
#[cfg(unix)]
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use zeroize::Zeroizing;

use super::store::{DeleteOutcome, SecretStore};
use crate::broker::gate::{self, AdmitRequest, PermissionMonitor, UseDecision};
use crate::broker::{ActorScope, ActorType, TargetType};

const ACCOUNT_PREFIX: &str = "plexi:folder:";
/// Tool name the permission monitor admits for a folder-secret read.
pub const SECRET_READ_TOOL: &str = "secret.read";
const TRUST_ORIGIN: &str = "host";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderSecretMeta {
    pub name: String,
    pub folder: PathBuf,
}

#[derive(Debug, Clone)]
pub struct SecretActor {
    pub kind: ActorType,
    pub id: String,
}

impl SecretActor {
    pub fn agent(id: &str) -> Self {
        Self {
            kind: ActorType::Agent,
            id: prefixed(id, "agent"),
        }
    }

    pub fn app(id: &str) -> Self {
        Self {
            kind: ActorType::Plugin,
            id: prefixed(id, "app"),
        }
    }
}

fn prefixed(id: &str, prefix: &str) -> String {
    let tag = format!("{prefix}:");
    if id.starts_with(&tag) {
        id.to_string()
    } else {
        format!("{tag}{id}")
    }
}

pub fn valid_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

pub fn canonical_folder(path: &Path) -> Result<PathBuf, String> {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|err| format!("current directory: {err}"))?
            .join(path)
    };
    let canon = abs
        .canonicalize()
        .map_err(|err| format!("folder {}: {err}", abs.display()))?;
    if !canon.is_dir() {
        return Err(format!("not a directory: {}", canon.display()));
    }
    Ok(canon)
}

#[cfg(test)]
pub fn set_folder_secret(
    store: &dyn SecretStore,
    name: &str,
    folder: &Path,
    value: &str,
) -> Result<PathBuf, String> {
    let folder = canonical_folder(folder)?;
    set_folder_account(store, &account_name(&folder, name), name, value)?;
    Ok(folder)
}

pub(super) fn set_folder_account(
    store: &dyn SecretStore,
    account: &str,
    name: &str,
    value: &str,
) -> Result<(), String> {
    if !valid_env_name(name) {
        return Err(format!(
            "secret name '{name}' is not an environment variable name"
        ));
    }
    if value.is_empty() {
        return Err("empty value, nothing stored".to_string());
    }
    store
        .set(account, value)
        .map_err(|err| format!("folder secret store failed: {err}"))
}

#[cfg(test)]
pub fn remove_folder_secret(
    store: &dyn SecretStore,
    name: &str,
    folder: &Path,
) -> Result<(), String> {
    let folder = canonical_folder(folder)?;
    remove_folder_account(store, &account_name(&folder, name), name, &folder)
}

/// Forward one deletion for the selected account. A no-match is a failure.
pub(super) fn remove_folder_account(
    store: &dyn SecretStore,
    account: &str,
    name: &str,
    folder: &Path,
) -> Result<(), String> {
    let outcome = store
        .delete(account)
        .map_err(|err| format!("folder secret delete failed for '{name}': {err}"))?;
    log::info!(
        "folder_secrets: rm name={name} folder={} outcome={outcome:?}",
        folder.display()
    );
    match outcome {
        DeleteOutcome::Deleted => Ok(()),
        DeleteOutcome::NoMatchReported => Err(format!(
            "the secret store returned no match for '{name}' in {}",
            folder.display()
        )),
    }
}

pub fn list_folder_secrets(store: &dyn SecretStore) -> Vec<FolderSecretMeta> {
    let mut metas: Vec<FolderSecretMeta> = store
        .list_with_prefix(ACCOUNT_PREFIX)
        .iter()
        .filter_map(|account| parse_account(account))
        .collect();
    metas.sort_by(|left, right| {
        left.folder
            .cmp(&right.folder)
            .then_with(|| left.name.cmp(&right.name))
    });
    metas
}

/// Secrets whose folder contains `cwd`, nearest folder winning on a name clash.
pub fn env_for_cwd(
    cwd: &Path,
    store: &dyn SecretStore,
) -> Result<Vec<(String, Zeroizing<String>)>, String> {
    ensure_host_gate();
    let cwd = canonical_folder(cwd)?;
    let mut best: HashMap<String, (usize, PathBuf)> = HashMap::new();
    for meta in list_folder_secrets(store) {
        if !cwd.starts_with(&meta.folder) {
            continue;
        }
        let depth = meta.folder.components().count();
        let replace = best
            .get(&meta.name)
            .map(|(have, _)| depth >= *have)
            .unwrap_or(true);
        if replace {
            best.insert(meta.name, (depth, meta.folder));
        }
    }
    let mut names: Vec<String> = best.keys().cloned().collect();
    names.sort();
    let mut out = Vec::with_capacity(names.len());
    for name in names {
        let folder = &best[&name].1;
        let account = account_name(folder, &name);
        let Some(value) = store
            .get_interactive(&account)
            .map_err(|error| error.to_string())?
        else {
            log::warn!(
                "folder_secrets: listed name={name} folder={} but the store returned no value",
                folder.display()
            );
            continue;
        };
        log::info!(
            "folder_secrets: inject name={name} folder={}",
            folder.display()
        );
        out.push((name, value));
    }
    Ok(out)
}

#[derive(Debug)]
pub enum ReadResult {
    Value(Zeroizing<String>),
    PermissionRequired { pending_id: String },
    Denied,
    NotFound,
}

/// Admit the read before the store is touched. A missing grant audits the
/// name and folder and returns [`ReadResult::PermissionRequired`].
pub fn read_folder_secret(
    monitor: &PermissionMonitor,
    store: &dyn SecretStore,
    actor: &SecretActor,
    name: &str,
    folder: &Path,
) -> Result<ReadResult, String> {
    let folder = canonical_folder(folder)?;
    let folder_str = folder.to_string_lossy().into_owned();
    let input = super::SecretScope::Folder {
        path: folder.clone(),
    }
    .input(SECRET_READ_TOOL, name, None)
    .to_string();
    let account = account_name(&folder, name);
    admitted_read(
        monitor,
        actor,
        SECRET_READ_TOOL,
        &input,
        &folder,
        &format!("name={name} folder={folder_str}"),
        || store.get_interactive(&account),
    )
}

/// Admit `tool` for `actor`, then call `fetch` only when a grant covers this
/// exact read. An Allow once grant is spent here. `label` names the secret in
/// logs; it never carries the value.
pub(super) fn admitted_read(
    monitor: &PermissionMonitor,
    actor: &SecretActor,
    tool: &str,
    input: &str,
    workspace_root: &Path,
    label: &str,
    fetch: impl FnOnce() -> Result<Option<Zeroizing<String>>, super::store::SecretError>,
) -> Result<ReadResult, String> {
    admitted_read_with(
        monitor,
        actor,
        tool,
        input,
        workspace_root,
        label,
        |req| Some(monitor.admit_for_use(req, None)),
        fetch,
    )
    .map(|read| read.expect("synchronous admission").result)
}

/// A read and the broker's approval of it, when one admitted it. The value
/// is released only under a permit carrying that approval (stint 1716).
#[derive(Debug)]
pub struct GrantedRead {
    pub result: ReadResult,
    pub(crate) approved: Option<crate::broker::gate::Approved>,
}

impl From<ReadResult> for GrantedRead {
    fn from(result: ReadResult) -> Self {
        Self {
            result,
            approved: None,
        }
    }
}

/// Admit before fetching. A live caller that already received its terminal
/// verdict returns None and never opens the store.
#[allow(clippy::too_many_arguments)]
pub(super) fn admitted_read_with(
    monitor: &PermissionMonitor,
    actor: &SecretActor,
    tool: &str,
    input: &str,
    workspace_root: &Path,
    label: &str,
    admit: impl FnOnce(AdmitRequest) -> Option<UseDecision>,
    fetch: impl FnOnce() -> Result<Option<Zeroizing<String>>, super::store::SecretError>,
) -> Result<Option<GrantedRead>, String> {
    let Some(principal) =
        crate::broker::principal::Principal::from_actor(actor.kind, &actor.id, None)
    else {
        log::info!(
            "secrets: {tool} denied {label} actor={}: not a requesting principal",
            actor.id
        );
        return Ok(Some(ReadResult::Denied.into()));
    };
    let call_id = format!("secret-read-{}", uuid::Uuid::new_v4());
    let Some(admission) = admit(AdmitRequest {
        call_id: &call_id,
        tool,
        input_json: input,
        principal: &principal,
        actor_scope: ActorScope::User,
        trust_origin: TRUST_ORIGIN,
        workspace_root,
        context_id: 0,
        package_id: "",
        instance_id: 0,
        target_type: TargetType::Secret,
    }) else {
        return Ok(None);
    };
    Ok(Some(match admission {
        UseDecision::Required { pending_request_id } => GrantedRead::from({
            log::info!(
                "secrets: {tool} permission_required {label} actor={} pending={pending_request_id}",
                actor.id
            );
            ReadResult::PermissionRequired {
                pending_id: pending_request_id,
            }
        }),
        UseDecision::Denied => GrantedRead::from({
            log::info!("secrets: {tool} denied {label} actor={}", actor.id);
            ReadResult::Denied
        }),
        UseDecision::Allowed {
            approved,
            grant_id,
            fingerprint,
            resource_id: resource,
        } => {
            if monitor
                .note_use(&actor.id, &call_id, &grant_id, &fingerprint, &resource, "")
                .is_err()
            {
                log::info!(
                    "secrets: {tool} blocked before fetch {label} actor={}",
                    actor.id
                );
                return Ok(Some(ReadResult::Denied.into()));
            }
            // A once grant was claimed by the admission: a missing or failed
            // read still spends it. No lock is held across the fetch.
            let value = fetch();
            let outcome = match &value {
                Ok(Some(_)) => "ok",
                Ok(None) => "missing",
                Err(error) => error.outcome(),
            };
            if monitor
                .note_outcome(
                    &actor.id,
                    &call_id,
                    &grant_id,
                    &fingerprint,
                    &resource,
                    "",
                    outcome,
                    "",
                    "",
                )
                .is_err()
            {
                log::info!("secrets: {tool} audit failed after fetch; withholding {label}");
                return Ok(Some(ReadResult::Denied.into()));
            }
            log::info!(
                "secrets: {tool} allowed {label} actor={} grant={grant_id} outcome={outcome}",
                actor.id
            );
            let result = match value {
                Ok(Some(value)) => ReadResult::Value(value),
                Ok(None) => ReadResult::NotFound,
                // A failed approved read is terminal. Never re-admit it or
                // mint a second pending request in an authentication retry.
                Err(error) => return Err(error.to_string()),
            };
            GrantedRead {
                result,
                approved: Some(*approved),
            }
        }
    }))
}

/// One folder-secret ask the Assistant sheet can show. The value is not here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderSecretSheet {
    pub pending_id: String,
    pub actor_id: String,
    pub resource_id: String,
    pub summary: String,
}

/// `true` while a human sheet is waiting for an Assistant pane to adopt it.
pub fn folder_secret_sheet_pending() -> bool {
    !human_sheets()
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .queue
        .is_empty()
}

/// Hand the next sheet to the Assistant. The click reply stays here so the
/// pane does not have to know about the waiter.
pub fn take_folder_secret_sheet() -> Option<FolderSecretSheet> {
    let mut sheets = human_sheets().lock().unwrap_or_else(|err| err.into_inner());
    if sheets.armed.is_some() {
        return None;
    }
    let ticket = sheets.queue.pop_front()?;
    sheets.armed = Some(ArmedClick {
        reply: ticket.reply,
        granted: ticket.granted,
    });
    Some(FolderSecretSheet {
        pending_id: ticket.pending_id,
        actor_id: ticket.actor_id,
        resource_id: ticket.resource_id,
        summary: ticket.summary,
    })
}

/// Forward a human click (`once`, `session`, `always`, `deny`, or `deny_always`) to the
/// waiter that calls `approve_pending`. Returns after that grant is recorded,
/// so the next read sees it. Returns false when no sheet is armed or the
/// waiter does not finish.
pub fn deliver_folder_secret_choice(
    choice: &str,
    evidence: Option<crate::broker::consent::DecisionEvidence>,
) -> bool {
    let armed = {
        let mut sheets = human_sheets().lock().unwrap_or_else(|err| err.into_inner());
        sheets.armed.take()
    };
    let Some(armed) = armed else {
        return false;
    };
    let sent = armed.reply.send((choice.to_string(), evidence)).is_ok();
    let granted = sent
        && armed
            .granted
            .recv_timeout(std::time::Duration::from_secs(5))
            .is_ok();
    log::info!("folder_secrets: human click choice={choice} delivered={sent} granted={granted}");
    granted
}

struct SheetTicket {
    pending_id: String,
    actor_id: String,
    resource_id: String,
    summary: String,
    reply: std::sync::mpsc::Sender<(String, Option<crate::broker::consent::DecisionEvidence>)>,
    /// Signaled after `approve_pending` returns. The click waits on this.
    granted: std::sync::mpsc::Receiver<()>,
}

struct ArmedClick {
    reply: std::sync::mpsc::Sender<(String, Option<crate::broker::consent::DecisionEvidence>)>,
    granted: std::sync::mpsc::Receiver<()>,
}

struct HumanSheets {
    queue: std::collections::VecDeque<SheetTicket>,
    armed: Option<ArmedClick>,
    /// Pending ids already handed to the host gate. Only `post_human_sheet`
    /// reads this, and that waiter exists on a Unix host build. Windows and
    /// `cfg(test)` admit locally, so the field is not part of those binaries.
    #[cfg(all(unix, not(test)))]
    posted: std::collections::HashSet<String>,
}

fn human_sheets() -> &'static std::sync::Mutex<HumanSheets> {
    static SHEETS: std::sync::OnceLock<std::sync::Mutex<HumanSheets>> = std::sync::OnceLock::new();
    SHEETS.get_or_init(|| {
        std::sync::Mutex::new(HumanSheets {
            queue: std::collections::VecDeque::new(),
            armed: None,
            #[cfg(all(unix, not(test)))]
            posted: std::collections::HashSet::new(),
        })
    })
}

#[cfg(all(unix, not(test)))]
fn post_human_sheet(pending_id: &str, actor_id: &str, resource: &str, summary: &str) {
    let (inbox, granted_tx) = {
        let mut sheets = human_sheets().lock().unwrap_or_else(|err| err.into_inner());
        if !sheets.posted.insert(pending_id.to_string()) {
            return;
        }
        let (reply, inbox) = std::sync::mpsc::channel();
        let (granted_tx, granted_rx) = std::sync::mpsc::channel();
        sheets.queue.push_back(SheetTicket {
            pending_id: pending_id.to_string(),
            actor_id: actor_id.to_string(),
            resource_id: resource.to_string(),
            summary: summary.to_string(),
            reply,
            granted: granted_rx,
        });
        (inbox, granted_tx)
    };
    let owned_id = pending_id.to_string();
    let monitor = match PermissionMonitor::for_profile(&crate::config::config_dir()) {
        Ok(monitor) => monitor,
        Err(error) => {
            log::error!("folder_secrets: no approval sheet for {pending_id}: {error}");
            return;
        }
    };
    if let Err(err) = std::thread::Builder::new()
        .name("folder-secret-approval".into())
        .spawn(move || wait_for_human_choice(monitor, owned_id, inbox, granted_tx))
    {
        log::warn!("folder_secrets: approval thread failed: {err}");
    }
    log::info!(
        "folder_secrets: human sheet pending={pending_id} actor={actor_id} resource={resource}"
    );
    nudge_host_repaint();
}

#[cfg(all(unix, not(test)))]
fn wait_for_human_choice(
    monitor: std::sync::Arc<PermissionMonitor>,
    pending_id: String,
    inbox: std::sync::mpsc::Receiver<(String, Option<crate::broker::consent::DecisionEvidence>)>,
    granted: std::sync::mpsc::Sender<()>,
) {
    let (choice, evidence) = inbox
        .recv_timeout(std::time::Duration::from_secs(180))
        .unwrap_or_else(|_| ("deny".to_string(), None));
    let approval = match choice.as_str() {
        "once" => gate::ApprovalChoice::Once,
        "session" => gate::ApprovalChoice::Session,
        "always" => gate::ApprovalChoice::Always,
        "deny_always" => gate::ApprovalChoice::DenyAlways,
        _ => gate::ApprovalChoice::Deny,
    };
    let decided = match evidence {
        Some(evidence) => monitor.decide_pending(&evidence, &pending_id, approval),
        None => monitor.cancel_pending(&pending_id),
    };
    match decided {
        Ok(()) => log::info!("folder_secrets: human choice={choice} pending={pending_id}"),
        Err(err) => {
            log::info!("folder_secrets: human choice failed pending={pending_id} err={err}")
        }
    }
    let _ = granted.send(());
    human_sheets()
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .posted
        .remove(&pending_id);
}

#[cfg(all(unix, not(test)))]
fn nudge_host_repaint() {
    #[cfg(unix)]
    {
        use std::io::Write;
        let path = crate::platform::ipc::endpoint_path();
        let Ok(mut stream) = std::os::unix::net::UnixStream::connect(&path) else {
            return;
        };
        let _ = stream.write_all(b"{\"type\":\"wake\"}\n");
    }
}

/// Start the gate only in the process that is listening on `notify.sock`.
/// A CLI `secret exec` also calls `env_for_cwd` and must not bind this socket.
fn ensure_host_gate() {
    #[cfg(all(unix, not(test)))]
    {
        static STARTED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
        if !process_holds_notify_socket() {
            return;
        }
        let _ = STARTED.get_or_init(|| {
            if let Err(err) = std::thread::Builder::new()
                .name("folder-secret-gate".into())
                .spawn(run_folder_secret_gate)
            {
                log::warn!("folder_secrets: gate thread failed: {err}");
            }
        });
    }
}

#[cfg(all(unix, not(test)))]
fn process_holds_notify_socket() -> bool {
    let wanted = crate::platform::ipc::endpoint_path();
    if open_fds()
        .into_iter()
        .any(|fd| unix_socket_path(fd).as_ref() == Some(&wanted))
    {
        return true;
    }
    // Linux `getsockname` on a unix socket often reports `socket:[inode]`
    // rather than the bound path. `/proc/net/unix` still names the listener.
    linux_listener_inode_is_ours(&wanted)
}

#[cfg(all(unix, not(test)))]
fn linux_listener_inode_is_ours(wanted: &Path) -> bool {
    let Ok(table) = std::fs::read_to_string("/proc/net/unix") else {
        return false;
    };
    let wanted = wanted.to_string_lossy();
    let mut inodes = Vec::new();
    for line in table.lines().skip(1) {
        let mut cols = line.split_whitespace();
        let Some(inode) = cols.nth(6) else {
            continue;
        };
        if cols.next() == Some(wanted.as_ref()) {
            inodes.push(format!("socket:[{inode}]"));
        }
    }
    if inodes.is_empty() {
        return false;
    }
    for fd in open_fds() {
        let Ok(link) = std::fs::read_link(format!("/proc/self/fd/{fd}")) else {
            continue;
        };
        if inodes.iter().any(|inode| link.to_string_lossy() == *inode) {
            return true;
        }
    }
    false
}

#[cfg(all(unix, not(test)))]
fn open_fds() -> Vec<i32> {
    let mut fds = Vec::new();
    for dir in ["/proc/self/fd", "/dev/fd"] {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if let Ok(fd) = entry.file_name().to_string_lossy().parse::<i32>() {
                fds.push(fd);
            }
        }
        if !fds.is_empty() {
            return fds;
        }
    }
    (0..256).collect()
}

#[cfg(all(unix, not(test)))]
fn unix_socket_path(fd: i32) -> Option<std::path::PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockname(
            fd,
            &mut addr as *mut libc::sockaddr_un as *mut libc::sockaddr,
            &mut len,
        )
    };
    if rc != 0 || addr.sun_family as i32 != libc::AF_UNIX {
        return None;
    }
    let bytes = unsafe {
        std::slice::from_raw_parts(addr.sun_path.as_ptr() as *const u8, addr.sun_path.len())
    };
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    if end == 0 {
        return None;
    }
    Some(PathBuf::from(OsStr::from_bytes(&bytes[..end])))
}

#[cfg(all(unix, not(test)))]
fn run_folder_secret_gate() {
    use std::os::unix::fs::PermissionsExt;
    let path = crate::config::config_dir().join("folder-secret-gate.sock");
    let _ = std::fs::remove_file(&path);
    let listener = match std::os::unix::net::UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(err) => {
            log::warn!(
                "folder_secrets: gate bind failed at {}: {err}",
                path.display()
            );
            return;
        }
    };
    if let Err(err) = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)) {
        log::warn!("folder_secrets: gate chmod failed: {err}");
    }
    log::info!("folder_secrets: gate listening at {}", path.display());
    for conn in listener.incoming() {
        match conn {
            Ok(stream) => {
                let _ = std::thread::Builder::new()
                    .name("folder-secret-gate-conn".into())
                    .spawn(move || serve_gate_client(stream));
            }
            Err(err) => log::warn!("folder_secrets: gate accept failed: {err}"),
        }
    }
}

#[cfg(all(unix, not(test)))]
fn serve_gate_client(stream: std::os::unix::net::UnixStream) {
    use std::io::{BufRead, BufReader, Write};
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(10)));
    let mut writer = match stream.try_clone() {
        Ok(writer) => writer,
        Err(err) => {
            log::warn!("folder_secrets: gate clone failed: {err}");
            return;
        }
    };
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let response = match handle_gate_line(&line) {
        Ok(body) => body,
        Err(err) => serde_json::json!({"status": "error", "error": err}).to_string(),
    };
    let _ = writeln!(writer, "{response}");
}

#[cfg(all(unix, not(test)))]
fn handle_gate_line(line: &str) -> Result<String, String> {
    let value: serde_json::Value =
        serde_json::from_str(line).map_err(|err| format!("folder secret gate: {err}"))?;
    let kind = value
        .get("kind")
        .and_then(|item| item.as_str())
        .unwrap_or("");
    let actor_id = value
        .get("actor_id")
        .and_then(|item| item.as_str())
        .unwrap_or("");
    let name = value
        .get("name")
        .and_then(|item| item.as_str())
        .unwrap_or("");
    let folder = value
        .get("folder")
        .and_then(|item| item.as_str())
        .unwrap_or("");
    let actor_kind = match kind {
        "agent" => ActorType::Agent,
        "app" => ActorType::Plugin,
        other => return Err(format!("unknown actor kind {other}")),
    };
    if actor_id.is_empty() || name.is_empty() || folder.is_empty() {
        return Err("folder secret gate: missing actor, name, or folder".to_string());
    }
    let actor = SecretActor {
        kind: actor_kind,
        id: actor_id.to_string(),
    };
    let monitor = PermissionMonitor::for_profile(&crate::config::config_dir())
        .map_err(|error| error.to_string())?;
    let result = read_folder_secret(
        &monitor,
        super::folder_store(),
        &actor,
        name,
        Path::new(folder),
    )?;
    if let ReadResult::PermissionRequired { pending_id } = &result {
        let resource = format!("{name}@folder:{folder}");
        let summary = format!("{name} @ {folder}");
        post_human_sheet(pending_id, actor_id, &resource, &summary);
    }
    Ok(encode_read_result(&result))
}

#[cfg(all(unix, not(test)))]
fn encode_read_result(result: &ReadResult) -> String {
    match result {
        ReadResult::Value(value) => {
            serde_json::json!({"status": "value", "value": value.as_str()}).to_string()
        }
        ReadResult::PermissionRequired { pending_id } => {
            serde_json::json!({"status": "permission_required", "pending_id": pending_id})
                .to_string()
        }
        ReadResult::Denied => serde_json::json!({"status": "denied"}).to_string(),
        ReadResult::NotFound => serde_json::json!({"status": "not_found"}).to_string(),
    }
}

/// Storage identity uses the bytes of the canonical folder selected by the host.
pub(super) fn account_name(folder: &Path, name: &str) -> String {
    format!("{ACCOUNT_PREFIX}{}:{name}", hex_encode(&path_bytes(folder)))
}

fn parse_account(account: &str) -> Option<FolderSecretMeta> {
    let rest = account.strip_prefix(ACCOUNT_PREFIX)?;
    let (hex, name) = rest.rsplit_once(':')?;
    if !valid_env_name(name) {
        return None;
    }
    let bytes = hex_decode(hex)?;
    Some(FolderSecretMeta {
        name: name.to_string(),
        folder: path_from_bytes(&bytes),
    })
}

#[cfg(unix)]
fn path_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(unix)]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(OsStr::from_bytes(bytes))
}

#[cfg(windows)]
fn path_bytes(path: &Path) -> Vec<u8> {
    path.to_string_lossy().as_bytes().to_vec()
}

#[cfg(windows)]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).to_string())
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

fn hex_decode(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    let mut index = 0;
    while index < bytes.len() {
        let hi = from_hex(bytes[index])?;
        let lo = from_hex(bytes[index + 1])?;
        out.push((hi << 4) | lo);
        index += 2;
    }
    Some(out)
}

fn from_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::gate::PermissionMonitor;

    fn store() -> super::super::store::InMemoryKeychain {
        super::super::store::InMemoryKeychain::new()
    }

    fn scratch() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn pane_inside_folder_receives_secret_and_outside_does_not() {
        let root = scratch();
        let folder_a = root.path().join("A");
        let folder_b = root.path().join("B");
        let nested = folder_a.join("nested");
        let sibling = root.path().join("A-extra");
        std::fs::create_dir(&folder_a).unwrap();
        std::fs::create_dir(&folder_b).unwrap();
        std::fs::create_dir(&nested).unwrap();
        std::fs::create_dir(&sibling).unwrap();
        let store = store();
        let value = "folder-secret-unit-value-9f3c2a";
        set_folder_secret(&store, "FOLDER_E2E_SECRET", &folder_a, value).unwrap();

        let inside = env_for_cwd(&folder_a, &store).unwrap();
        assert_eq!(inside.len(), 1);
        assert_eq!(inside[0].0, "FOLDER_E2E_SECRET");
        assert_eq!(inside[0].1.as_str(), value);

        let child = env_for_cwd(&nested, &store).unwrap();
        assert_eq!(child[0].1.as_str(), value);

        assert!(env_for_cwd(&folder_b, &store).unwrap().is_empty());
        assert!(env_for_cwd(&sibling, &store).unwrap().is_empty());
        assert!(env_for_cwd(root.path(), &store).unwrap().is_empty());
    }

    #[test]
    fn nearer_folder_wins_the_same_name() {
        let root = scratch();
        let parent = root.path().join("proj");
        let child = parent.join("sub");
        std::fs::create_dir_all(&child).unwrap();
        let store = store();
        set_folder_secret(&store, "TOKEN", &parent, "parent-value").unwrap();
        set_folder_secret(&store, "TOKEN", &child, "child-value").unwrap();
        let env = env_for_cwd(&child, &store).unwrap();
        assert_eq!(env.len(), 1);
        assert_eq!(env[0].1.as_str(), "child-value");
        let parent_env = env_for_cwd(&parent, &store).unwrap();
        assert_eq!(parent_env[0].1.as_str(), "parent-value");
    }

    #[test]
    fn list_returns_names_and_folders_without_values() {
        let root = scratch();
        let folder = root.path().join("A");
        std::fs::create_dir(&folder).unwrap();
        let store = store();
        let value = "folder-secret-unit-value-9f3c2a";
        set_folder_secret(&store, "FOLDER_E2E_SECRET", &folder, value).unwrap();
        let listed = list_folder_secrets(&store);
        let rendered = format!("{listed:?}");
        assert!(rendered.contains("FOLDER_E2E_SECRET"));
        assert!(rendered.contains(&folder.canonicalize().unwrap().display().to_string()));
        assert!(!rendered.contains(value));
    }

    #[test]
    fn rejects_invalid_names_and_empty_values() {
        let root = scratch();
        std::fs::create_dir(root.path().join("A")).unwrap();
        let store = store();
        let folder = root.path().join("A");
        assert!(set_folder_secret(&store, "1BAD", &folder, "x").is_err());
        assert!(set_folder_secret(&store, "HAS-DASH", &folder, "x").is_err());
        assert!(set_folder_secret(&store, "OK_NAME", &folder, "").is_err());
    }

    fn pending_of(result: ReadResult) -> String {
        match result {
            ReadResult::PermissionRequired { pending_id } => pending_id,
            other => panic!("expected permission_required, got {other:?}"),
        }
    }

    /// Stint 1635: the only allow is a human decision on a pending read.
    /// Neither the denial nor the approval fetches a value; the approval
    /// covers that actor, name and folder only.
    #[test]
    fn approval_of_a_pending_read_is_the_only_allow() {
        let root = scratch();
        let folder = root.path().join("A");
        let other_folder = root.path().join("B");
        std::fs::create_dir(&folder).unwrap();
        std::fs::create_dir(&other_folder).unwrap();
        let store = store();
        let value = "folder-secret-unit-value-9f3c2a";
        set_folder_secret(&store, "FOLDER_E2E_SECRET", &folder, value).unwrap();
        set_folder_secret(&store, "FOLDER_E2E_SECRET", &other_folder, "other-value").unwrap();
        let monitor = PermissionMonitor::ephemeral();

        for actor in [SecretActor::agent("reader"), SecretActor::app("mail")] {
            super::super::store::take_test_value_reads();
            let pending_id = pending_of(
                read_folder_secret(&monitor, &store, &actor, "FOLDER_E2E_SECRET", &folder).unwrap(),
            );
            assert!(pending_id.starts_with("req_"));
            monitor
                .approve_pending(&pending_id, gate::ApprovalChoice::Always)
                .unwrap();
            assert_eq!(
                super::super::store::take_test_value_reads(),
                0,
                "denial and approval must not fetch a value"
            );
            match read_folder_secret(&monitor, &store, &actor, "FOLDER_E2E_SECRET", &folder)
                .unwrap()
            {
                ReadResult::Value(got) => assert_eq!(got.as_str(), value),
                other => panic!("expected the secret, got {other:?}"),
            }
            assert!(
                matches!(
                    read_folder_secret(
                        &monitor,
                        &store,
                        &actor,
                        "FOLDER_E2E_SECRET",
                        &other_folder
                    )
                    .unwrap(),
                    ReadResult::PermissionRequired { .. }
                ),
                "an approval for one folder must not cover another"
            );
        }
        let stranger = SecretActor::agent("someone-else");
        assert!(matches!(
            read_folder_secret(&monitor, &store, &stranger, "FOLDER_E2E_SECRET", &folder).unwrap(),
            ReadResult::PermissionRequired { .. }
        ));

        let audit = serde_json::to_string(&audit_as_json(&monitor.audit_records())).unwrap();
        assert!(audit.contains("FOLDER_E2E_SECRET"));
        assert!(audit.contains("ask"));
        assert!(!audit.contains(value));
    }

    /// Stint 1635: a one-shot approval is spent by the admitted attempt even
    /// when the item is missing or the store fails.
    #[test]
    fn once_approval_is_spent_on_a_missing_or_failed_read() {
        struct FailingRead;
        impl super::super::store::NonDestructiveStore for FailingRead {
            fn get(&self, _: &str) -> Option<Zeroizing<String>> {
                None
            }
            fn get_interactive(
                &self,
                _: &str,
            ) -> Result<Option<Zeroizing<String>>, super::super::store::SecretError> {
                Err(super::super::store::SecretError::Keychain {
                    stage: super::super::store::KeychainStage::Read,
                    status: -25293,
                })
            }
            fn add_new(&self, _: &str, _: &str) -> Result<(), super::super::store::SecretError> {
                panic!("no write expected")
            }
            fn delete_if_value(
                &self,
                _: &str,
                _: &str,
            ) -> Result<(), super::super::store::SecretError> {
                panic!("no delete expected")
            }
            fn list_with_prefix(&self, _: &str) -> Vec<String> {
                Vec::new()
            }
            fn scan_accounts(&self) -> Result<Vec<String>, super::super::store::SecretError> {
                Ok(Vec::new())
            }
        }
        impl SecretStore for FailingRead {
            fn set(&self, _: &str, _: &str) -> Result<(), super::super::store::SecretError> {
                panic!("no write expected")
            }
            fn delete(&self, _: &str) -> Result<DeleteOutcome, super::super::store::SecretError> {
                panic!("no delete expected")
            }
        }

        let root = scratch();
        let folder = root.path().join("A");
        std::fs::create_dir(&folder).unwrap();
        let missing = store();
        let failing = FailingRead;
        let stores: [&dyn SecretStore; 2] = [&missing, &failing];
        for store in stores {
            let monitor = PermissionMonitor::ephemeral();
            let actor = SecretActor::agent("reader");
            let pending_id =
                pending_of(read_folder_secret(&monitor, store, &actor, "ABSENT", &folder).unwrap());
            monitor
                .approve_pending(&pending_id, gate::ApprovalChoice::Once)
                .unwrap();
            let spent = read_folder_secret(&monitor, store, &actor, "ABSENT", &folder);
            assert!(
                matches!(spent, Ok(ReadResult::NotFound) | Err(_)),
                "{spent:?}"
            );
            assert!(
                matches!(
                    read_folder_secret(&monitor, store, &actor, "ABSENT", &folder).unwrap(),
                    ReadResult::PermissionRequired { .. }
                ),
                "the one-shot approval must be spent by the attempt that used it"
            );
        }
    }

    fn audit_as_json(records: &[crate::broker::gate::AuditFact]) -> Vec<serde_json::Value> {
        records
            .iter()
            .map(|fact| {
                serde_json::json!({
                    "kind": fact.kind,
                    "actor": fact.actor,
                    "call_id": fact.call_id,
                    "grant_id": fact.grant_id,
                    "resource_id": fact.resource_id,
                    "args_fingerprint": fact.args_fingerprint,
                    "operation_id": fact.operation_id,
                    "decision": fact.decision,
                    "revision_before": fact.revision_before,
                    "revision_after": fact.revision_after,
                })
            })
            .collect()
    }

    #[test]
    fn build_env_injects_only_inside_the_folder() {
        let _enabled =
            crate::release::set_test_experiments(&[crate::release::ReleaseFeature::FolderSecrets]);
        let root = scratch();
        let _profile = crate::config::set_test_profile_dir(root.path().join("profile"));
        let folder_a = root.path().join("A");
        let folder_b = root.path().join("B");
        std::fs::create_dir(&folder_a).unwrap();
        std::fs::create_dir(&folder_b).unwrap();
        let name = "PLEXI_FOLDER_SECRET_BUILD_ENV";
        let value = "folder-secret-unit-value-9f3c2a";
        set_folder_secret(super::super::folder_store(), name, &folder_a, value).unwrap();
        let inside = crate::host::shell::build_env(Some(&folder_a));
        assert_eq!(inside.get(name).map(String::as_str), Some(value));
        assert!(
            inside
                .keys()
                .all(|key| !key.starts_with("PLEXI_TERMINAL_ENV_VALUE_")),
            "folder secrets must not be copied into PLEXI_TERMINAL_ENV_VALUE_*"
        );
        let outside = crate::host::shell::build_env(Some(&folder_b));
        assert!(!outside.contains_key(name));
        assert_eq!(
            super::super::backend_label(),
            "in-memory",
            "this test must use the in-memory folder store"
        );

        let withheld = crate::host::shell::build_env_with_secrets(
            Some(&folder_a),
            crate::host::shell::SecretPolicy {
                terminal: true,
                folder: false,
            },
        );
        assert!(
            !withheld.contains_key(name),
            "a pane-requested spawn must not receive the folder secret"
        );
        assert!(withheld
            .keys()
            .all(|key| !key.starts_with("PLEXI_TERMINAL_ENV_VALUE_")));
    }

    #[test]
    fn human_click_once_allows_exactly_one_read() {
        let root = scratch();
        let folder = root.path().join("A");
        std::fs::create_dir(&folder).unwrap();
        let store = store();
        let value = "folder-secret-unit-value-9f3c2a";
        set_folder_secret(&store, "FOLDER_E2E_SECRET", &folder, value).unwrap();
        let monitor = PermissionMonitor::ephemeral();
        let actor = SecretActor::agent("reader");
        let first =
            read_folder_secret(&monitor, &store, &actor, "FOLDER_E2E_SECRET", &folder).unwrap();
        let pending_id = match first {
            ReadResult::PermissionRequired { pending_id } => pending_id,
            other => panic!("expected permission_required, got {other:?}"),
        };
        monitor
            .approve_pending(&pending_id, crate::broker::gate::ApprovalChoice::Once)
            .unwrap();
        let once =
            read_folder_secret(&monitor, &store, &actor, "FOLDER_E2E_SECRET", &folder).unwrap();
        match once {
            ReadResult::Value(got) => assert_eq!(got.as_str(), value),
            other => panic!("expected one read, got {other:?}"),
        }
        let again =
            read_folder_secret(&monitor, &store, &actor, "FOLDER_E2E_SECRET", &folder).unwrap();
        assert!(
            matches!(again, ReadResult::PermissionRequired { .. }),
            "a second read must ask again"
        );
        let blob = serde_json::to_string(&audit_as_json(&monitor.audit_records())).unwrap();
        assert!(blob.contains("allow_once"));
        assert!(blob.contains("FOLDER_E2E_SECRET"));
        assert!(!blob.contains(value));
    }
}

/// Stint 1635: `secret rm` issues one delete and forwards its outcome. The
/// store panics on any read or list, so an existence pre-check fails here.
#[cfg(test)]
mod removal_outcome_tests {
    use super::super::store::{DeleteOnlyStore, SecretError};
    use super::*;

    #[test]
    fn remove_forwards_one_delete_outcome() {
        let folder = tempfile::tempdir().unwrap();
        let deleted = DeleteOnlyStore::answering(|| Ok(DeleteOutcome::Deleted));
        remove_folder_secret(&deleted, "TOKEN", folder.path()).unwrap();
        assert_eq!(deleted.deletes(), 1);

        let none = DeleteOnlyStore::answering(|| Ok(DeleteOutcome::NoMatchReported));
        let error = remove_folder_secret(&none, "TOKEN", folder.path()).unwrap_err();
        assert!(error.contains("returned no match for 'TOKEN'"), "{error}");
        assert_eq!(none.deletes(), 1);

        let failed = DeleteOnlyStore::answering(|| {
            Err(SecretError::Keychain {
                stage: super::super::store::KeychainStage::Delete,
                status: -25293,
            })
        });
        let error = remove_folder_secret(&failed, "TOKEN", folder.path()).unwrap_err();
        assert!(error.contains("delete failed for 'TOKEN'"), "{error}");
        assert!(error.contains("-25293"), "{error}");
        assert!(!error.contains("no match"), "{error}");
        assert_eq!(failed.deletes(), 1);
    }
}

#[cfg(test)]
mod approval_retry_tests {
    use super::super::store::SecretError;
    use super::*;

    #[test]
    fn concurrent_secret_requests_cannot_share_one_approval() {
        let monitor = PermissionMonitor::ephemeral();
        let root = tempfile::tempdir().unwrap();
        let actor = SecretActor {
            kind: ActorType::Agent,
            id: "pane:0957-concurrent".into(),
        };
        const INPUT: &str = r#"{"name":"FOO","scope":"global"}"#;
        let first = admitted_read(
            &monitor,
            &actor,
            SECRET_READ_TOOL,
            INPUT,
            root.path(),
            "FOO",
            || panic!("unapproved fetch"),
        )
        .unwrap();
        let ReadResult::PermissionRequired { pending_id } = first else {
            panic!("expected approval")
        };
        monitor
            .approve_pending(&pending_id, gate::ApprovalChoice::Once)
            .unwrap();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (finish_tx, finish_rx) = std::sync::mpsc::channel();
        let timeout = crate::testing::load_aware_timeout(std::time::Duration::from_secs(10));
        std::thread::scope(|threads| {
            let target = (&monitor, &actor, root.path());
            let worker = threads.spawn(move || {
                admitted_read(
                    target.0,
                    target.1,
                    SECRET_READ_TOOL,
                    INPUT,
                    target.2,
                    "FOO",
                    || {
                        started_tx.send(()).unwrap();
                        finish_rx.recv_timeout(timeout).unwrap();
                        Ok(None)
                    },
                )
            });
            started_rx.recv_timeout(timeout).unwrap();
            // While the approved request is waiting in Keychain, a separate
            // request must not fetch with that same one-shot authorization.
            let second = admitted_read(
                &monitor,
                &actor,
                SECRET_READ_TOOL,
                INPUT,
                root.path(),
                "FOO",
                || Ok(None),
            );
            finish_tx.send(()).unwrap();
            assert!(matches!(worker.join().unwrap(), Ok(ReadResult::NotFound)));
            assert!(matches!(second, Ok(ReadResult::PermissionRequired { .. })));
        });
    }

    #[test]
    fn one_approval_retry_finishes_even_when_keychain_fails_or_item_is_missing() {
        for failure in [None, Some(-25293), Some(-50)] {
            let monitor = PermissionMonitor::ephemeral();
            let root = tempfile::tempdir().unwrap();
            let actor = SecretActor {
                kind: ActorType::Agent,
                id: "pane:0957".into(),
            };
            let reads = std::cell::Cell::new(0);
            let read = || {
                admitted_read(
                    &monitor,
                    &actor,
                    SECRET_READ_TOOL,
                    r#"{"name":"FOO","scope":"global"}"#,
                    root.path(),
                    "FOO",
                    || {
                        reads.set(reads.get() + 1);
                        match failure {
                            None => Ok(None),
                            Some(-25293) => Err(SecretError::Keychain {
                                stage: super::super::store::KeychainStage::Read,
                                status: -25293,
                            }),
                            _ => Err(SecretError::Backend("test failure".into())),
                        }
                    },
                )
            };
            let ReadResult::PermissionRequired { pending_id } = read().unwrap() else {
                panic!("expected approval")
            };
            assert_eq!(reads.get(), 0);
            monitor
                .approve_pending(&pending_id, gate::ApprovalChoice::Once)
                .unwrap();
            let result = read();
            if failure.is_none() {
                assert!(matches!(result, Ok(ReadResult::NotFound)));
            } else {
                assert!(result.is_err());
            }
            assert_eq!(reads.get(), 1);
            assert!(
                monitor.list_pending().is_empty(),
                "an approved retry must not post a new banner"
            );
            let expected = match failure {
                None => "missing",
                Some(-25293) => "auth_failed",
                _ => "backend_error",
            };
            assert!(monitor
                .audit_records()
                .iter()
                .any(|row| row.decision == expected));
            // A different command still needs its own approval: do not turn a
            // one-shot failure into a reusable grant.
            assert!(matches!(read(), Ok(ReadResult::PermissionRequired { .. })));
            assert_eq!(reads.get(), 1);
        }
    }
}

#[cfg(test)]
mod lazy_startup_tests {
    use super::*;
    #[test]
    fn disabled_folder_bindings_and_agent_startup_never_read_values() {
        let root = tempfile::tempdir().unwrap();
        let _profile = crate::config::set_test_profile_dir(root.path().join("profile"));
        let _shared = crate::config::set_test_shared_dir(root.path().join("shared"));
        let folder = root.path().join("project");
        std::fs::create_dir(&folder).unwrap();
        super::super::init_workspace(&folder, &crate::config::workspace_channel_dir()).unwrap();
        set_folder_secret(
            super::super::folder_store(),
            "DEMO_TOKEN_0957",
            &folder,
            "test-only",
        )
        .unwrap();
        for enabled in [false, true] {
            let features = if enabled {
                vec![crate::release::ReleaseFeature::FolderSecrets]
            } else {
                vec![]
            };
            let _features = crate::release::set_test_experiments(&features);
            super::super::store::take_test_value_reads();
            // Disabled: every class allowed but the experiment off. Enabled:
            // an agent's policy, which resolves no class at all.
            let policy = if enabled {
                crate::host::shell::SecretPolicy::NONE
            } else {
                crate::host::shell::SecretPolicy::ALL
            };
            let env = crate::host::shell::build_env_with_secrets(Some(&folder), policy);
            assert!(!env.contains_key("DEMO_TOKEN_0957"));
            assert_eq!(
                super::super::store::take_test_value_reads(),
                0,
                "disabled bindings and withheld agent secrets need names only"
            );
        }
        super::super::folder_store()
            .delete(&account_name(&folder, "DEMO_TOKEN_0957"))
            .unwrap();
    }
}
