//! CLI-to-host RPC over `notify.sock` reply streams (with a file fallback for
//! direct-injected unit tests).
//!
//! The CLI writes a request line to the channel's `notify.sock` carrying a
//! `response_file` correlation token; the host registers the open socket stream
//! in [`ReplyRegistry`] and writes the response directly back over that stream.
//! When a unit test invokes a handler directly without a live socket stream in
//! [`ReplyRegistry`], `write_response` falls back to an atomic file write at
//! `response_file`.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::platform::ipc::IpcStream;

/// Default client-side wait for a host response.
pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// Fallback file-poll interval for unit tests that bypass `notify.sock`.
const POLL_INTERVAL: Duration = Duration::from_millis(5);

/// Entries in `rpc/` or `ReplyRegistry` older than this are startup/stale
/// garbage: no live request/response round trip spans ten minutes.
const SWEEP_MAX_AGE: Duration = Duration::from_secs(600);

/// Wire prefix distinguishing a `.err` slot-reply frame from raw slot bytes on
/// the same socket stream.
const SLOT_ERR_PREFIX: &[u8] = b"\x00PLEXI_ERR:";

/// Wire sentinel representing a valid 0-byte slot payload on a socket stream
/// so the client can distinguish an empty slot from a disconnected peer that
/// closed the stream without answering.
const SLOT_EMPTY_SENTINEL: &[u8] = b"\x00PLEXI_EMPTY\x00";

type StreamMap = Mutex<HashMap<String, (Instant, IpcStream)>>;

fn host_reply_streams() -> &'static StreamMap {
    static REGISTRY: OnceLock<StreamMap> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn client_reply_streams() -> &'static StreamMap {
    static REGISTRY: OnceLock<StreamMap> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn prune_expired(map: &mut HashMap<String, (Instant, IpcStream)>) {
    let now = Instant::now();
    map.retain(|_, (registered_at, _)| {
        now.saturating_duration_since(*registered_at) <= SWEEP_MAX_AGE
    });
}

/// Host-side registry of open `notify.sock` streams awaiting a reply, keyed by
/// the request's `response_file` correlation token.
pub(crate) struct ReplyRegistry;

impl ReplyRegistry {
    /// Associate `stream` with `response_file` so the subsequent
    /// [`write_response`] / [`write_json_response`] call answers over the open
    /// socket instead of touching the filesystem.
    pub(crate) fn register(response_file: impl Into<String>, stream: IpcStream) {
        #[cfg(unix)]
        let _ = stream.set_nonblocking(false);
        let mut guard = host_reply_streams()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        prune_expired(&mut guard);
        guard.insert(response_file.into(), (Instant::now(), stream));
    }

    /// Take the registered stream for `response_file`, if any.
    pub(crate) fn take(response_file: &str) -> Option<IpcStream> {
        let mut guard = host_reply_streams()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        guard.remove(response_file).map(|(_, stream)| stream)
    }

    /// Drop any registered stream for `response_file` without writing.
    pub(crate) fn remove(response_file: &str) {
        let mut guard = host_reply_streams()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        guard.remove(response_file);
    }
}

/// Client-side registration of the connected `notify.sock` stream for a
/// pending `response_file` correlation token.
pub(crate) fn register_client_stream(response_file: impl Into<String>, stream: IpcStream) {
    #[cfg(unix)]
    let _ = stream.set_nonblocking(false);
    let mut guard = client_reply_streams()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    prune_expired(&mut guard);
    guard.insert(response_file.into(), (Instant::now(), stream));
}

fn take_client_stream(response_file: &str) -> Option<IpcStream> {
    let mut guard = client_reply_streams()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    guard.remove(response_file).map(|(_, stream)| stream)
}

/// Extract a `"response_file"` string token from a serialized JSON request
/// line, if present and non-empty.
pub(crate) fn extract_response_file_token(line: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    let token = value.get("response_file")?.as_str()?;
    (!token.is_empty()).then(|| token.to_owned())
}

/// Mint a one-shot response-file correlation token under the active profile's
/// `rpc/` path namespace: `<config_dir>/rpc/<prefix>-<uuid>.<ext>`.
///
/// Does not create `<config_dir>/rpc/` on disk: socket RPCs never touch the
/// filesystem, and the unit-test file fallback (`atomic_write`) creates parent
/// directories on demand.
pub(crate) fn response_file(prefix: &str, ext: &str) -> String {
    response_file_in(&crate::config::config_dir(), prefix, ext)
}

/// `response_file` against an explicit profile dir — for callers that target
/// a channel other than the ambient one (`plexi host status --channel ...`).
pub(crate) fn response_file_in(profile_dir: &Path, prefix: &str, ext: &str) -> String {
    profile_dir
        .join("rpc")
        .join(format!("{prefix}-{}.{ext}", uuid::Uuid::new_v4()))
        .to_string_lossy()
        .into_owned()
}

#[derive(Debug)]
pub(crate) enum PollError {
    /// No response arrived within the timeout.
    TimedOut,
    /// The response stream or fallback file could not be read.
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

fn read_stream_to_end(
    mut stream: IpcStream,
    timeout: Option<Duration>,
) -> Result<Option<Vec<u8>>, PollError> {
    let _ = stream.set_read_timeout(timeout);
    let mut buf = Vec::new();
    match stream.read_to_end(&mut buf) {
        Ok(_) => {
            // Strip a leading PEER_IDENTITY_ACK byte (0x06) if the caller did
            // not consume it prior to polling.
            if buf.first() == Some(&0x06) && buf.len() > 1 {
                buf.remove(0);
            }
            if buf.is_empty() || buf.as_slice() == [0x06] {
                // Peer closed the socket without writing a stream reply (e.g.,
                // a unit-test mock listener that writes to the fallback file).
                Ok(None)
            } else {
                Ok(Some(buf))
            }
        }
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) =>
        {
            Err(PollError::TimedOut)
        }
        Err(e) => Err(PollError::Unreadable(e)),
    }
}

/// Wait for the host's reply on the registered `notify.sock` stream (or poll
/// the fallback `response_file` in unit tests). `None` timeout waits forever
/// (`plexi notify --timeout 0`).
pub(crate) fn poll_bytes(
    response_file: &str,
    timeout: Option<Duration>,
) -> Result<Vec<u8>, PollError> {
    let deadline = timeout.map(|t| Instant::now() + t);
    if let Some(stream) = take_client_stream(response_file) {
        match read_stream_to_end(stream, timeout)? {
            Some(bytes) if bytes.as_slice() == SLOT_EMPTY_SENTINEL => return Ok(Vec::new()),
            Some(bytes) => return Ok(bytes),
            None => {}
        }
    }
    let path = PathBuf::from(response_file);
    loop {
        if path.exists() {
            return take_response(&path);
        }
        if deadline.is_some_and(|dl| Instant::now() >= dl) {
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

/// A slot-read round trip answers with either raw payload bytes or a JSON
/// error (`<response_file>.err`).
pub(crate) enum SlotReply {
    Data(Vec<u8>),
    Err(Vec<u8>),
}

/// Read a slot reply from the registered `notify.sock` stream, falling back to
/// polling the data/error files in unit tests without a socket stream.
pub(crate) fn poll_slot_reply(
    response_file: &str,
    timeout: Option<Duration>,
) -> Result<SlotReply, PollError> {
    let deadline = timeout.map(|t| Instant::now() + t);
    if let Some(stream) = take_client_stream(response_file) {
        if let Some(bytes) = read_stream_to_end(stream, timeout)? {
            if let Some(err_bytes) = bytes.strip_prefix(SLOT_ERR_PREFIX) {
                return Ok(SlotReply::Err(err_bytes.to_vec()));
            }
            if bytes.as_slice() == SLOT_EMPTY_SENTINEL {
                return Ok(SlotReply::Data(Vec::new()));
            }
            return Ok(SlotReply::Data(bytes));
        }
    }
    let data_path = PathBuf::from(response_file);
    let err_path = PathBuf::from(format!("{response_file}.err"));
    loop {
        if err_path.exists() {
            let _ = std::fs::remove_file(&data_path);
            return take_response(&err_path).map(SlotReply::Err);
        }
        if data_path.exists() {
            let _ = std::fs::remove_file(&err_path);
            return take_response(&data_path).map(SlotReply::Data);
        }
        if deadline.is_some_and(|dl| Instant::now() >= dl) {
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

fn write_to_registered_stream(mut stream: IpcStream, wire_body: &[u8]) -> std::io::Result<()> {
    let payload = if wire_body.is_empty() {
        SLOT_EMPTY_SENTINEL
    } else {
        wire_body
    };
    stream.write_all(payload)?;
    stream.flush()?;
    #[cfg(unix)]
    let _ = stream.shutdown(std::net::Shutdown::Write);
    Ok(())
}

/// Host-side response write: sends `body` over the open `notify.sock` stream
/// registered in [`ReplyRegistry`] for `response_file` (or its base token when
/// `response_file` ends with `.err`), falling back to an atomic file write only
/// when no stream is registered (e.g. unit tests that inject requests without a
/// socket).
pub(crate) fn write_response(response_file: &str, body: &[u8]) -> bool {
    if let Some(stream) = ReplyRegistry::take(response_file) {
        return match write_to_registered_stream(stream, body) {
            Ok(()) => true,
            Err(e) => {
                log::warn!("rpc: stream write failed for {response_file:?}: {e}");
                false
            }
        };
    }
    if let Some(base) = response_file.strip_suffix(".err") {
        if let Some(stream) = ReplyRegistry::take(base) {
            let mut framed = Vec::with_capacity(SLOT_ERR_PREFIX.len() + body.len());
            framed.extend_from_slice(SLOT_ERR_PREFIX);
            framed.extend_from_slice(body);
            return match write_to_registered_stream(stream, &framed) {
                Ok(()) => true,
                Err(e) => {
                    log::warn!("rpc: error stream write failed for {response_file:?}: {e}");
                    false
                }
            };
        }
    }
    match crate::platform::fs::atomic_write(Path::new(response_file), body) {
        Ok(()) => true,
        Err(e) => {
            log::error!("rpc: could not write response file {response_file:?}: {e}");
            false
        }
    }
}

/// Alias for [`write_response`] for raw byte payloads.
#[allow(dead_code)]
pub(crate) fn write_raw_response(response_file: &str, body: &[u8]) -> bool {
    write_response(response_file, body)
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

/// Write a sensitive JSON response (`plexi secret get` value reply) strictly
/// over the registered socket stream in [`ReplyRegistry`], never touching the
/// filesystem even if no stream is registered.
pub(crate) fn write_stream_only_json_response(
    response_file: &str,
    value: serde_json::Value,
) -> bool {
    let Some(stream) = ReplyRegistry::take(response_file) else {
        log::error!(
            "rpc: refused to write stream-only response to filesystem for {response_file:?}"
        );
        return false;
    };
    let json = match serde_json::to_string(&value) {
        Ok(json) => json,
        Err(e) => {
            log::error!("rpc: could not serialize stream-only response for {response_file:?}: {e}");
            return false;
        }
    };
    match write_to_registered_stream(stream, json.as_bytes()) {
        Ok(()) => true,
        Err(e) => {
            log::warn!("rpc: stream-only write failed for {response_file:?}: {e}");
            false
        }
    }
}

/// Send one JSON RPC request over `socket_path` with a generated
/// `response_file` token and read the JSON reply directly from the same socket
/// stream without touching `<profile>/rpc/` or sleeping in a poll loop.
#[allow(dead_code)]
pub(crate) fn request_json(
    socket_path: &Path,
    payload: serde_json::Value,
    timeout: Duration,
) -> Result<serde_json::Value, PollError> {
    request_json_with_prefix(socket_path, payload, "rpc-request", timeout)
}

/// [`request_json`] with an explicit correlation-token prefix.
#[allow(dead_code)]
pub(crate) fn request_json_with_prefix(
    socket_path: &Path,
    mut payload: serde_json::Value,
    prefix: &str,
    timeout: Duration,
) -> Result<serde_json::Value, PollError> {
    let rf = response_file(prefix, "json");
    if let Some(obj) = payload.as_object_mut() {
        obj.insert(
            "response_file".to_string(),
            serde_json::Value::String(rf.clone()),
        );
    }
    #[cfg(unix)]
    let mut stream = IpcStream::connect(socket_path).map_err(PollError::Unreadable)?;
    #[cfg(windows)]
    let mut stream = crate::platform::ipc::connect_timeout(socket_path, timeout)
        .map_err(PollError::Unreadable)?;
    let line = format!("{payload}\n");
    stream
        .write_all(line.as_bytes())
        .and_then(|()| stream.flush())
        .map_err(PollError::Unreadable)?;
    register_client_stream(rf.clone(), stream);
    let body = poll_string(&rf, Some(timeout))?;
    serde_json::from_str(&body)
        .map_err(|e| PollError::Unreadable(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))
}

/// Startup sweep: delete `rpc/` entries older than `SWEEP_MAX_AGE`. Catches
/// legacy response files from older binaries. Best-effort; called once per host
/// boot.
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
        assert_eq!(std::fs::read(&path).expect("read back"), b"{\"ok\":true}");
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
    fn response_file_in_does_not_touch_filesystem() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = response_file_in(dir.path(), "pane-list-response", "json");
        assert!(
            !dir.path().join("rpc").exists(),
            "minting a correlation token must not create <profile>/rpc/ on disk"
        );
        assert!(path.contains("/rpc/pane-list-response-"));
        assert!(path.ends_with(".json"));
    }

    #[cfg(unix)]
    #[test]
    fn request_json_round_trips_over_unix_stream_with_zero_rpc_files() {
        use std::io::{BufRead, BufReader};

        let dir = tempfile::tempdir().expect("tempdir");
        let _profile = crate::config::set_test_profile_dir(dir.path().to_path_buf());
        let sock_path = dir.path().join("notify.sock");
        let listener = crate::platform::ipc::IpcListener::bind(&sock_path).expect("bind");

        let (tx, rx) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let mut h = crate::testing::HostHarness::new();
            let pane_id = h.add_focused_terminal();
            tx.send(pane_id).unwrap();
            let stream = listener.incoming().next().unwrap().expect("accept");
            let mut write_half = stream.try_clone().expect("clone");
            let mut line = String::new();
            BufReader::new(stream)
                .read_line(&mut line)
                .expect("read request");
            crate::app::handle_socket_line(&line, &h.ipc_tx, Some(&[]), &mut write_half);
            drop(write_half);
            h.hidden_frame();
        });

        let reply = request_json(
            &sock_path,
            serde_json::json!({ "type": "list_panes" }),
            Duration::from_secs(5),
        )
        .unwrap_or_else(|e| panic!("request_json failed: {e}"));
        let expected_pane = rx.recv().expect("pane_id");
        server.join().expect("server thread");

        let panes = reply.as_array().expect("list_panes array");
        assert!(
            panes.iter().any(|p| p["id"] == expected_pane),
            "list_panes reply over socket stream must include spawned pane {expected_pane}: {reply}"
        );
        assert!(
            !dir.path().join("rpc").exists(),
            "socket RPC must not create <profile>/rpc/ or any response files"
        );
    }

    #[cfg(unix)]
    #[test]
    fn parked_wait_holds_stream_across_logic_frames_until_event_fires() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _profile = crate::config::set_test_profile_dir(dir.path().to_path_buf());
        let mut h = crate::testing::HostHarness::new();
        let pane_id = h.add_focused_terminal();

        let wait_token = response_file_in(dir.path(), "slot-wait", "txt");
        let (mut server_stream, client_stream) =
            std::os::unix::net::UnixStream::pair().expect("pair");
        register_client_stream(wait_token.clone(), client_stream);

        let wait_req = crate::protocol::AppRequest::SlotWait {
            pane_id,
            slot_name: "status".to_string(),
            pattern: "^done:".to_string(),
            timeout_secs: 5.0,
            response_file: wait_token.clone(),
        };
        let line = serde_json::to_string(&wait_req).expect("serialize");
        crate::app::handle_socket_line(&line, &h.ipc_tx, Some(&[]), &mut server_stream);
        drop(server_stream);

        let token_for_client = wait_token.clone();
        let client = std::thread::spawn(move || {
            poll_slot_reply(&token_for_client, Some(Duration::from_secs(5)))
        });

        // Multiple App::logic frames run while the waiter is parked; stream stays open.
        for _ in 0..4 {
            h.hidden_frame();
            assert!(
                !client.is_finished(),
                "waiter must stay blocked before slot matches"
            );
        }

        // Now write a matching slot value; the parked waiter wakes and receives payload + EOF.
        let write_token = response_file_in(dir.path(), "slot-write", "json");
        let (mut write_srv, write_cli) = std::os::unix::net::UnixStream::pair().expect("pair");
        register_client_stream(write_token.clone(), write_cli);
        let write_req = crate::protocol::AppRequest::SlotWrite {
            pane_id,
            slot_name: "status".to_string(),
            content: b"done:ok".to_vec(),
            append: false,
            replace: true,
            response_file: write_token.clone(),
        };
        let write_line = serde_json::to_string(&write_req).expect("serialize");
        crate::app::handle_socket_line(&write_line, &h.ipc_tx, Some(&[]), &mut write_srv);
        drop(write_srv);
        h.hidden_frame();

        let write_ack: serde_json::Value = serde_json::from_str(
            &poll_string(&write_token, Some(Duration::from_secs(2))).expect("slot write ack"),
        )
        .expect("slot write json");
        assert_eq!(write_ack["ok"], true);

        let reply = client
            .join()
            .expect("client thread")
            .unwrap_or_else(|e| panic!("parked slot wait failed: {e}"));
        assert!(matches!(reply, SlotReply::Data(bytes) if bytes == b"done:ok"));
        assert!(
            !dir.path().join("rpc").exists(),
            "parked wait over socket stream must not create <profile>/rpc/ files"
        );
    }
}
