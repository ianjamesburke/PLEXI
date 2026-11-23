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
        let _ = stream.read_to_end(&mut rest);
        ack.extend(rest);
        return Err(format!(
            "the host refused the spawn receipt: {}",
            String::from_utf8_lossy(&ack)
        ));
    }
    Ok(stream)
}

/// `None` means the host claimed the request but has not returned a pane id.
/// Claiming is enough to prevent a retry; it is not proof of a completed launch.
pub(super) fn wait(
    queue_file: &Path,
    receipt: &mut IpcStream,
    timeout: Duration,
) -> Result<Option<String>, String> {
    let mut reply = Vec::new();
    if let Some(reply) = read_reply(receipt, &mut reply, timeout)? {
        return Ok(Some(reply));
    }
    reconcile_timeout(queue_file, receipt, reply)
}

/// The host's whole reply, or `None` if it has not finished by `timeout`.
fn read_reply(
    receipt: &mut IpcStream,
    reply: &mut Vec<u8>,
    timeout: Duration,
) -> Result<Option<String>, String> {
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
            Err(error) => return Err(format!("could not set the spawn reply deadline: {error}")),
        }
        match receipt.read(&mut chunk) {
            Ok(0) if reply.is_empty() => {
                return Err("the host closed the connection before answering the queued spawn; check `pane list` before retrying".to_string());
            }
            Ok(0) => return Ok(Some(String::from_utf8_lossy(reply).into_owned())),
            Ok(read) => reply.extend_from_slice(&chunk[..read]),
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                if Instant::now() >= deadline {
                    return Ok(None);
                }
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => return Err(format!("could not read queued spawn response: {error}")),
        }
    }
}

fn reconcile_timeout(
    queue_file: &Path,
    receipt: &mut IpcStream,
    mut reply: Vec<u8>,
) -> Result<Option<String>, String> {
    // Both host and client must win this unlink before claiming/cancelling.
    // The host reads first, but cannot launch if this client removes the file.
    match std::fs::remove_file(queue_file) {
        Ok(()) => {
            log::info!("queued spawn: cancelled unclaimed request {queue_file:?}");
            Err(timeout_message())
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            log::info!("queued spawn: host claimed request before timeout {queue_file:?}");
            read_reply(receipt, &mut reply, Duration::from_millis(50))
        }
        Err(error) => {
            // Cancellation is not proven. Do not report a failed launch that
            // callers might retry while the host can still consume this file.
            log::warn!("queued spawn: cancellation unavailable for {queue_file:?}: {error}");
            eprintln!("queued pane is still pending; cancellation failed: {error}. Do not retry; check host status.");
            Ok(None)
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
            reconcile_timeout(&queue, &mut receipt, Vec::new()).unwrap(),
            Some(r#"{"pane_id":42}"#.into())
        );
    }

    #[test]
    fn queued_spawn_timeout_cancels_unconsumed_request() {
        let dir = tempfile::tempdir().unwrap();
        let _profile = crate::config::set_test_profile_dir(dir.path().to_path_buf());
        let queue = dir.path().join("request.json");
        std::fs::write(&queue, "{}").unwrap();
        let (mut receipt, _host) = UnixStream::pair().unwrap();
        let error = wait(&queue, &mut receipt, Duration::ZERO).unwrap_err();
        assert!(error.contains("host did not pick up the queued pane within 10s"));
        assert!(error.contains("host status"));
        assert!(!queue.exists());
        // Even a host that read before cancellation cannot win the claim.
        assert_eq!(
            std::fs::remove_file(queue).unwrap_err().kind(),
            ErrorKind::NotFound
        );
    }

    #[test]
    fn queued_spawn_consumed_request_does_not_report_failure() {
        let dir = tempfile::tempdir().unwrap();
        let (mut receipt, _host) = UnixStream::pair().unwrap();
        assert_eq!(
            wait(&dir.path().join("consumed.json"), &mut receipt, Duration::ZERO).unwrap(),
            None
        );
    }

    #[test]
    fn queued_spawn_host_error_is_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let (mut receipt, mut host) = UnixStream::pair().unwrap();
        host.write_all(br#"{"error":"launch rejected"}"#).unwrap();
        drop(host);
        let content = wait(&dir.path().join("consumed.json"), &mut receipt, Duration::from_secs(1))
            .unwrap()
            .unwrap();
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
}
