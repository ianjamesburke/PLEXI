//! `pane state|status|wait|heartbeat` (and the older defaulting commands) act on
//! the calling pane when no id is given: explicit id, else `PLEXI_PANE_ID`,
//! else exit 1 with one shared message. Stint 1227.
#![cfg(unix)]

use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixListener;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

fn plexi(home: &std::path::Path, args: &[&str], pane_env: Option<&str>) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_plexi"));
    // Remove (not blank) every PLEXI_* var so "unset" is truly unset.
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("PLEXI_") {
            command.env_remove(key);
        }
    }
    command.args(args).env("HOME", home).stdin(Stdio::null());
    if let Some(value) = pane_env {
        command.env("PLEXI_PANE_ID", value);
    }
    command
}

/// Run the CLI against a fake host and return the first request it sends.
fn first_request(args: &[&str], pane_env: Option<&str>) -> serde_json::Value {
    let home = tempfile::tempdir().unwrap();
    let socket = home.path().join("notify.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut child = plexi(home.path(), args, pane_env)
        .env("PLEXI_SOCKET", &socket)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).unwrap();
                let mut line = String::new();
                BufReader::new(stream).read_line(&mut line).unwrap();
                let request: serde_json::Value = serde_json::from_str(&line).expect("request json");
                // The CLI also reports a `cli_trace` record; skip it.
                if request["type"] == "cli_trace" {
                    continue;
                }
                child.kill().ok();
                child.wait().ok();
                return request;
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                if let Some(status) = child.try_wait().unwrap() {
                    panic!("CLI exited before sending a request: {status}");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => {
                child.kill().ok();
                panic!("accept failed: {e}");
            }
        }
    }
}

fn run_failing(args: &[&str], pane_env: Option<&str>) -> Output {
    let home = tempfile::tempdir().unwrap();
    plexi(home.path(), args, pane_env)
        .output()
        .expect("run plexi")
}

const SELF_COMMANDS: &[(&[&str], &str)] = &[
    (&["pane", "state"], "get_pane_state"),
    (&["pane", "status"], "pane_status"),
    (&["pane", "wait", "--until", "idle"], "pane_lifecycle_wait"),
    (&["pane", "heartbeat", "--off"], "pane_heartbeat"),
];

#[test]
fn current_pane_self_commands_target_plexi_pane_id_when_no_id_given() {
    for (args, request_type) in SELF_COMMANDS {
        let request = first_request(args, Some("77"));
        assert_eq!(request["type"], *request_type, "{args:?}");
        assert_eq!(request["pane_id"], 77, "{args:?}: {request}");
    }
}

#[test]
fn current_pane_explicit_id_wins_over_plexi_pane_id() {
    let request = first_request(&["pane", "state", "5"], Some("77"));
    assert_eq!(request["type"], "get_pane_state");
    assert_eq!(request["pane_id"], 5);
}

#[test]
fn current_pane_unset_variable_fails_with_shared_message() {
    let mut commands: Vec<Vec<&str>> = SELF_COMMANDS.iter().map(|(a, _)| a.to_vec()).collect();
    commands.extend([
        vec!["pane", "capture"],
        vec!["pane", "close"],
        vec!["pane", "zoom"],
    ]);
    for args in commands {
        let output = run_failing(&args, None);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "{args:?}: {stderr}");
        assert!(
            stderr.contains("no pane id given and PLEXI_PANE_ID is not set"),
            "{args:?}: {stderr}"
        );
        assert!(stderr.contains("plexi pane list"), "{args:?}: {stderr}");
    }
}

#[test]
fn current_pane_invalid_variable_fails_naming_variable_and_value() {
    for raw in ["", "abc"] {
        let output = run_failing(&["pane", "state"], Some(raw));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "{raw:?}: {stderr}");
        assert!(
            stderr.contains("PLEXI_PANE_ID is not a valid pane id"),
            "{stderr}"
        );
        assert!(stderr.contains(&format!("{raw:?}")), "{stderr}");
    }
}
