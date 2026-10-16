//! Durable per-session flight recorder.
//!
//! Every host run mints a `sess-<uuid>` (in `PermissionMonitor`). This module
//! writes one JSONL trace per run to `<profile>/sessions/<sess-id>.jsonl`, with
//! `<profile>/sessions/latest.jsonl` pointing at the active or most recent one.
//! Each line carries the shared envelope
//! `{seq, ts, host_session_id, context_id, pane_id, call_id}` plus a `kind` and a
//! kind-specific `data` object, so one chronological read reconstructs a dogfood
//! session without joining five files.
//!
//! The recorder is a tap, never an authority: the sealed
//! `permission-audit.jsonl` and `grants.toml` are untouched, and every failure
//! here is logged and dropped from the trace, never propagated into the host
//! path being observed.
//!
//! Records made before the profile's monitor has opened (and so before the
//! session id exists) wait in a small bounded backlog and are flushed in order
//! when [`adopt_session`] runs. A profile that never opens a monitor (a unit
//! test) never touches the disk.
//!
//! The trace rolls at 10 MB / day change like every other live log; archived
//! segments keep the session id as a filename prefix and the reader stitches
//! them back together by `seq`.

use crate::platform::rolling::RollingFile;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Records held while a profile has no session yet.
const BACKLOG_CAP: usize = 512;

/// One pending record, captured with its own timestamp.
struct Pending {
    ts: String,
    kind: String,
    context_id: Option<u64>,
    pane_id: Option<u64>,
    call_id: Option<String>,
    data: Value,
}

struct Active {
    id: String,
    seq: u64,
    file: Option<RollingFile>,
    write_failures: u64,
}

#[derive(Default)]
struct Slot {
    active: Option<Active>,
    backlog: Vec<Pending>,
}

fn slots() -> &'static Mutex<HashMap<PathBuf, Slot>> {
    static SLOTS: OnceLock<Mutex<HashMap<PathBuf, Slot>>> = OnceLock::new();
    SLOTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// `<profile>/sessions`.
pub fn sessions_dir(profile: &Path) -> PathBuf {
    profile.join("sessions")
}

/// Record into the current profile (`config_dir()`).
pub fn record(
    kind: &str,
    context_id: Option<u64>,
    pane_id: Option<u64>,
    call_id: Option<&str>,
    data: Value,
) {
    record_in(&crate::config::config_dir(), kind, context_id, pane_id, call_id, data);
}

/// Record into an explicit profile directory.
pub fn record_in(
    profile: &Path,
    kind: &str,
    context_id: Option<u64>,
    pane_id: Option<u64>,
    call_id: Option<&str>,
    data: Value,
) {
    let key = crate::platform::path::canonical_or_self(profile);
    let pending = Pending {
        ts: crate::platform::clock::now_rfc3339(),
        kind: kind.to_string(),
        context_id,
        pane_id,
        call_id: call_id.map(str::to_string),
        data,
    };
    let mut map = slots().lock().unwrap_or_else(|e| e.into_inner());
    let slot = map.entry(key.clone()).or_default();
    match slot.active.as_mut() {
        Some(active) => write_row(&key, active, pending),
        None => {
            if slot.backlog.len() >= BACKLOG_CAP {
                slot.backlog.remove(0);
            }
            slot.backlog.push(pending);
        }
    }
}

/// Bind the profile's recorder to the host run's session id, flushing any
/// backlog. Idempotent for the same id; a different id (a restart inside one
/// process, as tests do) starts a fresh trace file.
pub fn adopt_session(profile: &Path, session_id: &str) {
    let key = crate::platform::path::canonical_or_self(profile);
    let mut map = slots().lock().unwrap_or_else(|e| e.into_inner());
    let slot = map.entry(key.clone()).or_default();
    if slot.active.as_ref().is_some_and(|a| a.id == session_id) {
        return;
    }
    let path = sessions_dir(&key).join(format!("{session_id}.jsonl"));
    let file = match RollingFile::open(&path) {
        Ok(file) => Some(file),
        Err(e) => {
            log::error!(
                "flight_recorder: could not open {} (session {session_id} will not be traced): {e}",
                path.display()
            );
            None
        }
    };
    if file.is_some() {
        point_latest_at(&key, session_id);
    }
    log::info!(
        "flight_recorder: session {session_id} trace at {} (backlog {})",
        path.display(),
        slot.backlog.len()
    );
    let mut active = Active {
        id: session_id.to_string(),
        seq: 0,
        file,
        write_failures: 0,
    };
    for pending in std::mem::take(&mut slot.backlog) {
        write_row(&key, &mut active, pending);
    }
    let started = Pending {
        ts: crate::platform::clock::now_rfc3339(),
        kind: "session_started".to_string(),
        context_id: None,
        pane_id: None,
        call_id: None,
        data: json!({ "pid": std::process::id(), "version": env!("CARGO_PKG_VERSION") }),
    };
    write_row(&key, &mut active, started);
    slot.active = Some(active);
}

fn write_row(profile: &Path, active: &mut Active, pending: Pending) {
    active.seq += 1;
    let row = json!({
        "seq": active.seq,
        "ts": pending.ts,
        "host_session_id": active.id,
        "context_id": pending.context_id,
        "pane_id": pending.pane_id,
        "call_id": pending.call_id,
        "kind": pending.kind,
        "data": pending.data,
    });
    let Some(file) = active.file.as_mut() else {
        return;
    };
    let mut line = match serde_json::to_string(&row) {
        Ok(line) => line,
        Err(e) => {
            log::warn!("flight_recorder: could not serialize {} row: {e}", pending.kind);
            return;
        }
    };
    line.push('\n');
    if let Err(e) = file.write_all(line.as_bytes()).and_then(|()| file.flush()) {
        active.write_failures += 1;
        // Log the first failure and then at doubling intervals so a dead disk
        // cannot flood the host log.
        if active.write_failures.is_power_of_two() {
            log::warn!(
                "flight_recorder: write to {} failed ({} so far): {e}",
                sessions_dir(profile).display(),
                active.write_failures
            );
        }
    }
}

/// Atomically repoint `latest.jsonl` at `session_id`. Unix uses a relative
/// symlink swapped in by rename; elsewhere the file holds the session id.
fn point_latest_at(profile: &Path, session_id: &str) {
    let dir = sessions_dir(profile);
    let latest = dir.join("latest.jsonl");
    let target = format!("{session_id}.jsonl");
    #[cfg(unix)]
    let result = {
        let tmp = dir.join(format!("latest.jsonl.tmp-{}", std::process::id()));
        // A leftover temp link from a crashed run is replaced by remove-free
        // overwrite: the rename below also replaces `latest.jsonl`.
        if tmp.symlink_metadata().is_ok() {
            if let Err(e) = std::fs::remove_file(&tmp) {
                log::warn!("flight_recorder: stale {} not removable: {e}", tmp.display());
            }
        }
        std::os::unix::fs::symlink(&target, &tmp).and_then(|()| std::fs::rename(&tmp, &latest))
    };
    #[cfg(not(unix))]
    let result = std::fs::write(&latest, &target);
    match result {
        Ok(()) => log::info!("flight_recorder: latest.jsonl -> {target}"),
        Err(e) => log::warn!(
            "flight_recorder: could not point {} at {target}: {e}",
            latest.display()
        ),
    }
}

// ── Reading ────────────────────────────────────────────────────────────────

/// Session id a trace filename belongs to, stripping an archive suffix
/// (`-YYYY-MM-DD` and an optional `.<n>`).
pub fn session_id_of_file(name: &str) -> Option<String> {
    let stem = name.strip_suffix(".jsonl")?;
    if stem == "latest" || stem.starts_with("latest.") {
        return None;
    }
    let base = stem.split('.').next()?;
    let len = base.len();
    let id = if len > 11
        && base.is_char_boundary(len - 11)
        && crate::platform::rolling::first_date_in(&base[len - 11..]).is_some()
        && base.as_bytes()[len - 11] == b'-'
    {
        &base[..len - 11]
    } else {
        base
    };
    (!id.is_empty()).then(|| id.to_string())
}

/// Resolve `latest` (or a literal id) to a session id.
pub fn resolve_session(profile: &Path, wanted: &str) -> Result<String, String> {
    if wanted != "latest" {
        return Ok(wanted.to_string());
    }
    let latest = sessions_dir(profile).join("latest.jsonl");
    if let Ok(target) = std::fs::read_link(&latest) {
        if let Some(id) = target
            .file_name()
            .and_then(|n| session_id_of_file(&n.to_string_lossy()))
        {
            return Ok(id);
        }
    }
    if let Ok(text) = std::fs::read_to_string(&latest) {
        let first = text.lines().next().unwrap_or("").trim();
        if first.starts_with("sess-") && !first.contains('{') {
            return Ok(first.trim_end_matches(".jsonl").to_string());
        }
    }
    // No pointer: fall back to the most recently modified trace.
    let mut newest: Option<(std::time::SystemTime, String)> = None;
    if let Ok(entries) = std::fs::read_dir(sessions_dir(profile)) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(id) = session_id_of_file(&name) else { continue };
            let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else { continue };
            if newest.as_ref().is_none_or(|(t, _)| modified > *t) {
                newest = Some((modified, id));
            }
        }
    }
    newest
        .map(|(_, id)| id)
        .ok_or_else(|| format!("no recorded sessions in {}", sessions_dir(profile).display()))
}

/// Every row of `session_id` across live and archived segments, in `seq` order.
/// Unparseable lines are skipped and counted in the returned total.
pub fn read_session(profile: &Path, session_id: &str) -> Result<(Vec<Value>, usize), String> {
    let dir = sessions_dir(profile);
    let entries =
        std::fs::read_dir(&dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    let mut rows = Vec::new();
    let mut skipped = 0usize;
    let mut found = false;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if session_id_of_file(&name).as_deref() != Some(session_id) {
            continue;
        }
        found = true;
        let text = match std::fs::read_to_string(entry.path()) {
            Ok(text) => text,
            Err(e) => {
                log::warn!("flight_recorder: cannot read {}: {e}", entry.path().display());
                continue;
            }
        };
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            match serde_json::from_str::<Value>(line) {
                Ok(row) => rows.push(row),
                Err(_) => skipped += 1,
            }
        }
    }
    if !found {
        return Err(format!("no recorded session '{session_id}'"));
    }
    rows.sort_by_key(|row| row["seq"].as_u64().unwrap_or(0));
    Ok((rows, skipped))
}

/// Summary of one recorded session for `debug session list`.
pub fn summarize(session_id: &str, rows: &[Value]) -> Value {
    let mut contexts = std::collections::BTreeSet::new();
    let mut panes = std::collections::BTreeSet::new();
    let mut agents = std::collections::BTreeSet::new();
    for row in rows {
        if let Some(c) = row["context_id"].as_u64() {
            contexts.insert(c);
        }
        if let Some(p) = row["pane_id"].as_u64() {
            panes.insert(p);
        }
        if let Some(agent) = agent_session_of(row) {
            agents.insert(agent);
        }
    }
    json!({
        "session_id": session_id,
        "started": rows.first().map(|r| r["ts"].clone()).unwrap_or(Value::Null),
        "last": rows.last().map(|r| r["ts"].clone()).unwrap_or(Value::Null),
        "records": rows.len(),
        "contexts": contexts,
        "panes": panes,
        "agent_sessions": agents,
    })
}

/// External agent session id carried by a `pane.lifecycle` timeline row.
pub fn agent_session_of(row: &Value) -> Option<String> {
    row["data"]["payload"]["provenance"]["session_id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// All session ids with a trace file, oldest first by last modification.
pub fn list_session_ids(profile: &Path) -> Vec<String> {
    let mut seen: HashMap<String, std::time::SystemTime> = HashMap::new();
    if let Ok(entries) = std::fs::read_dir(sessions_dir(profile)) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(id) = session_id_of_file(&name) else { continue };
            let modified = entry
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            let slot = seen.entry(id).or_insert(modified);
            if modified > *slot {
                *slot = modified;
            }
        }
    }
    let mut ids: Vec<(std::time::SystemTime, String)> =
        seen.into_iter().map(|(id, t)| (t, id)).collect();
    ids.sort();
    ids.into_iter().map(|(_, id)| id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_map_back_to_session_ids() {
        assert_eq!(session_id_of_file("sess-abc.jsonl").as_deref(), Some("sess-abc"));
        assert_eq!(
            session_id_of_file("sess-abc-2026-03-01.jsonl").as_deref(),
            Some("sess-abc")
        );
        assert_eq!(
            session_id_of_file("sess-abc-2026-03-01.2.jsonl").as_deref(),
            Some("sess-abc")
        );
        assert_eq!(session_id_of_file("latest.jsonl"), None);
        assert_eq!(session_id_of_file("notes.txt"), None);
    }

    #[test]
    fn backlog_is_flushed_in_order_once_the_session_is_adopted() {
        let dir = tempfile::tempdir().unwrap();
        record_in(dir.path(), "first", Some(1), None, None, json!({"n": 1}));
        record_in(dir.path(), "second", None, Some(7), Some("call-1"), json!({"n": 2}));
        adopt_session(dir.path(), "sess-test");
        record_in(dir.path(), "third", None, None, None, json!({}));
        let (rows, skipped) = read_session(dir.path(), "sess-test").unwrap();
        assert_eq!(skipped, 0);
        let kinds: Vec<&str> = rows.iter().map(|r| r["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["first", "second", "session_started", "third"]);
        let seqs: Vec<u64> = rows.iter().map(|r| r["seq"].as_u64().unwrap()).collect();
        assert_eq!(seqs, [1, 2, 3, 4]);
        assert_eq!(rows[1]["pane_id"], 7);
        assert_eq!(rows[1]["call_id"], "call-1");
        assert_eq!(rows[0]["host_session_id"], "sess-test");
        assert_eq!(resolve_session(dir.path(), "latest").unwrap(), "sess-test");
    }
}
