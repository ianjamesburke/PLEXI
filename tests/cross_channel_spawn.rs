#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn cross_channel_app_open_drops_foreign_pane_and_sends_caller_cwd() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let channel_dir = home.join(".plexi-pr-1195");
    std::fs::create_dir(&channel_dir).unwrap();
    let socket = channel_dir.join("notify.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let binary = home.join("plexi-pr-1195");
    std::fs::copy(env!("CARGO_BIN_EXE_plexi"), &binary).unwrap();
    let cwd = home.join("caller-workspace");
    std::fs::create_dir(&cwd).unwrap();
    let mut command = Command::new(binary);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("PLEXI_") {
            command.env_remove(key);
        }
    }
    let mut child = command
        .args(["app", "open", "balls"])
        .current_dir(&cwd)
        .env("HOME", home)
        .env("PLEXI_SOCKET", home.join(".plexi-beta/notify.sock"))
        .env("PLEXI_PANE_ID", "229")
        .env("PLEXI_CONTEXT_ID", "4")
        .env("PLEXI_RUNNING", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let request = loop {
        let stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                if let Some(status) = child.try_wait().unwrap() {
                    panic!("CLI exited before request: {status}");
                }
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            Err(e) => {
                child.kill().ok();
                panic!("accept failed: {e}");
            }
        };
        let mut line = String::new();
        let mut reader = BufReader::new(stream);
        reader.read_line(&mut line).unwrap();
        let request: serde_json::Value = serde_json::from_str(&line).unwrap();
        if request["type"] != "spawn_pane" {
            continue;
        }
        let mut stream = reader.into_inner();
        stream.write_all(br#"{"pane_id":123}"#).unwrap();
        stream.shutdown(std::net::Shutdown::Write).unwrap();
        break request;
    };
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        request.get("from_pane_id").is_none(),
        "foreign identity leaked: {request}"
    );
    assert_eq!(request["cwd"], cwd.to_str().unwrap());
}
