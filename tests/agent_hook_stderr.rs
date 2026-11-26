#![cfg(unix)]

mod common;
#[path = "../src/testing/timeout.rs"]
mod timeouts;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const HOOK: &[u8] = br#"{"hook_event_name":"Stop","session_id":"test-session"}"#;

fn isolated_home() -> tempfile::TempDir {
    // Keep the Unix socket path short and profile initialization offline.
    let home = tempfile::Builder::new()
        .prefix("px1812")
        .tempdir_in("/tmp")
        .unwrap();
    let profile = home.path().join(".plexi");
    std::fs::create_dir(&profile).unwrap();
    std::fs::write(
        profile.join("config.toml"),
        "config_version = 1\ncore_only = true\n",
    )
    .unwrap();
    home
}

fn agent_command(home: &Path, verb: &str) -> Command {
    let socket = home.join("host.sock");
    let mut command = Command::new(common::plexi_binary());
    command
        .env_clear()
        .env("HOME", home)
        .env("PATH", "/usr/bin:/bin")
        // Model an inherited endpoint. An explicit --socket to an absent
        // path strips pane identity before dispatch.
        .env("PLEXI_SOCKET", socket)
        .env("PLEXI_PANE_ID", "42")
        .current_dir(home)
        .args(["agent", verb, "--agent", "claude-code"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    command
}

fn run(command: &mut Command, input: &[u8]) -> Output {
    let mut child = command.spawn().unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let deadline = Instant::now() + timeouts::load_aware_timeout(Duration::from_secs(30));
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            panic!(
                "agent command hung: {:?}",
                child.wait_with_output().unwrap()
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    child.wait_with_output().unwrap()
}

fn closed_stderr(command: &mut Command, force_epipe: bool) {
    #[cfg(target_os = "macos")]
    use std::os::fd::AsRawFd;
    #[cfg(not(target_os = "macos"))]
    use std::os::unix::process::CommandExt;

    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    if force_epipe {
        #[cfg(target_os = "macos")]
        {
            // Darwin sys/fcntl.h. The locked libc does not export this yet.
            const F_SETNOSIGPIPE: libc::c_int = 73;
            // SAFETY: writer owns this live pipe descriptor.
            assert_eq!(
                unsafe { libc::fcntl(writer.as_raw_fd(), F_SETNOSIGPIPE, 1) },
                0
            );
        }
        #[cfg(not(target_os = "macos"))]
        // SAFETY: the callback uses only async-signal-safe syscalls and
        // blocks SIGPIPE before exec, even if the binary restores SIG_DFL.
        unsafe {
            command.pre_exec(|| {
                let mut set = std::mem::zeroed();
                libc::sigemptyset(&mut set);
                libc::sigaddset(&mut set, libc::SIGPIPE);
                if libc::sigprocmask(libc::SIG_BLOCK, &set, std::ptr::null_mut()) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    command.stderr(writer);
}

fn channel_log(home: &Path) -> String {
    std::fs::read_to_string(home.join(".plexi/plexi.log")).expect("isolated profile log")
}

#[test]
fn cli_agent_hook_survives_closed_stderr() {
    for force_epipe in [true, false] {
        let home = isolated_home();
        let mut command = agent_command(home.path(), "hook-event");
        closed_stderr(&mut command, force_epipe);
        let output = run(&mut command, HOOK);
        let log = channel_log(home.path());
        assert_eq!(output.status.code(), Some(0), "{output:?}\n{log}");
        assert!(log.contains("socket connect failed"), "{log}");
        assert!(!log.contains("PANIC:"), "{log}");
    }
}

#[test]
fn cli_agent_report_preserves_connection_failure() {
    for closed in [None, Some(true), Some(false)] {
        let home = isolated_home();
        let mut command = agent_command(home.path(), "report");
        command.args(["--state", "idle"]);
        if let Some(force_epipe) = closed {
            closed_stderr(&mut command, force_epipe);
        }
        let output = run(&mut command, b"");
        let log = channel_log(home.path());
        assert_eq!(output.status.code(), Some(1), "{output:?}\n{log}");
        if closed.is_none() {
            let diagnostic = String::from_utf8_lossy(&output.stderr);
            assert!(
                diagnostic.contains("could not connect to PLEXI_SOCKET"),
                "{diagnostic}"
            );
        }
        assert!(log.contains("socket connect failed"), "{log}");
        assert!(!log.contains("PANIC:"), "{log}");
    }
}

#[test]
fn cli_agent_hook_delivers_state_report() {
    let home = isolated_home();
    let listener = UnixListener::bind(home.path().join("host.sock")).unwrap();
    listener.set_nonblocking(true).unwrap();
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + timeouts::load_aware_timeout(Duration::from_secs(30));
        let accept = || loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("accept state report: {error}"),
            }
        };
        // The CLI brackets the state report with two invocation-trace frames.
        [(); 3].map(|()| {
            let mut stream = accept();
            let timeout = timeouts::load_aware_timeout(Duration::from_secs(5));
            stream.set_read_timeout(Some(timeout)).unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            assert!(line.ends_with('\n'), "{line:?}");
            stream.write_all(&[0x06]).unwrap();
            serde_json::from_str::<serde_json::Value>(&line).unwrap()
        })
    });
    let output = run(&mut agent_command(home.path(), "hook-event"), HOOK);
    let [start, payload, exit] = server.join().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(start["type"], "cli_trace");
    assert_eq!(start["phase"], "start");
    assert_eq!(exit["type"], "cli_trace");
    assert_eq!(exit["phase"], "exit");
    assert_eq!(exit["exit_code"], 0);
    assert_eq!(
        payload,
        serde_json::json!({
            "type": "set_agent_state", "pane_id": 42, "state": "idle",
            "agent": "claude-code", "session_id": "test-session", "event": "Stop",
        })
    );
    assert!(!channel_log(home.path()).contains("peer identity ack not received"));
}
