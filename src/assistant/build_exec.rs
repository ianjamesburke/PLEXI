//! Embedded, non-visible execution surface for the Assistant's app-build
//! commands (stint 0421).
//!
//! The Assistant must never drive a user-visible PTY for its own work.
//! `host.build.run` executes an allowlisted set of `plexi app` subcommands
//! (`init`, `check`) as a hidden subprocess of the running host binary,
//! captures both streams, and returns them directly to the model — one tool
//! call per command instead of a `host.terminals.run`/`host.terminals.read`
//! pair against a human-observed terminal. `host.terminals.*` remains only
//! for genuinely user-facing terminal work.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Default and maximum wall-clock budget for one build command.
pub(crate) const DEFAULT_BUILD_TIMEOUT_MS: u64 = 120_000;
pub(crate) const MAX_BUILD_TIMEOUT_MS: u64 = 300_000;

/// Per-stream capture cap. Longer output keeps the head and tail with an
/// elision marker, the shape a model can still reason about.
const MAX_STREAM_CHARS: usize = 16_384;

/// How long to wait for the stream readers after the child is reaped. A
/// grandchild that inherited the pipes (e.g. a python validator spawned by
/// `plexi app check`) can hold them open past the child's death; the drain
/// bound guarantees `run_build_command` always returns so the broker worker
/// blocked on the tool reply can never wedge.
const STREAM_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

/// Result of one embedded build command.
#[derive(Debug)]
pub(crate) struct BuildCommandOutput {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub duration_ms: u64,
}

/// Gate the embedded surface to the sanctioned app-authoring commands.
/// Everything else — including pane-opening flags — must go through the
/// dedicated host tools, so the scope of a `host.build.run` grant stays
/// exactly "app scaffolding and validation".
pub(crate) fn validate_build_args(args: &[String]) -> Result<(), String> {
    let (Some(first), Some(second)) = (args.first(), args.get(1)) else {
        return Err(
            "invalid_input: args must start with [\"app\", \"init\"|\"check\"]".to_string(),
        );
    };
    if first != "app" || !matches!(second.as_str(), "init" | "check") {
        return Err(format!(
            "command_not_allowed: only `plexi app init` and `plexi app check` run here, got `plexi {}`",
            args.join(" ")
        ));
    }
    if args.iter().any(|arg| arg == "--open") {
        return Err(
            "command_not_allowed: --open is rejected; open the app with host.apps.open instead"
                .to_string(),
        );
    }
    Ok(())
}

/// Run `exe args…` from `cwd` with no visible surface: stdin closed, both
/// output streams captured on reader threads (so a chatty command can never
/// deadlock the pipe), and a hard kill once `timeout` elapses. On Unix the
/// child gets its own process group so the timeout kill reaches grandchildren
/// (uv/python spawned by `plexi app check`) — killing only the direct child
/// would leave them running and holding the pipes open.
pub(crate) fn run_build_command(
    exe: &Path,
    args: &[String],
    cwd: &Path,
    timeout: Duration,
) -> Result<BuildCommandOutput, String> {
    let started = Instant::now();
    let mut command = Command::new(exe);
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().map_err(|error| {
        format!(
            "spawn_failed: {} {}: {error}",
            exe.display(),
            args.join(" ")
        )
    })?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "spawn_failed: child stdout was not piped".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "spawn_failed: child stderr was not piped".to_string())?;
    let (stdout_tx, stdout_rx) = std::sync::mpsc::channel();
    let (stderr_tx, stderr_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = stdout_tx.send(read_stream(stdout));
    });
    std::thread::spawn(move || {
        let _ = stderr_tx.send(read_stream(stderr));
    });

    let mut timed_out = false;
    let exit_code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) => {
                if started.elapsed() >= timeout {
                    timed_out = true;
                    kill_child_tree(&mut child);
                    // Reap the killed child so the readers see EOF.
                    break child.wait().ok().and_then(|status| status.code());
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(error) => {
                return Err(format!("wait_failed: {error}"));
            }
        }
    };

    // Bounded drain: a pipe-inheriting grandchild that survived the kill (or
    // a reader panic) must degrade to a marked partial capture, never a hang.
    let drain = |rx: std::sync::mpsc::Receiver<String>, name: &str| {
        rx.recv_timeout(STREAM_DRAIN_TIMEOUT).unwrap_or_else(|_| {
            log::warn!(
                "assistant build_exec: {name} capture incomplete — a child process is still holding the pipe"
            );
            format!("[{name} capture incomplete: a child process is still holding the pipe]")
        })
    };
    let stdout = drain(stdout_rx, "stdout");
    let stderr = drain(stderr_rx, "stderr");
    Ok(BuildCommandOutput {
        exit_code,
        stdout: truncate_stream(&stdout),
        stderr: truncate_stream(&stderr),
        timed_out,
        duration_ms: started.elapsed().as_millis() as u64,
    })
}

/// Kill the child and, on Unix, its whole process group (enabled by the
/// `process_group(0)` spawn above) so grandchildren die with it.
fn kill_child_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let pgid = child.id() as libc::pid_t;
        // Negative pid targets the process group. The child is its own group
        // leader, so pgid == child pid.
        if unsafe { libc::kill(-pgid, libc::SIGKILL) } == 0 {
            return;
        }
        log::warn!("assistant build_exec: process-group kill failed; killing direct child only");
    }
    if let Err(error) = child.kill() {
        log::warn!("assistant build_exec: kill after timeout failed: {error}");
    }
}

fn read_stream(mut stream: impl Read) -> String {
    let mut buffer = Vec::new();
    if let Err(error) = stream.read_to_end(&mut buffer) {
        log::warn!("assistant build_exec: stream read failed: {error}");
    }
    String::from_utf8_lossy(&buffer).into_owned()
}

/// Keep the head and tail of an oversized stream; the middle is what a model
/// least needs from a long check log.
fn truncate_stream(stream: &str) -> String {
    if stream.chars().count() <= MAX_STREAM_CHARS {
        return stream.to_string();
    }
    let half = MAX_STREAM_CHARS / 2;
    let head: String = stream.chars().take(half).collect();
    let tail_start = stream.chars().count() - half;
    let tail: String = stream.chars().skip(tail_start).collect();
    format!("{head}\n… [output truncated] …\n{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn validate_allows_init_and_check_and_rejects_everything_else() {
        validate_build_args(&args(&["app", "init", "--global", "snake"])).unwrap();
        validate_build_args(&args(&["app", "check", "/tmp/apps/snake"])).unwrap();

        let empty = validate_build_args(&[]).unwrap_err();
        assert!(empty.starts_with("invalid_input"), "{empty}");
        let host = validate_build_args(&args(&["host", "start"])).unwrap_err();
        assert!(host.starts_with("command_not_allowed"), "{host}");
        let uninstall = validate_build_args(&args(&["app", "uninstall", "x"])).unwrap_err();
        assert!(uninstall.starts_with("command_not_allowed"), "{uninstall}");
        let open = validate_build_args(&args(&["app", "init", "snake", "--open"])).unwrap_err();
        assert!(open.contains("host.apps.open"), "{open}");
    }

    #[test]
    fn run_captures_both_streams_and_exit_code() {
        let out = run_build_command(
            &PathBuf::from("/bin/sh"),
            &args(&["-c", "echo made-it; echo warn >&2; exit 3"]),
            &std::env::temp_dir(),
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(out.exit_code, Some(3));
        assert_eq!(out.stdout.trim(), "made-it");
        assert_eq!(out.stderr.trim(), "warn");
        assert!(!out.timed_out);
    }

    #[test]
    fn run_kills_on_timeout_and_reports_it() {
        let out = run_build_command(
            &PathBuf::from("/bin/sh"),
            &args(&["-c", "sleep 30"]),
            &std::env::temp_dir(),
            Duration::from_millis(200),
        )
        .unwrap();
        assert!(out.timed_out);
        assert!(out.duration_ms < 10_000, "{}", out.duration_ms);
    }

    /// Review fix (stint 0421): a grandchild that inherits the pipes must be
    /// killed with the process group (Unix) and must never wedge the capture —
    /// the pre-fix code joined `read_to_end` unboundedly and hung until the
    /// grandchild exited on its own.
    #[cfg(unix)]
    #[test]
    fn run_timeout_reaps_pipe_holding_grandchildren_without_hanging() {
        let started = std::time::Instant::now();
        let out = run_build_command(
            &PathBuf::from("/bin/sh"),
            &args(&["-c", "sleep 30 & sleep 30"]),
            &std::env::temp_dir(),
            Duration::from_millis(200),
        )
        .unwrap();
        assert!(out.timed_out);
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "capture must not wait for the grandchild: {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn missing_binary_fails_loudly() {
        let error = run_build_command(
            &PathBuf::from("/nonexistent/plexi"),
            &args(&["app", "check"]),
            &std::env::temp_dir(),
            Duration::from_secs(1),
        )
        .unwrap_err();
        assert!(error.starts_with("spawn_failed"), "{error}");
    }

    #[test]
    fn truncate_stream_keeps_head_and_tail() {
        let long = "x".repeat(40_000);
        let cut = truncate_stream(&long);
        assert!(cut.contains("[output truncated]"));
        assert!(cut.len() < 20_000);
        assert_eq!(truncate_stream("short"), "short");
    }
}
