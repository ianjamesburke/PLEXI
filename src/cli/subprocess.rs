//! Timeout-bounded subprocess capture, shared by every CLI-descriptor path
//! (`--plexi` native probe, registry version detection, and the Tier-3 `--help`
//! crawl). Centralises the "spawn, capture, kill if it hangs" logic so no
//! descriptor path can ever block the host on a misbehaving CLI.

use std::time::Duration;

/// Captured result of a bounded subprocess run. A timeout surfaces as
/// `success == false` with empty output (and an internal `log::warn`); callers
/// that reach the next descriptor tier on any failure don't need to tell a
/// timeout apart from a non-zero exit.
pub struct Captured {
    /// Did the process exit with a success status (exit code 0)?
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

impl Captured {
    /// Help/usage text, preferring stdout but falling back to stderr — many
    /// CLIs print `--help` to stderr while exiting 0.
    pub fn help_text(&self) -> String {
        if self.stdout.trim().is_empty() {
            self.stderr.clone()
        } else {
            self.stdout.clone()
        }
    }
}

/// Run `bin args…`, capturing stdout/stderr, killing the child if it runs
/// longer than `timeout`. Never blocks indefinitely.
///
/// Errors only on spawn failure (binary not on PATH, permission denied). A
/// timeout is reported via `Captured::timed_out`, not as an `Err`, because the
/// caller usually wants to fall through to the next descriptor tier rather than
/// abort.
pub fn run_capture(bin: &str, args: &[&str], timeout: Duration) -> std::io::Result<Captured> {
    use std::process::Stdio;
    use std::sync::mpsc;

    let child = std::process::Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let pid = child.id();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });

    match rx.recv_timeout(timeout) {
        Ok(Ok(output)) => Ok(Captured {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }),
        Ok(Err(e)) => Err(e),
        Err(_) => {
            // Deadline exceeded — kill the child so it doesn't linger. The
            // reader thread owns the `Child`, so we signal by pid.
            #[cfg(unix)]
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGKILL);
            }
            #[cfg(not(unix))]
            let _ = pid;
            log::warn!(
                "cli_subprocess: `{bin} {}` timed out after {}s — killed",
                args.join(" "),
                timeout.as_secs()
            );
            Ok(Captured {
                success: false,
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_stdout_on_success() {
        let c = run_capture("echo", &["hello"], Duration::from_secs(5)).expect("echo runs");
        assert!(c.success);
        assert_eq!(c.stdout.trim(), "hello");
    }

    #[test]
    fn timeout_kills_long_running_child() {
        let start = std::time::Instant::now();
        let c = run_capture("sleep", &["10"], Duration::from_millis(300)).expect("sleep spawns");
        // A timeout surfaces as a failed run with empty output…
        assert!(!c.success);
        assert!(c.stdout.is_empty());
        // …and must return promptly, not wait the full 10s.
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "timeout did not short-circuit: took {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn spawn_failure_is_err() {
        let r = run_capture(
            "__plexi_nonexistent_binary_xyz__",
            &["--help"],
            Duration::from_secs(2),
        );
        assert!(r.is_err(), "expected spawn error for missing binary");
    }

    #[test]
    fn help_text_falls_back_to_stderr() {
        let c = Captured {
            success: true,
            stdout: "   ".into(),
            stderr: "usage info".into(),
        };
        assert_eq!(c.help_text(), "usage info");
    }
}
