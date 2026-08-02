//! CLI surface for host-arbitrated named leases.

use std::time::Duration;

fn pane_id() -> Result<u64, i32> {
    match std::env::var("PLEXI_PANE_ID")
        .ok()
        .and_then(|id| id.parse().ok())
    {
        Some(id) => Ok(id),
        None => {
            eprintln!("error: PLEXI_PANE_ID is required — run lock commands from a Plexi pane");
            Err(1)
        }
    }
}

fn reply(response_file: &str, timeout: Duration) -> i32 {
    match crate::rpc::poll_slot_reply(response_file, Some(timeout)) {
        Ok(crate::rpc::SlotReply::Data(data)) => {
            let value: serde_json::Value = match serde_json::from_slice(&data) {
                Ok(value) => value,
                Err(error) => {
                    eprintln!("error: invalid lock response: {error}");
                    return 1;
                }
            };
            if value.get("ok").and_then(|ok| ok.as_bool()) == Some(true) {
                0
            } else {
                eprintln!("error: {value}");
                1
            }
        }
        Ok(crate::rpc::SlotReply::Err(data)) => {
            let value: serde_json::Value = serde_json::from_slice(&data)
                .unwrap_or_else(|_| serde_json::json!({"error": "invalid lock error"}));
            eprintln!(
                "error: {}",
                value
                    .get("error")
                    .and_then(|e| e.as_str())
                    .unwrap_or("lock request failed")
            );
            if value.get("timeout").and_then(|v| v.as_bool()) == Some(true) {
                2
            } else {
                1
            }
        }
        Err(error) => {
            eprintln!("error: lock host reply failed: {error}");
            1
        }
    }
}

pub fn acquire(name: &str, timeout_secs: f64) -> i32 {
    if !timeout_secs.is_finite()
        || !(0.0..=crate::app::pane_wait::MAX_WAIT_TIMEOUT_SECS).contains(&timeout_secs)
    {
        eprintln!("error: invalid --timeout {timeout_secs}");
        return 1;
    }
    let pane_id = match pane_id() {
        Ok(id) => id,
        Err(code) => return code,
    };
    let response_file = crate::rpc::response_file("lock-acquire", "json");
    if super::send_to_socket(
        serde_json::json!({"type":"lock_acquire","name":name,"pane_id":pane_id,"timeout_secs":timeout_secs,"response_file":response_file}),
    ) != 0
    {
        return 1;
    }
    reply(
        &response_file,
        Duration::from_secs_f64(timeout_secs) + Duration::from_secs(5),
    )
}

pub fn release(name: &str) -> i32 {
    let pane_id = match pane_id() {
        Ok(id) => id,
        Err(code) => return code,
    };
    let response_file = crate::rpc::response_file("lock-release", "json");
    if super::send_to_socket(
        serde_json::json!({"type":"lock_release","name":name,"pane_id":pane_id,"response_file":response_file}),
    ) != 0
    {
        return 1;
    }
    reply(&response_file, Duration::from_secs(10))
}

pub fn status(name: &str) -> i32 {
    let response_file = crate::rpc::response_file("lock-status", "json");
    if super::send_to_socket(
        serde_json::json!({"type":"lock_status","name":name,"response_file":response_file}),
    ) != 0
    {
        return 1;
    }
    match crate::rpc::poll_string(&response_file, Some(Duration::from_secs(10))) {
        Ok(value) => {
            println!("{value}");
            0
        }
        Err(crate::rpc::PollError::TimedOut) => {
            eprintln!("error: lock status timed out after the host accepted the request");
            2
        }
        Err(error) => {
            eprintln!("error: lock status failed: {error}");
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_host_timeout_exits_nonzero() {
        let dir = tempfile::tempdir().unwrap();
        let response = dir.path().join("lock.json");
        crate::rpc::write_json_response(
            &format!("{}.err", response.display()),
            serde_json::json!({"ok": false, "timeout": true, "error": "timed out"}),
        );
        assert_eq!(
            reply(&response.to_string_lossy(), Duration::from_secs(1)),
            2
        );
    }
}
