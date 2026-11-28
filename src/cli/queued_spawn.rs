//! Wait for a queued spawn without leaving a failed request for later execution.
use crate::platform::ipc::IpcStream;
use std::io::{ErrorKind, Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};

/// How long the host may take to confirm it holds the receipt connection.
const RECEIPT_ACK_DEADLINE: Duration = Duration::from_secs(5);

/// Open the connection the host answers this queued spawn on, and return once
/// the host holds it. The reply never touches the filesystem (stint 1285).
pub(super) fn open_receipt(token: &str) -> Result<IpcStream, String> {
    let endpoint = crate::platform::ipc::endpoint_path();
    let mut stream = IpcStream::connect(&endpoint)
        .map_err(|error| format!("could not reach the Plexi host at {endpoint:?}: {error}"))?;
    let request = serde_json::json!({"type": "spawn_receipt", "response_file": token});
    stream
        .write_all(format!("{request}\n").as_bytes())
        .map_err(|error| format!("could not send the spawn receipt request: {error}"))?;
    stream
        .set_read_timeout(Some(RECEIPT_ACK_DEADLINE))
        .map_err(|error| format!("could not set the receipt deadline: {error}"))?;
    let mut ack = vec![0; crate::app::PEER_IDENTITY_ACK.len()];
    stream
        .read_exact(&mut ack)
        .map_err(|error| format!("the host did not confirm the spawn receipt: {error}"))?;
    if ack != crate::app::PEER_IDENTITY_ACK {
        let mut rest = Vec::new();
        stream.read_to_end(&mut rest)
            .map_err(|error| format!("could not read the spawn receipt refusal: {error}"))?;
        ack.extend(rest);
        let body = String::from_utf8_lossy(&ack).into_owned();
        let reason = match super::reply::HostReply::decode(Ok(body)) {
            super::reply::HostReply::Refused(refusal) => refusal.coded_reason(),
            super::reply::HostReply::Payload(text) => text,
            super::reply::HostReply::Transport(error) => error.to_string(),
        };
        return Err(format!("the host refused the spawn receipt: {reason}"));
    }
    Ok(stream)
}

/// How a queued outside spawn ended, as alpha reported it (Codex 3290 r4).
#[derive(Debug, PartialEq, Eq)]
pub(super) enum SpawnOutcome {
    /// The host answered: its reply (a pane id or the launch error).
    Answered(String),
    /// The request was removed before any host took it: nothing launches.
    Cancelled(String),
    /// A host took the request but no answer arrived, or its cancellation
    /// could not be proved. It may have launched; do not retry blindly.
    Pending,
}

/// What reading the receipt produced.
enum ReceiptRead {
    Reply(String),
    TimedOut,
    Closed,
    Failed(String),
}

/// Wait for the host's answer. Every way the wait ends without one (the
/// deadline, the host closing the receipt, a read error) goes through the
/// one reconciliation of the queue file.
pub(super) fn wait(queue_file: &Path, receipt: &mut IpcStream, timeout: Duration) -> SpawnOutcome {
    let mut reply = Vec::new();
    let read = read_reply(receipt, &mut reply, timeout);
    let why = match read {
        ReceiptRead::Reply(reply) => return SpawnOutcome::Answered(reply),
        ReceiptRead::TimedOut => timeout_message(),
        ReceiptRead::Closed => "the host closed the connection before answering the queued spawn".to_string(),
        ReceiptRead::Failed(error) => format!("could not read queued spawn response: {error}"),
    };
    reconcile(queue_file, receipt, reply, why)
}

/// Both host and client must win the unlink of the queue file before
/// claiming or cancelling it. Removed here: cancelled, proved. Already gone:
/// a host took it, so a late answer is read, else it is pending. Not
/// removable: cancellation is not proved, so it is pending too.
fn reconcile(queue_file: &Path, receipt: &mut IpcStream, mut reply: Vec<u8>, why: String) -> SpawnOutcome {
    match std::fs::remove_file(queue_file) {
        Ok(()) => {
            log::info!("queued spawn: cancelled unclaimed request {queue_file:?}: {why}");
            SpawnOutcome::Cancelled(format!("{why}; the request was cancelled and nothing was launched"))
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            log::info!("queued spawn: host claimed request {queue_file:?}; {why}");
            match read_reply(receipt, &mut reply, Duration::from_millis(50)) {
                ReceiptRead::Reply(reply) => SpawnOutcome::Answered(reply),
                ReceiptRead::TimedOut | ReceiptRead::Closed | ReceiptRead::Failed(_) => SpawnOutcome::Pending,
            }
        }
        Err(error) => {
            log::warn!("queued spawn: cancellation unavailable for {queue_file:?}: {error}");
            eprintln!("queued pane is still pending; cancellation failed: {error}. Do not retry; check host status.");
            SpawnOutcome::Pending
        }
    }
}

/// The host's whole reply, or why none arrived.
fn read_reply(receipt: &mut IpcStream, reply: &mut Vec<u8>, timeout: Duration) -> ReceiptRead {
    let deadline = Instant::now() + timeout;
    let mut chunk = [0; 4096];
    loop {
        // A zero read timeout is an error, so the last poll waits a moment.
        let remaining = deadline
            .saturating_duration_since(Instant::now())
            .max(Duration::from_millis(1));
        // macOS answers EINVAL once the host has closed its end. The read
        // below then returns the buffered reply or EOF without blocking.
        match receipt.set_read_timeout(Some(remaining)) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::InvalidInput => {}
            Err(error) => return ReceiptRead::Failed(format!("could not set the spawn reply deadline: {error}")),
        }
        match receipt.read(&mut chunk) {
            Ok(0) if reply.is_empty() => return ReceiptRead::Closed,
            Ok(0) => return ReceiptRead::Reply(String::from_utf8_lossy(reply).into_owned()),
            Ok(read) => reply.extend_from_slice(&chunk[..read]),
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                if Instant::now() >= deadline {
                    return ReceiptRead::TimedOut;
                }
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => return ReceiptRead::Failed(error.to_string()),
        }
    }
}

fn timeout_message() -> String {
    let profile = crate::config::config_dir();
    let channel = super::channel_label_from(
        profile
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(".plexi"),
    );
    let bin = if channel == "default" {
        "plexi".to_string()
    } else {
        format!("plexi-{channel}")
    };
    format!("host did not pick up the queued pane within 10s (is the Plexi {channel} window running? {bin} host status)")
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;

    #[test]
    fn queued_spawn_late_response_at_timeout_reports_success() {
        let dir = tempfile::tempdir().unwrap();
        let queue = dir.path().join("request.json");
        let (mut receipt, mut host) = UnixStream::pair().unwrap();
        // The host consumed the request and answered after the final poll.
        host.write_all(br#"{"pane_id":42}"#).unwrap();
        drop(host);
        assert_eq!(
            reconcile(&queue, &mut receipt, Vec::new(), "timed out".into()),
            SpawnOutcome::Answered(r#"{"pane_id":42}"#.into())
        );
    }

    #[test]
    fn queued_spawn_timeout_cancels_unconsumed_request() {
        let dir = tempfile::tempdir().unwrap();
        let _profile = crate::config::set_test_profile_dir(dir.path().to_path_buf());
        let queue = dir.path().join("request.json");
        std::fs::write(&queue, "{}").unwrap();
        let (mut receipt, _host) = UnixStream::pair().unwrap();
        let SpawnOutcome::Cancelled(error) = wait(&queue, &mut receipt, Duration::ZERO) else {
            panic!("expected a cancelled request");
        };
        assert!(error.contains("host did not pick up the queued pane within 10s"));
        assert!(error.contains("host status"));
        assert!(!queue.exists());
        // Even a host that read before cancellation cannot win the claim.
        assert_eq!(
            std::fs::remove_file(queue).unwrap_err().kind(),
            ErrorKind::NotFound
        );
    }

    /// Codex 3290 r3 B3: a host that closes the receipt without answering
    /// leaves no unclaimed request behind for a later host to launch.
    #[test]
    fn a_receipt_closed_unanswered_cancels_the_unclaimed_request() {
        let dir = tempfile::tempdir().unwrap();
        let queue = dir.path().join("request.json");
        std::fs::write(&queue, "{}").unwrap();
        let (mut receipt, host) = UnixStream::pair().unwrap();
        drop(host);
        let SpawnOutcome::Cancelled(error) = wait(&queue, &mut receipt, Duration::from_secs(1)) else {
            panic!("expected a cancelled request");
        };
        assert!(error.contains("nothing was launched"), "{error}");
        assert!(!queue.exists(), "the unclaimed request survived the closed receipt");
    }

    /// Codex 3290 r4 S3: a host that took the request and closed the receipt
    /// before answering may have launched it: pending, as alpha said, never
    /// a failure an automated caller would retry.
    #[test]
    fn a_claimed_request_closed_unanswered_is_pending_not_failed() {
        let dir = tempfile::tempdir().unwrap();
        let (mut receipt, host) = UnixStream::pair().unwrap();
        drop(host);
        assert_eq!(
            wait(&dir.path().join("claimed.json"), &mut receipt, Duration::from_secs(1)),
            SpawnOutcome::Pending
        );
    }

    /// Codex 3290 r4 S3: when the request cannot be removed, its cancellation
    /// is not proved; it is pending and the file is left for no retry.
    #[test]
    fn an_unremovable_request_is_pending_not_failed() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let locked = dir.path().join("locked");
        std::fs::create_dir(&locked).unwrap();
        let queue = locked.join("request.json");
        std::fs::write(&queue, "{}").unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o500)).unwrap();
        let (mut receipt, host) = UnixStream::pair().unwrap();
        drop(host);
        let outcome = wait(&queue, &mut receipt, Duration::from_secs(1));
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(outcome, SpawnOutcome::Pending);
    }

    #[test]
    fn queued_spawn_consumed_request_does_not_report_failure() {
        let dir = tempfile::tempdir().unwrap();
        let (mut receipt, _host) = UnixStream::pair().unwrap();
        assert_eq!(
            wait(&dir.path().join("consumed.json"), &mut receipt, Duration::ZERO),
            SpawnOutcome::Pending
        );
    }

    #[test]
    fn queued_spawn_host_error_is_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let (mut receipt, mut host) = UnixStream::pair().unwrap();
        host.write_all(br#"{"error":"launch rejected"}"#).unwrap();
        drop(host);
        let SpawnOutcome::Answered(content) =
            wait(&dir.path().join("consumed.json"), &mut receipt, Duration::from_secs(1))
        else {
            panic!("expected the host's answer");
        };
        assert_eq!(
            super::super::queued_spawn_pane_id(&content),
            Err("launch rejected".into())
        );
    }

    #[test]
    fn a_queued_spawn_answers_on_the_receipt_and_writes_no_reply_file() {
        let mut h = crate::testing::HostHarness::new();
        h.add_focused_terminal();
        let token = crate::rpc::response_file("queued-spawn-response", "json");
        let (mut receipt, server) = UnixStream::pair().unwrap();
        let line = serde_json::json!({"type": "spawn_receipt", "response_file": token}).to_string();
        let mut server = server;
        crate::app::handle_socket_line_in(
            &crate::config::config_dir(),
            &line,
            &h.ipc_tx,
            Some(&crate::broker::caller::PeerWitness::capture(std::process::id())),
            &mut server,
        );
        let mut ack = vec![0; crate::app::PEER_IDENTITY_ACK.len()];
        receipt.read_exact(&mut ack).unwrap();
        assert_eq!(ack, crate::app::PEER_IDENTITY_ACK);
        h.hidden_frame();
        assert!(crate::rpc::write_json_response(&token, serde_json::json!({"pane_id": 7})));
        drop(server);
        let mut body = String::new();
        receipt.read_to_string(&mut body).unwrap();
        assert!(body.contains("\"pane_id\":7"), "{body}");
        assert!(!Path::new(&token).exists(), "no reply file");
    }

    /// Codex 3290 r3 B3: a CLI that opened its receipt and then failed to
    /// queue the request goes away; the host releases the receipt stream at
    /// its next queue drain instead of holding it until pruning.
    #[test]
    fn a_receipt_whose_client_left_is_released_at_the_next_drain() {
        let mut h = crate::testing::HostHarness::new();
        let token = crate::rpc::response_file("queued-spawn-response", "json");
        let (mut receipt, mut server) = UnixStream::pair().unwrap();
        let line = serde_json::json!({"type": "spawn_receipt", "response_file": token}).to_string();
        crate::app::handle_socket_line_in(
            &crate::config::config_dir(),
            &line,
            &h.ipc_tx,
            Some(&crate::broker::caller::PeerWitness::capture(std::process::id())),
            &mut server,
        );
        let mut ack = vec![0; crate::app::PEER_IDENTITY_ACK.len()];
        receipt.read_exact(&mut ack).unwrap();
        h.hidden_frame();
        assert_eq!(crate::rpc::ReplyRegistry::connected(&token), Some(true));
        // The queue write failed; the CLI exits.
        drop(receipt);
        drop(server);
        h.inject_ipc(plexi_protocol::HostRequest::Wake);
        h.hidden_frame();
        assert_eq!(
            crate::rpc::ReplyRegistry::connected(&token),
            None,
            "the host still holds a receipt nobody will read"
        );
    }
}
