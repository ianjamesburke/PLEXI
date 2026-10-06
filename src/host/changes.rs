//! Text-editor change sets.
//!
//! An agent edit is prepared here and stays off disk until `accept`. Permission
//! is the existing monitor: the same exact `host.files.edit` grant covers
//! propose, accept, refresh, and revert. Creative acceptance is running
//! `accept`. A byte change on disk after propose marks the set stale and
//! accept refuses it until `refresh` rebases the same edit.

use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::broker::gate::{self, Admission, AdmitRequest, PermissionMonitor};
use crate::broker::{
    ActorScope, ActorType, Decision, ExactBinding, GrantDuration, GrantRecord, GrantSource,
    ResourceScope, TargetType,
};

const TOOL: &str = "host.files.edit";

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ChangeSet {
    pub id: String,
    pub agent_id: String,
    pub path: String,
    pub old_string: String,
    pub new_string: String,
    pub base_text: String,
    pub proposed_text: String,
    pub base_digest: String,
    pub status: String,
}

#[derive(Clone, Debug)]
pub struct Prepared {
    pub id: String,
    pub diff: String,
    pub base_digest: String,
    pub applied: bool,
}

#[derive(Debug)]
pub enum GateStop {
    Required { pending_request_id: String },
    Failed(String),
}

struct BoundEdit {
    file: PathBuf,
    path: String,
    workspace: PathBuf,
    input: String,
    agent_id: String,
}

fn actor_id(agent: &str) -> String {
    if agent.starts_with("agent:") {
        agent.to_string()
    } else {
        format!("agent:{agent}")
    }
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

fn digest(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    hex_encode(&hasher.finalize())
}

pub fn edit_input(path: &str, old: &str, new: &str) -> String {
    serde_json::json!({
        "new_string": new,
        "old_string": old,
        "path": path,
    })
    .to_string()
}

fn unified_diff(old: &str, new: &str, path: &str) -> String {
    let mut out = format!("--- a/{path}\n+++ b/{path}\n");
    for line in old.lines() {
        out.push('-');
        out.push_str(line);
        out.push('\n');
    }
    for line in new.lines() {
        out.push('+');
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn sets_dir() -> PathBuf {
    crate::config::config_dir().join("change-sets")
}

fn ledger_path() -> PathBuf {
    crate::config::config_dir().join("change-ledger.jsonl")
}

fn set_path(id: &str) -> PathBuf {
    sets_dir().join(format!("{id}.json"))
}

fn monitor() -> std::sync::Arc<PermissionMonitor> {
    PermissionMonitor::for_profile(&crate::config::config_dir())
}

fn bind(agent: &str, path: &Path, old: &str, new: &str) -> Result<BoundEdit, String> {
    let file = std::fs::canonicalize(path)
        .map_err(|error| format!("file_not_found: {}: {error}", path.display()))?;
    let workspace = file
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("file_not_found: {} has no parent", file.display()))?;
    let workspace = std::fs::canonicalize(&workspace).unwrap_or(workspace);
    let path_text = file.display().to_string();
    Ok(BoundEdit {
        file,
        input: edit_input(&path_text, old, new),
        path: path_text,
        workspace,
        agent_id: actor_id(agent),
    })
}

fn binding(edit: &BoundEdit) -> Result<ExactBinding, String> {
    Ok(ExactBinding {
        actor_type: ActorType::Agent,
        actor_id: edit.agent_id.clone(),
        actor_scope: ActorScope::User,
        trust_origin: "host".to_string(),
        workspace_root: edit.workspace.clone(),
        target_type: TargetType::HostTool,
        target_id: TOOL.to_string(),
        resource_scope: ResourceScope::Path,
        resource_id: Some(edit.path.clone()),
        args_fingerprint: gate::fingerprint_args(&edit.input)?,
        session_id: None,
        package_id: String::new(),
        instance_id: Some(0),
        context_id: Some(0),
        call_id: String::new(),
        operation_id: String::new(),
    })
}

/// Record an exact allow for this edit. The value of the edit is not logged.
pub fn allow_edit(agent: &str, path: &Path, old: &str, new: &str) -> Result<(), String> {
    let edit = bind(agent, path, old, new)?;
    let bound = binding(&edit)?;
    let grant_id = format!("grant_{}", uuid::Uuid::new_v4());
    let record = GrantRecord::from_binding(
        &bound,
        Decision::Allow,
        GrantDuration::Always,
        GrantSource::User,
        &grant_id,
    );
    let mon = monitor();
    mon.store().record(record);
    mon.store().save();
    log::info!(
        "changes: recorded allow agent={} path={} tool={TOOL} grant={grant_id}",
        edit.agent_id,
        edit.path
    );
    Ok(())
}

/// Record a saved once-grant so `plexi changes accept` in another process can
/// commit a set the Assistant already prepared. The editor's Accept button
/// does not use this grant; the click itself is the human decision.
pub fn authorize_accept(id: &str) -> Result<(), String> {
    let set = load_set(id)?;
    let edit = bind(
        &set.agent_id,
        Path::new(&set.path),
        &set.old_string,
        &set.new_string,
    )?;
    let bound = binding(&edit)?;
    let grant_id = format!("grant_{}", uuid::Uuid::new_v4());
    let record = GrantRecord::from_binding(
        &bound,
        Decision::Allow,
        GrantDuration::Once,
        GrantSource::User,
        &grant_id,
    );
    let mon = monitor();
    mon.store().record(record);
    mon.store().save();
    log::info!(
        "changes: recorded once accept grant id={} path={} grant={grant_id}",
        set.id,
        set.path
    );
    Ok(())
}

struct AdmissionOk {
    grant_id: String,
    fingerprint: String,
    resource: String,
    call_id: String,
}

fn admit_edit(edit: &BoundEdit) -> Result<AdmissionOk, GateStop> {
    let call_id = format!("call_{}", uuid::Uuid::new_v4());
    let mon = monitor();
    let admission = mon.admit(AdmitRequest {
        call_id: &call_id,
        tool: TOOL,
        input_json: &edit.input,
        actor_type: ActorType::Agent,
        actor_id: &edit.agent_id,
        actor_scope: ActorScope::User,
        trust_origin: "host",
        workspace_root: &edit.workspace,
        context_id: 0,
        package_id: "",
        instance_id: 0,
        target_type: TargetType::HostTool,
    });
    match admission {
        Admission::Proceed {
            grant_id,
            fingerprint,
            resource_id,
            ..
        } => {
            let resource = resource_id.unwrap_or_default();
            if mon
                .note_use(
                    &edit.agent_id,
                    &call_id,
                    &grant_id,
                    &fingerprint,
                    &resource,
                    "",
                )
                .is_err()
            {
                return Err(GateStop::Failed("permission_denied".to_string()));
            }
            Ok(AdmissionOk {
                grant_id,
                fingerprint,
                resource,
                call_id,
            })
        }
        Admission::Required { pending_request_id } => {
            Err(GateStop::Required { pending_request_id })
        }
        Admission::Denied { code } => Err(GateStop::Failed(code.to_string())),
    }
}

fn note(
    edit: &BoundEdit,
    admitted: &AdmissionOk,
    decision: &str,
    before: &str,
    after: &str,
) -> Result<(), String> {
    monitor()
        .note_outcome(
            &edit.agent_id,
            &admitted.call_id,
            &admitted.grant_id,
            &admitted.fingerprint,
            &admitted.resource,
            "",
            decision,
            before,
            after,
        )
        .map_err(|error| format!("audit_failed: {error}"))
}

fn apply_once(content: &str, old: &str, new: &str) -> Result<String, String> {
    if old.is_empty() {
        return Err("invalid_input: old_string must be non-empty".to_string());
    }
    match content.matches(old).count() {
        0 => Err("edit_no_match: old_string not found".to_string()),
        1 => Ok(content.replacen(old, new, 1)),
        n => Err(format!(
            "edit_ambiguous: old_string matches {n} times; provide a longer unique snippet"
        )),
    }
}

fn save_set(set: &ChangeSet) -> Result<(), String> {
    let dir = sets_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("change_set_store: {}: {error}", dir.display()))?;
    let body = serde_json::to_vec_pretty(set)
        .map_err(|error| format!("change_set_store: serialize: {error}"))?;
    let path = set_path(&set.id);
    std::fs::write(&path, body)
        .map_err(|error| format!("change_set_store: {}: {error}", path.display()))
}

pub fn load_set(id: &str) -> Result<ChangeSet, String> {
    let path = set_path(id);
    let body = std::fs::read(&path)
        .map_err(|error| format!("change_set_not_found: {}: {error}", path.display()))?;
    serde_json::from_slice(&body)
        .map_err(|error| format!("change_set_not_found: {}: {error}", path.display()))
}

pub fn diff_for(set: &ChangeSet) -> String {
    unified_diff(&set.base_text, &set.proposed_text, &set.path)
}

struct IndexedSet {
    canonical_path: String,
    mtime: Option<std::time::SystemTime>,
    set: ChangeSet,
}

struct SetIndex {
    dir: PathBuf,
    /// Name, mtime, and length of each json file. A content rewrite changes
    /// mtime even when the directory mtime does not, so accept is visible on
    /// the next frame without re-reading every set while nothing changed.
    stamp: Vec<(String, Option<std::time::SystemTime>, u64)>,
    sets: Vec<IndexedSet>,
}

fn set_index() -> &'static std::sync::Mutex<Option<SetIndex>> {
    static INDEX: std::sync::OnceLock<std::sync::Mutex<Option<SetIndex>>> = std::sync::OnceLock::new();
    INDEX.get_or_init(|| std::sync::Mutex::new(None))
}

fn directory_stamp(dir: &Path) -> Vec<(String, Option<std::time::SystemTime>, u64)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut stamp = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let meta = entry.metadata().ok();
        let mtime = meta.as_ref().and_then(|meta| meta.modified().ok());
        let len = meta.map(|meta| meta.len()).unwrap_or(0);
        stamp.push((name, mtime, len));
    }
    stamp.sort_by(|a, b| a.0.cmp(&b.0));
    stamp
}

fn read_index(dir: &Path) -> Vec<IndexedSet> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut sets = Vec::new();
    for entry in entries.flatten() {
        let file = entry.path();
        if file.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Ok(body) = std::fs::read(&file) else {
            continue;
        };
        let Ok(set) = serde_json::from_slice::<ChangeSet>(&body) else {
            continue;
        };
        let canonical_path = std::fs::canonicalize(&set.path)
            .map(|canonical| canonical.display().to_string())
            .unwrap_or_else(|_| set.path.clone());
        let mtime = entry.metadata().and_then(|meta| meta.modified()).ok();
        sets.push(IndexedSet {
            canonical_path,
            mtime,
            set,
        });
    }
    sets
}

/// Change sets whose file is `path`, newest first.
///
/// The directory is parsed again only when a set file is added or rewritten.
/// Scanning it on every frame stalled the UI thread while a repaint was pending.
pub fn sets_for_path(path: &Path) -> Vec<ChangeSet> {
    let Ok(want) = std::fs::canonicalize(path) else {
        return Vec::new();
    };
    let want = want.display().to_string();
    let dir = sets_dir();
    let stamp = directory_stamp(&dir);
    let mut guard = set_index().lock().unwrap_or_else(|error| error.into_inner());
    let fresh = !matches!(
        guard.as_ref(),
        Some(cached) if cached.dir == dir && cached.stamp == stamp
    );
    if fresh {
        let started = std::time::Instant::now();
        let sets = read_index(&dir);
        let elapsed = started.elapsed();
        if elapsed.as_millis() >= 50 {
            log::warn!(
                "changes: indexed {} set(s) in {}ms",
                sets.len(),
                elapsed.as_millis()
            );
        }
        *guard = Some(SetIndex {
            dir,
            stamp,
            sets,
        });
    }
    let Some(index) = guard.as_ref() else {
        return Vec::new();
    };
    let mut found: Vec<_> = index
        .sets
        .iter()
        .filter(|entry| entry.canonical_path == want)
        .map(|entry| (entry.mtime, entry.set.clone()))
        .collect();
    found.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    found.into_iter().map(|(_, set)| set).collect()
}

/// Mark a not-yet-committed set stale. Committed and reverted sets are left alone.
pub fn mark_stale(id: &str, reason: &str) -> Result<ChangeSet, String> {
    let mut set = load_set(id)?;
    if set.status == "committed" || set.status == "reverted" || set.status == "rejected" {
        return Ok(set);
    }
    if set.status != "stale" {
        set.status = "stale".to_string();
        save_set(&set)?;
        log::info!(
            "changes: stale id={} agent={} path={} reason={reason}",
            set.id,
            set.agent_id,
            set.path
        );
    }
    Ok(set)
}

/// Drop a pending or stale set without writing the file.
pub fn reject_pending(id: &str) -> Result<ChangeSet, String> {
    let mut set = load_set(id)?;
    if set.status != "pending" && set.status != "stale" {
        return Err(format!(
            "change_set_not_pending: {} is {}",
            set.id, set.status
        ));
    }
    set.status = "rejected".to_string();
    save_set(&set)?;
    log::info!(
        "changes: rejected id={} agent={} path={}",
        set.id,
        set.agent_id,
        set.path
    );
    Ok(set)
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct EditorBufferFile {
    pid: u32,
    buffers: Vec<EditorBufferRecord>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct EditorBufferRecord {
    path: String,
    text: String,
    dirty: bool,
}

/// One open text-editor buffer the host publishes so a later `accept` can see
/// unsaved text without a second permission store.
#[derive(Clone, Debug)]
pub struct OpenEditorBuffer {
    pub path: String,
    pub text: String,
    pub dirty: bool,
}

fn editor_buffers_path() -> PathBuf {
    crate::config::config_dir().join("editor-buffers.json")
}

fn canonical_text(path: &str) -> String {
    std::fs::canonicalize(path)
        .map(|canonical| canonical.display().to_string())
        .unwrap_or_else(|_| path.to_string())
}

/// Replace the host's snapshot of open editor buffers. A dead pid is ignored
/// by [`editor_buffer_conflicts`].
pub fn publish_editor_buffers(buffers: &[OpenEditorBuffer]) {
    if buffers.is_empty() {
        clear_editor_buffers();
        return;
    }
    let path = editor_buffers_path();
    if let Some(parent) = path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            log::warn!(
                "changes: editor buffer snapshot dir {}: {error}",
                parent.display()
            );
            return;
        }
    }
    let payload = EditorBufferFile {
        pid: std::process::id(),
        buffers: buffers
            .iter()
            .map(|buffer| EditorBufferRecord {
                path: canonical_text(&buffer.path),
                text: buffer.text.clone(),
                dirty: buffer.dirty,
            })
            .collect(),
    };
    let body = match serde_json::to_vec(&payload) {
        Ok(body) => body,
        Err(error) => {
            log::warn!("changes: editor buffer snapshot serialize: {error}");
            return;
        }
    };
    if std::fs::read(&path).ok().as_deref() == Some(body.as_slice()) {
        return;
    }
    let tmp = path.with_extension("json.tmp");
    if let Err(error) = std::fs::write(&tmp, &body) {
        log::warn!("changes: editor buffer snapshot write: {error}");
        return;
    }
    if let Err(error) = std::fs::rename(&tmp, &path) {
        log::warn!("changes: editor buffer snapshot rename: {error}");
    }
}

pub fn clear_editor_buffers() {
    let path = editor_buffers_path();
    if path.exists() {
        if let Err(error) = std::fs::remove_file(&path) {
            log::warn!(
                "changes: editor buffer snapshot remove {}: {error}",
                path.display()
            );
        }
    }
}

/// True when a live host has this file open with buffer text other than `base_text`.
pub fn editor_buffer_conflicts(path: &str, base_text: &str) -> bool {
    let raw = match std::fs::read_to_string(editor_buffers_path()) {
        Ok(raw) => raw,
        Err(_) => return false,
    };
    let parsed: EditorBufferFile = match serde_json::from_str(&raw) {
        Ok(parsed) => parsed,
        Err(error) => {
            log::warn!("changes: editor buffer snapshot parse: {error}");
            return false;
        }
    };
    if !crate::host::pane_liveness::pid_is_alive(parsed.pid) {
        return false;
    }
    let want = canonical_text(path);
    parsed.buffers.iter().any(|buffer| {
        canonical_text(&buffer.path) == want && buffer.text != base_text
    })
}

fn append_ledger(set: &ChangeSet, action: &str, before: &str, after: &str) -> Result<(), String> {
    let path = ledger_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("change_ledger: {}: {error}", parent.display()))?;
    }
    let line = serde_json::json!({
        "action": action,
        "agent_id": set.agent_id,
        "change_set_id": set.id,
        "path": set.path,
        "revision_before": before,
        "revision_after": after,
    });
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| format!("change_ledger: {}: {error}", path.display()))?;
    writeln!(file, "{line}")
        .map_err(|error| format!("change_ledger: {}: {error}", path.display()))?;
    log::info!(
        "changes: ledger action={action} id={} agent={} path={}",
        set.id,
        set.agent_id,
        set.path
    );
    Ok(())
}

fn stage(
    path: &str,
    agent: &str,
    old: &str,
    new: &str,
    base_text: String,
    proposed_text: String,
) -> Result<Prepared, String> {
    if base_text == proposed_text {
        return Err("edit_empty: content is unchanged".to_string());
    }
    let base_digest = digest(&base_text);
    let set = ChangeSet {
        id: format!("cs-{}", uuid::Uuid::new_v4()),
        agent_id: actor_id(agent),
        path: path.to_string(),
        old_string: old.to_string(),
        new_string: new.to_string(),
        proposed_text: proposed_text.clone(),
        base_text: base_text.clone(),
        base_digest: base_digest.clone(),
        status: "pending".to_string(),
    };
    let diff = unified_diff(&base_text, &proposed_text, path);
    save_set(&set)?;
    log::info!(
        "changes: proposed id={} agent={} path={} applied=false status=pending",
        set.id,
        set.agent_id,
        set.path
    );
    Ok(Prepared {
        id: set.id,
        diff,
        base_digest,
        applied: false,
    })
}

/// Prepare an edit. The file on disk is not modified.
pub fn prepare_edit(path: &Path, old: &str, new: &str, agent: &str) -> Result<Prepared, String> {
    let file = std::fs::canonicalize(path)
        .map_err(|error| format!("file_not_found: {}: {error}", path.display()))?;
    let base_text = std::fs::read_to_string(&file)
        .map_err(|error| format!("read_failed: {}: {error}", file.display()))?;
    let proposed_text = apply_once(&base_text, old, new)?;
    stage(
        &file.display().to_string(),
        agent,
        old,
        new,
        base_text,
        proposed_text,
    )
}

/// Prepare a full-file replacement. A missing file is not created.
pub fn prepare_replace(path: &Path, content: &str, agent: &str) -> Result<Prepared, String> {
    let file = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let base_text = if file.is_file() {
        std::fs::read_to_string(&file)
            .map_err(|error| format!("read_failed: {}: {error}", file.display()))?
    } else {
        String::new()
    };
    stage(
        &file.display().to_string(),
        agent,
        &base_text,
        content,
        base_text.clone(),
        content.to_string(),
    )
}

pub fn preview(id: &str) -> Result<String, String> {
    let set = load_set(id)?;
    let diff = unified_diff(&set.base_text, &set.proposed_text, &set.path);
    Ok(format!("status={}\n{diff}", set.status))
}

fn read_disk(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|error| format!("read_failed: {path}: {error}"))
}

fn write_disk(path: &str, body: &str) -> Result<(), String> {
    let file = Path::new(path);
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("write_failed: {}: {error}", parent.display()))?;
    }
    std::fs::write(file, body).map_err(|error| format!("write_failed: {path}: {error}"))
}

/// Write a pending set. Caller has already admitted it.
pub fn commit_prepared(id: &str) -> Result<ChangeSet, String> {
    let mut set = load_set(id)?;
    if set.status == "stale" {
        return Err(format!("stale: {} must be refreshed", set.id));
    }
    if set.status != "pending" {
        return Err(format!(
            "change_set_not_pending: {} is {}",
            set.id, set.status
        ));
    }
    let current = if Path::new(&set.path).is_file() {
        read_disk(&set.path)?
    } else if set.base_text.is_empty() {
        String::new()
    } else {
        return Err(format!("read_failed: {} is missing", set.path));
    };
    if digest(&current) != set.base_digest {
        set.status = "stale".to_string();
        save_set(&set)?;
        log::info!(
            "changes: stale id={} agent={} path={} accept refused",
            set.id,
            set.agent_id,
            set.path
        );
        return Err(format!(
            "stale: {} changed on disk; refresh before accept",
            set.id
        ));
    }
    write_disk(&set.path, &set.proposed_text)?;
    let after = digest(&set.proposed_text);
    set.status = "committed".to_string();
    save_set(&set)?;
    if let Err(error) = append_ledger(&set, "commit", &set.base_digest, &after) {
        let _ = write_disk(&set.path, &set.base_text);
        set.status = "pending".to_string();
        let _ = save_set(&set);
        return Err(error);
    }
    log::info!(
        "changes: committed id={} agent={} path={}",
        set.id,
        set.agent_id,
        set.path
    );
    Ok(set)
}

pub fn refresh_prepared(id: &str) -> Result<ChangeSet, String> {
    let mut set = load_set(id)?;
    if set.status != "pending" && set.status != "stale" {
        return Err(format!(
            "change_set_not_pending: {} is {}",
            set.id, set.status
        ));
    }
    let current = read_disk(&set.path)?;
    let proposed = apply_once(&current, &set.old_string, &set.new_string)?;
    set.base_text = current;
    set.base_digest = digest(&set.base_text);
    set.proposed_text = proposed;
    set.status = "pending".to_string();
    save_set(&set)?;
    log::info!(
        "changes: refreshed id={} agent={} path={} status=pending",
        set.id,
        set.agent_id,
        set.path
    );
    Ok(set)
}

pub fn revert_prepared(id: &str) -> Result<ChangeSet, String> {
    let mut set = load_set(id)?;
    if set.status != "committed" {
        return Err(format!(
            "change_set_not_committed: {} is {}",
            set.id, set.status
        ));
    }
    let current = read_disk(&set.path)?;
    if digest(&current) != digest(&set.proposed_text) {
        return Err(format!(
            "revert_conflict: {} no longer matches the committed text",
            set.id
        ));
    }
    write_disk(&set.path, &set.base_text)?;
    let after = digest(&set.base_text);
    let before = digest(&set.proposed_text);
    set.status = "reverted".to_string();
    save_set(&set)?;
    if let Err(error) = append_ledger(&set, "revert", &before, &after) {
        let _ = write_disk(&set.path, &set.proposed_text);
        set.status = "committed".to_string();
        let _ = save_set(&set);
        return Err(error);
    }
    log::info!(
        "changes: reverted id={} agent={} path={}",
        set.id,
        set.agent_id,
        set.path
    );
    Ok(set)
}

fn gate_edit(
    agent: &str,
    path: &Path,
    old: &str,
    new: &str,
) -> Result<(BoundEdit, AdmissionOk), GateStop> {
    let edit = bind(agent, path, old, new).map_err(GateStop::Failed)?;
    let admitted = admit_edit(&edit)?;
    Ok((edit, admitted))
}

pub fn propose_gated(agent: &str, path: &Path, old: &str, new: &str) -> Result<Prepared, GateStop> {
    let (edit, admitted) = gate_edit(agent, path, old, new)?;
    let prepared = prepare_edit(&edit.file, old, new, agent).map_err(GateStop::Failed)?;
    note(
        &edit,
        &admitted,
        "propose",
        &prepared.base_digest,
        &prepared.base_digest,
    )
    .map_err(GateStop::Failed)?;
    Ok(prepared)
}

pub fn accept_gated(id: &str) -> Result<ChangeSet, GateStop> {
    let set = load_set(id).map_err(GateStop::Failed)?;
    if set.status == "pending" && editor_buffer_conflicts(&set.path, &set.base_text) {
        let _ = mark_stale(id, "unsaved editor buffer");
        log::info!(
            "changes: stale id={} agent={} path={} accept refused reason=unsaved editor buffer",
            set.id,
            set.agent_id,
            set.path
        );
        return Err(GateStop::Failed(format!(
            "stale: {id} conflicts with unsaved editor text"
        )));
    }
    let (edit, admitted) = gate_edit(
        &set.agent_id,
        Path::new(&set.path),
        &set.old_string,
        &set.new_string,
    )?;
    match commit_prepared(id) {
        Ok(committed) => {
            let after = digest(&committed.proposed_text);
            note(&edit, &admitted, "commit", &committed.base_digest, &after)
                .map_err(GateStop::Failed)?;
            Ok(committed)
        }
        Err(error) => {
            let decision = if error.starts_with("stale:") {
                "stale"
            } else {
                "error"
            };
            let _ = note(&edit, &admitted, decision, &set.base_digest, "");
            Err(GateStop::Failed(error))
        }
    }
}

pub fn refresh_gated(id: &str) -> Result<ChangeSet, GateStop> {
    let set = load_set(id).map_err(GateStop::Failed)?;
    let (edit, admitted) = gate_edit(
        &set.agent_id,
        Path::new(&set.path),
        &set.old_string,
        &set.new_string,
    )?;
    let refreshed = refresh_prepared(id).map_err(GateStop::Failed)?;
    note(
        &edit,
        &admitted,
        "refresh",
        &set.base_digest,
        &refreshed.base_digest,
    )
    .map_err(GateStop::Failed)?;
    Ok(refreshed)
}

pub fn revert_gated(id: &str) -> Result<ChangeSet, GateStop> {
    let set = load_set(id).map_err(GateStop::Failed)?;
    let (edit, admitted) = gate_edit(
        &set.agent_id,
        Path::new(&set.path),
        &set.old_string,
        &set.new_string,
    )?;
    let reverted = revert_prepared(id).map_err(GateStop::Failed)?;
    note(
        &edit,
        &admitted,
        "revert",
        &digest(&set.proposed_text),
        &digest(&set.base_text),
    )
    .map_err(GateStop::Failed)?;
    Ok(reverted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn isolate() -> (tempfile::TempDir, crate::config::TestProfileDirGuard) {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("profile");
        std::fs::create_dir_all(&profile).unwrap();
        let guard = crate::config::set_test_profile_dir(profile);
        (dir, guard)
    }

    fn sample(dir: &Path) -> PathBuf {
        let file = dir.join("draft.txt");
        std::fs::write(&file, "alpha\n").unwrap();
        file
    }

    #[test]
    fn propose_does_not_touch_disk_and_preview_is_a_diff() {
        let (dir, _guard) = isolate();
        let file = sample(dir.path());
        let before = std::fs::read(&file).unwrap();
        allow_edit("editor-bot", &file, "alpha", "beta").unwrap();
        let prepared = propose_gated("editor-bot", &file, "alpha", "beta").unwrap();
        assert!(!prepared.applied);
        assert_eq!(std::fs::read(&file).unwrap(), before);
        let preview = preview(&prepared.id).unwrap();
        assert!(preview.contains("status=pending"), "{preview}");
        assert!(preview.contains("-alpha\n"), "{preview}");
        assert!(preview.contains("+beta\n"), "{preview}");
    }

    #[test]
    fn ungranted_propose_leaves_the_file_and_audits_the_ask() {
        let (dir, _guard) = isolate();
        let file = sample(dir.path());
        let error = propose_gated("editor-bot", &file, "alpha", "beta").unwrap_err();
        assert!(matches!(error, GateStop::Required { .. }));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "alpha\n");
        let audit =
            std::fs::read_to_string(crate::config::config_dir().join("permission-audit.jsonl"))
                .unwrap();
        assert!(audit.contains("agent:editor-bot"), "{audit}");
        assert!(audit.contains("\"kind\":\"ask\""), "{audit}");
        assert!(!audit.contains("beta"), "{audit}");
    }

    #[test]
    fn accept_records_the_agent_and_revert_restores_the_file() {
        let (dir, _guard) = isolate();
        let file = sample(dir.path());
        allow_edit("editor-bot", &file, "alpha", "beta").unwrap();
        let prepared = propose_gated("editor-bot", &file, "alpha", "beta").unwrap();
        let committed = accept_gated(&prepared.id).unwrap();
        assert_eq!(committed.status, "committed");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "beta\n");
        let audit =
            std::fs::read_to_string(crate::config::config_dir().join("permission-audit.jsonl"))
                .unwrap();
        let ledger =
            std::fs::read_to_string(crate::config::config_dir().join("change-ledger.jsonl"))
                .unwrap();
        assert!(audit.contains("agent:editor-bot"), "{audit}");
        assert!(audit.contains("\"decision\":\"commit\""), "{audit}");
        assert!(
            ledger.contains("\"agent_id\":\"agent:editor-bot\""),
            "{ledger}"
        );
        assert!(ledger.contains("\"action\":\"commit\""), "{ledger}");
        revert_gated(&prepared.id).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "alpha\n");
        let ledger =
            std::fs::read_to_string(crate::config::config_dir().join("change-ledger.jsonl"))
                .unwrap();
        assert!(ledger.contains("\"action\":\"revert\""), "{ledger}");
    }

    #[test]
    fn disk_change_marks_the_set_stale_until_refresh() {
        let (dir, _guard) = isolate();
        let file = sample(dir.path());
        allow_edit("editor-bot", &file, "alpha", "beta").unwrap();
        let prepared = propose_gated("editor-bot", &file, "alpha", "beta").unwrap();
        std::fs::write(&file, "alpha\nextra\n").unwrap();
        let refused = accept_gated(&prepared.id).unwrap_err();
        match refused {
            GateStop::Failed(message) => assert!(message.starts_with("stale:"), "{message}"),
            GateStop::Required { .. } => panic!("accept should be granted"),
        }
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "alpha\nextra\n");
        assert!(preview(&prepared.id).unwrap().starts_with("status=stale\n"));
        refresh_gated(&prepared.id).unwrap();
        assert!(preview(&prepared.id)
            .unwrap()
            .starts_with("status=pending\n"));
        accept_gated(&prepared.id).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "beta\nextra\n");
    }

    #[test]
    fn unsaved_editor_buffer_refuses_accept_without_writing() {
        let (dir, _guard) = isolate();
        let file = sample(dir.path());
        allow_edit("editor-bot", &file, "alpha", "beta").unwrap();
        let prepared = propose_gated("editor-bot", &file, "alpha", "beta").unwrap();
        let canonical = std::fs::canonicalize(&file).unwrap();
        publish_editor_buffers(&[OpenEditorBuffer {
            path: canonical.display().to_string(),
            text: "alpha!\n".to_string(),
            dirty: true,
        }]);
        let refused = accept_gated(&prepared.id).unwrap_err();
        match refused {
            GateStop::Failed(message) => assert!(message.starts_with("stale:"), "{message}"),
            GateStop::Required { .. } => panic!("accept should be granted"),
        }
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "alpha\n");
        assert!(preview(&prepared.id).unwrap().starts_with("status=stale\n"));
    }
}
