//! CLI-to-host RPC over one-shot response files.
//!
//! The CLI writes a request line to the channel's `notify.sock` carrying a
//! `response_file` path; the host answers by writing that file; the CLI polls
//! until the file appears. This module owns every piece of that mechanism:
//!
//! - **Path construction** (client side): `response_file` / `response_file_in`
//!   mint a `<profile>/rpc/<prefix>-<uuid>.<ext>` path and ensure the `rpc/`
//!   subdir exists — the host may be an older binary that writes without
//!   creating directories.
//! - **Polling** (client side): `poll_string` / `poll_bytes` /
//!   `poll_slot_reply` wait for the file, read it, and delete it — on success
//!   *and* on timeout, so an answer that lands just after the deadline does
//!   not orphan a file.
//! - **Writing** (host side): `write_response` / `write_json_response` write
//!   atomically (temp file + rename) so a polling client never observes a
//!   partial response.
//! - **Sweeping** (host side): `sweep_stale` removes `rpc/` entries older
//!   than a few minutes at host startup — the backstop for clients that died
//!   before their response arrived.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// Default client-side wait for a host response.
pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Entries in `rpc/` older than this are startup-sweep garbage: no live
/// request/response round trip spans minutes.
const SWEEP_MAX_AGE: Duration = Duration::from_secs(600);

/// Mint a one-shot response-file path under the active profile's `rpc/`
/// subdir: `<config_dir>/rpc/<prefix>-<uuid>.<ext>`.
pub(crate) fn response_file(prefix: &str, ext: &str) -> String {
    response_file_in(&crate::config::config_dir(), prefix, ext)
}

/// `response_file` against an explicit profile dir — for callers that target
/// a channel other than the ambient one (`plexi host status --channel ...`).
pub(crate) fn response_file_in(profile_dir: &Path, prefix: &str, ext: &str) -> String {
    let rpc_dir = profile_dir.join("rpc");
    if let Err(e) = std::fs::create_dir_all(&rpc_dir) {
        log::warn!("rpc: could not create {rpc_dir:?}: {e}");
    }
    rpc_dir
        .join(format!("{prefix}-{}.{ext}", uuid::Uuid::new_v4()))
        .to_string_lossy()
        .into_owned()
}

pub(crate) enum PollError {
    /// No response arrived within the timeout.
    TimedOut,
    /// The file appeared but could not be read.
    Unreadable(std::io::Error),
}

impl std::fmt::Display for PollError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PollError::TimedOut => write!(f, "timed out waiting for response"),
            PollError::Unreadable(e) => write!(f, "could not read response file: {e}"),
        }
    }
}

/// Poll until `response_file` appears, then read and delete it. `None`
/// timeout waits forever (`plexi notify --timeout 0`). On timeout the file is
/// removed best-effort in case the host answered just after the last check.
pub(crate) fn poll_bytes(
    response_file: &str,
    timeout: Option<Duration>,
) -> Result<Vec<u8>, PollError> {
    let path = PathBuf::from(response_file);
    let deadline = timeout.map(|t| std::time::Instant::now() + t);
    loop {
        if path.exists() {
            return take_response(&path);
        }
        if deadline.is_some_and(|dl| std::time::Instant::now() >= dl) {
            let _ = std::fs::remove_file(&path);
            return Err(PollError::TimedOut);
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// `poll_bytes` decoded as UTF-8 (lossy — responses are host-written JSON).
pub(crate) fn poll_string(
    response_file: &str,
    timeout: Option<Duration>,
) -> Result<String, PollError> {
    poll_bytes(response_file, timeout).map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// A slot-read round trip answers on one of two files: raw payload bytes at
/// `response_file`, or a JSON error at `<response_file>.err` (payloads are
/// byte-clean, so errors cannot share the data file).
pub(crate) enum SlotReply {
    Data(Vec<u8>),
    Err(Vec<u8>),
}

/// Poll for a slot reply on either file; delete both on every exit path.
pub(crate) fn poll_slot_reply(
    response_file: &str,
    timeout: Option<Duration>,
) -> Result<SlotReply, PollError> {
    let data_path = PathBuf::from(response_file);
    let err_path = PathBuf::from(format!("{response_file}.err"));
    let deadline = timeout.map(|t| std::time::Instant::now() + t);
    loop {
        if err_path.exists() {
            let _ = std::fs::remove_file(&data_path);
            return take_response(&err_path).map(SlotReply::Err);
        }
        if data_path.exists() {
            let _ = std::fs::remove_file(&err_path);
            return take_response(&data_path).map(SlotReply::Data);
        }
        if deadline.is_some_and(|dl| std::time::Instant::now() >= dl) {
            let _ = std::fs::remove_file(&data_path);
            let _ = std::fs::remove_file(&err_path);
            return Err(PollError::TimedOut);
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

fn take_response(path: &Path) -> Result<Vec<u8>, PollError> {
    let content = std::fs::read(path);
    let _ = std::fs::remove_file(path);
    content.map_err(PollError::Unreadable)
}

/// Host-side response write: atomic (temp file + rename) so a polling client
/// never reads a partial file. Returns whether the write landed; failure is
/// logged here — the response-file name identifies the request kind.
pub(crate) fn write_response(response_file: &str, body: &[u8]) -> bool {
    let temp_file = format!("{response_file}.tmp");
    if let Err(e) = std::fs::write(&temp_file, body) {
        log::error!("rpc: could not write temp response file {temp_file:?}: {e}");
        return false;
    }
    if let Err(e) = std::fs::rename(&temp_file, response_file) {
        log::error!("rpc: could not rename temp response file to {response_file:?}: {e}");
        let _ = std::fs::remove_file(&temp_file);
        return false;
    }
    true
}

/// `write_response` for a JSON value.
pub(crate) fn write_json_response(response_file: &str, value: serde_json::Value) -> bool {
    match serde_json::to_string(&value) {
        Ok(json) => write_response(response_file, json.as_bytes()),
        Err(e) => {
            log::error!("rpc: could not serialize response for {response_file:?}: {e}");
            false
        }
    }
}

/// Startup sweep: delete `rpc/` entries older than `SWEEP_MAX_AGE`. Catches
/// responses the host wrote after their client gave up, and clients that died
/// mid-request. Best-effort; called once per host boot.
pub(crate) fn sweep_stale(profile_dir: &Path) {
    let rpc_dir = profile_dir.join("rpc");
    let entries = match std::fs::read_dir(&rpc_dir) {
        Ok(entries) => entries,
        Err(_) => return, // no rpc dir yet — nothing to sweep
    };
    let now = std::time::SystemTime::now();
    let mut swept = 0u32;
    for entry in entries.flatten() {
        let age = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok());
        if age.is_some_and(|age| age > SWEEP_MAX_AGE) {
            match std::fs::remove_file(entry.path()) {
                Ok(()) => swept += 1,
                Err(e) => log::warn!("rpc: sweep could not remove {:?}: {e}", entry.path()),
            }
        }
    }
    if swept > 0 {
        log::info!("rpc: swept {swept} stale response file(s) from {rpc_dir:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poll_reads_and_deletes_on_success() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("x-response-1.json");
        std::fs::write(&path, b"{\"ok\":true}").expect("plant response");
        let content = poll_bytes(path.to_str().unwrap(), Some(Duration::from_secs(1)))
            .unwrap_or_else(|e| panic!("poll failed: {e}"));
        assert_eq!(content, b"{\"ok\":true}");
        assert!(!path.exists(), "response file must be deleted on success");
    }

    /// A response that lands only after the client's deadline is the
    /// startup sweep's job (`sweep_removes_only_stale_entries`), not the
    /// poller's — the client process is gone by then.
    #[test]
    fn poll_times_out_when_no_response_arrives() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("x-response-2.json");
        let result = poll_bytes(path.to_str().unwrap(), Some(Duration::from_millis(1)));
        assert!(matches!(result, Err(PollError::TimedOut)));
        assert!(!path.exists(), "timeout must not create files");
    }

    #[test]
    fn slot_reply_prefers_err_file_and_cleans_both() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("slot-response.json");
        let err_path = dir.path().join("slot-response.json.err");
        std::fs::write(&path, b"data").expect("plant data");
        std::fs::write(&err_path, b"{\"ok\":false}").expect("plant err");
        let reply = poll_slot_reply(path.to_str().unwrap(), Some(Duration::from_secs(1)))
            .unwrap_or_else(|e| panic!("poll failed: {e}"));
        assert!(matches!(reply, SlotReply::Err(bytes) if bytes == b"{\"ok\":false}"));
        assert!(!path.exists() && !err_path.exists(), "both files cleaned");
    }

    #[test]
    fn write_response_is_atomic_and_leaves_no_temp() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("w-response.json");
        let path_str = path.to_str().unwrap();
        assert!(write_response(path_str, b"{\"ok\":true}"));
        assert_eq!(
            std::fs::read(&path).expect("read back"),
            b"{\"ok\":true}"
        );
        assert!(
            !Path::new(&format!("{path_str}.tmp")).exists(),
            "temp file must be renamed away"
        );
    }

    #[test]
    fn sweep_removes_only_stale_entries() {
        let dir = tempfile::tempdir().expect("tempdir");
        let rpc_dir = dir.path().join("rpc");
        std::fs::create_dir_all(&rpc_dir).expect("mkdir rpc");
        let stale = rpc_dir.join("old-response.json");
        let fresh = rpc_dir.join("new-response.json");
        std::fs::write(&stale, b"old").expect("plant stale");
        std::fs::write(&fresh, b"new").expect("plant fresh");
        std::fs::OpenOptions::new()
            .write(true)
            .open(&stale)
            .and_then(|f| f.set_modified(std::time::SystemTime::now() - Duration::from_secs(3600)))
            .expect("age stale file");
        sweep_stale(dir.path());
        assert!(!stale.exists(), "stale entry must be swept");
        assert!(fresh.exists(), "fresh entry must survive");
    }

    #[test]
    fn response_file_in_creates_rpc_subdir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = response_file_in(dir.path(), "pane-list-response", "json");
        assert!(dir.path().join("rpc").is_dir(), "rpc/ must exist");
        assert!(path.contains("/rpc/pane-list-response-"));
        assert!(path.ends_with(".json"));
    }
}
