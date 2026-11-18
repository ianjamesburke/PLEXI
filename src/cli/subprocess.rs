//! Timeout-bounded subprocess capture, shared by every CLI-descriptor path
//! (`--plexi` native probe, registry version detection, and the Tier-3 `--help`
//! crawl). Adapts `plexi_distribution::bounded::run` to the descriptor tiers,
//! so no descriptor path can ever block the host on a misbehaving CLI.

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

/// Run `bin args…` through `plexi_distribution::bounded::run`, capturing
/// stdout/stderr. At `timeout` the child and its process group are killed and
/// reaped. Never blocks indefinitely.
///
/// Errors when the child cannot run (binary not on PATH, permission denied)
/// or could not be stopped at the limit and may still be running. A timeout
/// that was stopped is a failed `Captured`, not an `Err`, because the caller
/// usually wants to fall through to the next descriptor tier rather than abort.
pub fn run_capture(bin: &str, args: &[&str], timeout: Duration) -> std::io::Result<Captured> {
    let mut command = std::process::Command::new(bin);
    command.args(args);
    let what = format!("`{bin} {}`", args.join(" "));
    match plexi_distribution::bounded::run(command, &what, timeout) {
        Ok(finished) => Ok(Captured {
            success: finished.status.success(),
            stdout: String::from_utf8_lossy(&finished.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&finished.stderr).into_owned(),
        }),
        Err(plexi_distribution::Error::TimedOut { .. }) => {
            log::warn!("cli_subprocess: {what} timed out after {timeout:?}; killed and reaped");
            Ok(Captured {
                success: false,
                stdout: String::new(),
                stderr: String::new(),
            })
        }
        Err(error) => Err(std::io::Error::other(error)),
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

    /// Regression (stint 1610): at the limit the child and its descendants
    /// are killed and the child is reaped, even while a descendant holds its
    /// output open. Before, only the child was signalled and the reaper sat
    /// behind the descendant's open pipe, leaving a zombie and a live sleeper.
    #[cfg(unix)]
    #[test]
    fn a_timed_out_child_and_its_descendants_are_killed_and_reaped() {
        let dir = tempfile::tempdir().unwrap();
        let (child_file, sleeper_file) = (dir.path().join("child"), dir.path().join("sleeper"));
        let script = format!(
            "echo $$ > '{}'; sleep 600 & echo $! > '{}'; wait",
            child_file.display(),
            sleeper_file.display()
        );
        let c = run_capture("/bin/sh", &["-c", &script], Duration::from_millis(500))
            .expect("sh spawns");
        assert!(!c.success);
        let pid = |path: &std::path::Path| -> libc::pid_t {
            std::fs::read_to_string(path)
                .unwrap()
                .trim()
                .parse()
                .unwrap()
        };
        let (child, sleeper) = (pid(&child_file), pid(&sleeper_file));
        // SAFETY: signal 0 only checks whether a pid exists; an unreaped
        // zombie still exists.
        let exists = |pid: libc::pid_t| unsafe { libc::kill(pid, 0) } == 0;
        let child_left = exists(child);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while exists(sleeper) && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        let sleeper_left = exists(sleeper);
        // SAFETY: test cleanup of processes this test started.
        unsafe {
            libc::kill(sleeper, libc::SIGKILL);
            libc::kill(child, libc::SIGKILL);
        }
        assert!(!child_left, "timed-out child {child} was left unreaped");
        assert!(!sleeper_left, "descendant {sleeper} survived the limit");
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
