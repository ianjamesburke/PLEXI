//! Folder-scoped secrets.
//!
//! A secret is bound to a canonical directory. A terminal pane whose cwd is
//! that directory or a descendant receives the value as an environment
//! variable at spawn. A pane outside the directory does not. Changing cwd
//! later does not change the environment.
//!
//! Agents and app tools do not receive the value unless a permission-gate
//! grant covers that actor, the secret name, and the folder. The audit record
//! carries the name and the folder, never the value.

use std::collections::HashMap;
#[cfg(unix)]
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use zeroize::Zeroizing;

use super::store::SecretStore;
use crate::broker::gate::{self, Admission, AdmitRequest, PermissionMonitor};
use crate::broker::{
    ActorScope, ActorType, Decision, ExactBinding, GrantDuration, GrantRecord, GrantSource,
    ResourceScope, TargetType,
};

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
            kind: ActorType::App,
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

pub fn set_folder_secret(
    store: &dyn SecretStore,
    name: &str,
    folder: &Path,
    value: &str,
) -> Result<PathBuf, String> {
    if !valid_env_name(name) {
        return Err(format!(
            "secret name '{name}' is not an environment variable name"
        ));
    }
    if value.is_empty() {
        return Err("empty value, nothing stored".to_string());
    }
    let folder = canonical_folder(folder)?;
    let account = account_name(&folder, name);
    store
        .set(&account, value)
        .map_err(|err| format!("folder secret store failed: {err}"))?;
    log::info!(
        "folder_secrets: set name={name} folder={} backend={}",
        folder.display(),
        super::folder_store::backend_label()
    );
    Ok(folder)
}

pub fn remove_folder_secret(
    store: &dyn SecretStore,
    name: &str,
    folder: &Path,
) -> Result<(), String> {
    let folder = canonical_folder(folder)?;
    let account = account_name(&folder, name);
    if store.get(&account).is_none() {
        return Err(format!(
            "secret '{name}' not found in {}",
            folder.display()
        ));
    }
    store
        .delete(&account)
        .map_err(|err| format!("folder secret delete failed: {err}"))?;
    log::info!(
        "folder_secrets: rm name={name} folder={}",
        folder.display()
    );
    Ok(())
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
        let Some(value) = store.get(&account) else {
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

pub fn grant_folder_secret(
    monitor: &PermissionMonitor,
    store: &dyn SecretStore,
    actor: &SecretActor,
    name: &str,
    folder: &Path,
) -> Result<String, String> {
    let folder = canonical_folder(folder)?;
    let account = account_name(&folder, name);
    if store.get(&account).is_none() {
        return Err(format!(
            "secret '{name}' not found in {}",
            folder.display()
        ));
    }
    let record = grant_record(actor, name, &folder)?;
    let grant_id = record.grant_id.clone();
    {
        let mut grants = monitor.store();
        grants.record(record);
        grants.save();
    }
    log::info!(
        "folder_secrets: grant name={name} folder={} actor={} grant_id={grant_id}",
        folder.display(),
        actor.id
    );
    Ok(grant_id)
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
    let input = read_input_json(name, &folder_str);
    let call_id = format!("secret-read-{}", uuid::Uuid::new_v4());
    let admission = monitor.admit(AdmitRequest {
        call_id: &call_id,
        tool: SECRET_READ_TOOL,
        input_json: &input,
        actor_type: actor.kind,
        actor_id: &actor.id,
        actor_scope: ActorScope::User,
        trust_origin: TRUST_ORIGIN,
        workspace_root: &folder,
        context_id: 0,
        package_id: "",
        instance_id: 0,
        target_type: TargetType::Secret,
    });
    match admission {
        Admission::Required { pending_request_id } => {
            log::info!(
                "folder_secrets: read permission_required name={name} folder={folder_str} actor={} pending={pending_request_id}",
                actor.id
            );
            Ok(ReadResult::PermissionRequired {
                pending_id: pending_request_id,
            })
        }
        Admission::Denied { code } => {
            log::info!(
                "folder_secrets: read denied name={name} folder={folder_str} actor={} code={code}",
                actor.id
            );
            Ok(ReadResult::Denied)
        }
        Admission::Proceed {
            grant_id,
            fingerprint,
            resource_id,
            ..
        } => {
            let resource = resource_id.unwrap_or_default();
            if monitor
                .note_use(
                    &actor.id,
                    &call_id,
                    &grant_id,
                    &fingerprint,
                    &resource,
                    "",
                )
                .is_err()
            {
                log::info!(
                    "folder_secrets: read blocked before fetch name={name} folder={folder_str} actor={}",
                    actor.id
                );
                return Ok(ReadResult::Denied);
            }
            let account = account_name(&folder, name);
            let value = store.get(&account);
            let outcome = if value.is_some() { "ok" } else { "missing" };
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
                log::info!(
                    "folder_secrets: audit failed after fetch; withholding name={name} folder={folder_str}",
                );
                return Ok(ReadResult::Denied);
            }
            // A human "Allow once" is GrantDuration::Once. Spending it here
            // is what makes the next read ask again. Always grants are left
            // in place; consume_once ignores them.
            monitor.consume_once(&grant_id, None);
            log::info!(
                "folder_secrets: read allowed name={name} folder={folder_str} actor={} grant={grant_id} outcome={outcome}",
                actor.id
            );
            match value {
                Some(value) => Ok(ReadResult::Value(value)),
                None => Ok(ReadResult::NotFound),
            }
        }
    }
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
    let mut sheets = human_sheets()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    if sheets.armed.is_some() {
        return None;
    }
    let ticket = sheets.queue.pop_front()?;
    sheets.armed = Some(ticket.reply);
    Some(FolderSecretSheet {
        pending_id: ticket.pending_id,
        actor_id: ticket.actor_id,
        resource_id: ticket.resource_id,
        summary: ticket.summary,
    })
}

/// Forward a human click (`once`, `session`, `always`, or `deny`) to the
/// waiter that calls `approve_pending`. Returns false when no sheet is armed.
pub fn deliver_folder_secret_choice(choice: &str) -> bool {
    let reply = {
        let mut sheets = human_sheets()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        sheets.armed.take()
    };
    let Some(reply) = reply else {
        return false;
    };
    let delivered = reply.send(choice.to_string()).is_ok();
    log::info!("folder_secrets: human click choice={choice} delivered={delivered}");
    delivered
}

/// Ask the host process's permission monitor when its gate socket is up.
/// `None` means this process should admit locally (no host is listening).
pub fn read_through_host_gate(
    actor: &SecretActor,
    name: &str,
    folder: &Path,
) -> Option<Result<ReadResult, String>> {
    #[cfg(all(unix, not(test)))]
    {
        proxy_read(actor, name, folder)
    }
    #[cfg(not(all(unix, not(test))))]
    {
        let _ = (actor, name, folder);
        None
    }
}

struct SheetTicket {
    pending_id: String,
    actor_id: String,
    resource_id: String,
    summary: String,
    reply: std::sync::mpsc::Sender<String>,
}

struct HumanSheets {
    queue: std::collections::VecDeque<SheetTicket>,
    armed: Option<std::sync::mpsc::Sender<String>>,
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
    let inbox = {
        let mut sheets = human_sheets()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if !sheets.posted.insert(pending_id.to_string()) {
            return;
        }
        let (reply, inbox) = std::sync::mpsc::channel();
        sheets.queue.push_back(SheetTicket {
            pending_id: pending_id.to_string(),
            actor_id: actor_id.to_string(),
            resource_id: resource.to_string(),
            summary: summary.to_string(),
            reply,
        });
        inbox
    };
    let owned_id = pending_id.to_string();
    let monitor = PermissionMonitor::for_profile(&crate::config::config_dir());
    if let Err(err) = std::thread::Builder::new()
        .name("folder-secret-approval".into())
        .spawn(move || wait_for_human_choice(monitor, owned_id, inbox))
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
    inbox: std::sync::mpsc::Receiver<String>,
) {
    let choice = inbox
        .recv_timeout(std::time::Duration::from_secs(180))
        .unwrap_or_else(|_| "deny".to_string());
    let approval = match choice.as_str() {
        "once" => gate::ApprovalChoice::Once,
        "session" => gate::ApprovalChoice::Session,
        "always" => gate::ApprovalChoice::Always,
        _ => gate::ApprovalChoice::Deny,
    };
    match monitor.approve_pending(&pending_id, approval) {
        Ok(()) => log::info!(
            "folder_secrets: human choice={choice} pending={pending_id}"
        ),
        Err(err) => log::info!(
            "folder_secrets: human choice failed pending={pending_id} err={err}"
        ),
    }
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
        libc::getsockname(fd, &mut addr as *mut libc::sockaddr_un as *mut libc::sockaddr, &mut len)
    };
    if rc != 0 || addr.sun_family as i32 != libc::AF_UNIX {
        return None;
    }
    let bytes = unsafe {
        std::slice::from_raw_parts(addr.sun_path.as_ptr() as *const u8, addr.sun_path.len())
    };
    let end = bytes.iter().position(|byte| *byte == 0).unwrap_or(bytes.len());
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
    let kind = value.get("kind").and_then(|item| item.as_str()).unwrap_or("");
    let actor_id = value
        .get("actor_id")
        .and_then(|item| item.as_str())
        .unwrap_or("");
    let name = value.get("name").and_then(|item| item.as_str()).unwrap_or("");
    let folder = value
        .get("folder")
        .and_then(|item| item.as_str())
        .unwrap_or("");
    let actor_kind = match kind {
        "agent" => ActorType::Agent,
        "app" => ActorType::App,
        other => return Err(format!("unknown actor kind {other}")),
    };
    if actor_id.is_empty() || name.is_empty() || folder.is_empty() {
        return Err("folder secret gate: missing actor, name, or folder".to_string());
    }
    let actor = SecretActor {
        kind: actor_kind,
        id: actor_id.to_string(),
    };
    let monitor = PermissionMonitor::for_profile(&crate::config::config_dir());
    let result = read_folder_secret(
        &monitor,
        super::folder_store(),
        &actor,
        name,
        Path::new(folder),
    )?;
    if let ReadResult::PermissionRequired { pending_id } = &result {
        let resource = format!("{name}@{folder}");
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

#[cfg(all(unix, not(test)))]
fn proxy_read(actor: &SecretActor, name: &str, folder: &Path) -> Option<Result<ReadResult, String>> {
    use std::io::{BufRead, BufReader, Write};
    let path = crate::config::config_dir().join("folder-secret-gate.sock");
    if !path.exists() {
        return None;
    }
    let kind = match actor.kind {
        ActorType::Agent => "agent",
        ActorType::App => "app",
        ActorType::System => "system",
        ActorType::ManagedPolicy => "policy",
    };
    let request = serde_json::json!({
        "kind": kind,
        "actor_id": actor.id,
        "name": name,
        "folder": folder,
    });
    let mut stream = match std::os::unix::net::UnixStream::connect(&path) {
        Ok(stream) => stream,
        Err(err) => return Some(Err(format!("folder secret gate: {err}"))),
    };
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(10)));
    if writeln!(stream, "{request}").is_err() {
        return Some(Err("folder secret gate write failed".to_string()));
    }
    let mut line = String::new();
    if BufReader::new(stream).read_line(&mut line).is_err() {
        return Some(Err("folder secret gate read failed".to_string()));
    }
    Some(decode_read_result(&line))
}

#[cfg(all(unix, not(test)))]
fn decode_read_result(line: &str) -> Result<ReadResult, String> {
    let value: serde_json::Value =
        serde_json::from_str(line).map_err(|err| format!("folder secret gate: {err}"))?;
    match value.get("status").and_then(|item| item.as_str()) {
        Some("value") => {
            let text = value
                .get("value")
                .and_then(|item| item.as_str())
                .unwrap_or("")
                .to_string();
            Ok(ReadResult::Value(Zeroizing::new(text)))
        }
        Some("permission_required") => Ok(ReadResult::PermissionRequired {
            pending_id: value
                .get("pending_id")
                .and_then(|item| item.as_str())
                .unwrap_or("")
                .to_string(),
        }),
        Some("denied") => Ok(ReadResult::Denied),
        Some("not_found") => Ok(ReadResult::NotFound),
        Some("error") => Err(value
            .get("error")
            .and_then(|item| item.as_str())
            .unwrap_or("folder secret gate")
            .to_string()),
        _ => Err("folder secret gate: bad response".to_string()),
    }
}

fn grant_record(actor: &SecretActor, name: &str, folder: &Path) -> Result<GrantRecord, String> {
    let folder_str = folder.to_string_lossy().into_owned();
    let input = read_input_json(name, &folder_str);
    let fingerprint = gate::fingerprint_args(&input)?;
    let binding = ExactBinding {
        actor_type: actor.kind,
        actor_id: actor.id.clone(),
        actor_scope: ActorScope::User,
        trust_origin: TRUST_ORIGIN.to_string(),
        workspace_root: folder.to_path_buf(),
        target_type: TargetType::Secret,
        target_id: SECRET_READ_TOOL.to_string(),
        resource_scope: ResourceScope::Path,
        resource_id: Some(format!("{name}@{folder_str}")),
        args_fingerprint: fingerprint,
        session_id: None,
        package_id: String::new(),
        instance_id: Some(0),
        context_id: Some(0),
        call_id: String::new(),
        operation_id: String::new(),
    };
    Ok(GrantRecord::from_binding(
        &binding,
        Decision::Allow,
        GrantDuration::Always,
        GrantSource::User,
        &format!("grant-secret-{}", uuid::Uuid::new_v4()),
    ))
}

fn read_input_json(name: &str, folder: &str) -> String {
    serde_json::json!({
        "name": name,
        "folder": folder,
    })
    .to_string()
}

fn account_name(folder: &Path, name: &str) -> String {
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

    #[test]
    fn agent_without_grant_is_permission_required_and_audit_omits_the_value() {
        let root = scratch();
        let folder = root.path().join("A");
        std::fs::create_dir(&folder).unwrap();
        let store = store();
        let value = "folder-secret-unit-value-9f3c2a";
        set_folder_secret(&store, "FOLDER_E2E_SECRET", &folder, value).unwrap();
        let monitor = PermissionMonitor::ephemeral();
        let actor = SecretActor::agent("reader");
        let blocked = read_folder_secret(&monitor, &store, &actor, "FOLDER_E2E_SECRET", &folder)
            .unwrap();
        match blocked {
            ReadResult::PermissionRequired { pending_id } => {
                assert!(pending_id.starts_with("req_"));
            }
            other => panic!("expected permission_required, got {other:?}"),
        }
        let audit = monitor.audit_records();
        assert!(!audit.is_empty());
        let blob = serde_json::to_string(&audit_as_json(&audit)).unwrap();
        assert!(blob.contains("FOLDER_E2E_SECRET"));
        assert!(blob.contains("ask"));
        assert!(!blob.contains(value));

        grant_folder_secret(&monitor, &store, &actor, "FOLDER_E2E_SECRET", &folder).unwrap();
        let allowed = read_folder_secret(&monitor, &store, &actor, "FOLDER_E2E_SECRET", &folder)
            .unwrap();
        match allowed {
            ReadResult::Value(got) => assert_eq!(got.as_str(), value),
            other => panic!("expected the secret, got {other:?}"),
        }
        let other = SecretActor::agent("someone-else");
        let still = read_folder_secret(&monitor, &store, &other, "FOLDER_E2E_SECRET", &folder)
            .unwrap();
        assert!(matches!(still, ReadResult::PermissionRequired { .. }));

        let app = SecretActor::app("mail");
        let app_blocked =
            read_folder_secret(&monitor, &store, &app, "FOLDER_E2E_SECRET", &folder).unwrap();
        assert!(matches!(
            app_blocked,
            ReadResult::PermissionRequired { .. }
        ));
        grant_folder_secret(&monitor, &store, &app, "FOLDER_E2E_SECRET", &folder).unwrap();
        let app_ok = read_folder_secret(&monitor, &store, &app, "FOLDER_E2E_SECRET", &folder).unwrap();
        assert!(matches!(app_ok, ReadResult::Value(_)));

        let after = serde_json::to_string(&audit_as_json(&monitor.audit_records())).unwrap();
        assert!(!after.contains(value));
        assert!(after.contains("FOLDER_E2E_SECRET"));
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

        let mut withheld = inside;
        crate::host::shell::withhold_folder_secrets(&mut withheld, Some(&folder_a));
        assert!(
            !withheld.contains_key(name),
            "a pane-requested spawn must not keep the folder secret"
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
        let first = read_folder_secret(&monitor, &store, &actor, "FOLDER_E2E_SECRET", &folder)
            .unwrap();
        let pending_id = match first {
            ReadResult::PermissionRequired { pending_id } => pending_id,
            other => panic!("expected permission_required, got {other:?}"),
        };
        monitor
            .approve_pending(&pending_id, crate::broker::gate::ApprovalChoice::Once)
            .unwrap();
        let once = read_folder_secret(&monitor, &store, &actor, "FOLDER_E2E_SECRET", &folder)
            .unwrap();
        match once {
            ReadResult::Value(got) => assert_eq!(got.as_str(), value),
            other => panic!("expected one read, got {other:?}"),
        }
        let again = read_folder_secret(&monitor, &store, &actor, "FOLDER_E2E_SECRET", &folder)
            .unwrap();
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
