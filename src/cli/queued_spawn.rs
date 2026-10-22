//! Wait for a queued spawn without leaving a failed request for later execution.
use std::path::Path;
use std::time::{Duration, Instant};

/// `None` means the host claimed the request but has not returned a pane id.
/// Claiming is enough to prevent a retry; it is not proof of a completed launch.
pub(super) fn wait(
    queue_file: &Path,
    response_file: &Path,
    timeout: Duration,
) -> Result<Option<String>, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(reply) = take_reply(response_file)? {
            return Ok(Some(reply));
        }
        if Instant::now() >= deadline {
            return reconcile_timeout(queue_file, response_file);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn take_reply(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(reply) => {
            if let Err(error) = std::fs::remove_file(path) {
                log::warn!("queued spawn: could not clean response {path:?}: {error}");
            }
            Ok(Some(reply))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("could not read queued spawn response: {error}")),
    }
}

fn reconcile_timeout(queue_file: &Path, response_file: &Path) -> Result<Option<String>, String> {
    // Recheck after the deadline. Unlike generic RPC polling, never delete a
    // late reply without reading it.
    if let Some(reply) = take_reply(response_file)? {
        return Ok(Some(reply));
    }
    // Both host and client must win this unlink before claiming/cancelling.
    // The host reads first, but cannot launch if this client removes the file.
    match std::fs::remove_file(queue_file) {
        Ok(()) => {
            log::info!("queued spawn: cancelled unclaimed request {queue_file:?}");
            Err(timeout_message())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            log::info!("queued spawn: host claimed request before timeout {queue_file:?}");
            take_reply(response_file)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_spawn_late_response_at_timeout_reports_success() {
        let dir = tempfile::tempdir().unwrap();
        let queue = dir.path().join("request.json");
        let reply = dir.path().join("reply.json");
        // The host consumed the request and answered after the final poll.
        std::fs::write(&reply, r#"{"pane_id":42}"#).unwrap();
        assert_eq!(
            reconcile_timeout(&queue, &reply).unwrap(),
            Some(r#"{"pane_id":42}"#.into())
        );
        assert!(!reply.exists());
    }

    #[test]
    fn queued_spawn_timeout_cancels_unconsumed_request() {
        let dir = tempfile::tempdir().unwrap();
        let _profile = crate::config::set_test_profile_dir(dir.path().to_path_buf());
        let queue = dir.path().join("request.json");
        std::fs::write(&queue, "{}").unwrap();
        let error = wait(&queue, &dir.path().join("reply.json"), Duration::ZERO).unwrap_err();
        assert!(error.contains("host did not pick up the queued pane within 10s"));
        assert!(error.contains("host status"));
        assert!(!queue.exists());
        // Even a host that read before cancellation cannot win the claim.
        assert_eq!(
            std::fs::remove_file(queue).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
    }

    #[test]
    fn queued_spawn_consumed_request_does_not_report_failure() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            wait(
                &dir.path().join("consumed.json"),
                &dir.path().join("reply.json"),
                Duration::ZERO
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn queued_spawn_host_error_is_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let reply = dir.path().join("reply.json");
        std::fs::write(&reply, r#"{"error":"launch rejected"}"#).unwrap();
        let content = wait(&dir.path().join("consumed.json"), &reply, Duration::ZERO)
            .unwrap()
            .unwrap();
        assert_eq!(
            super::super::queued_spawn_pane_id(&content),
            Err("launch rejected".into())
        );
    }
}
