//! Embedded, non-visible execution surface for the Assistant's plugin-build
//! commands (stint 0421).
//!
//! The Assistant must never drive a user-visible PTY for its own work.
//! `host.build.run` executes an allowlisted set of `plexi plugin` subcommands
//! (`init`, `check`) as a hidden subprocess of the running host binary,
//! captures both streams, and returns them directly to the model — one tool
//! call per command instead of a `host.terminals.run`/`host.terminals.read`
//! pair against a human-observed terminal. `host.terminals.*` remains only
//! for genuinely user-facing terminal work.

use std::path::Path;
use std::time::{Duration, Instant};

use plexi_distribution::bounded::{Helper, Output, RunError};

use crate::platform::text;

/// Default and maximum wall-clock budget for one build command.
pub(crate) const DEFAULT_BUILD_TIMEOUT_MS: u64 = 120_000;
pub(crate) const MAX_BUILD_TIMEOUT_MS: u64 = 300_000;

/// Per-stream capture cap. Longer output keeps the head and tail with an
/// elision marker, the shape a model can still reason about.
const MAX_STREAM_CHARS: usize = 16_384;

/// Result of one embedded build command.
#[derive(Debug)]
pub(crate) struct BuildCommandOutput {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub duration_ms: u64,
}

/// Gate the embedded surface to the sanctioned plugin-authoring commands.
/// Everything else — including pane-opening flags — must go through the
/// dedicated host tools, so the scope of a `host.build.run` grant stays
/// exactly "plugin scaffolding and validation".
pub(crate) fn validate_build_args(args: &[String]) -> Result<(), String> {
    let (Some(first), Some(second)) = (args.first(), args.get(1)) else {
        return Err(
            "invalid_input: args must start with [\"plugin\", \"init\"|\"check\"]".to_string(),
        );
    };
    if first != "plugin" || !matches!(second.as_str(), "init" | "check") {
        return Err(format!(
            "command_not_allowed: only `plexi plugin init` and `plexi plugin check` run here, got `plexi {}`",
            args.join(" ")
        ));
    }
    if args.iter().any(|arg| arg == "--open") {
        return Err(
            "command_not_allowed: --open is rejected; open the plugin with host.views.open instead"
                .to_string(),
        );
    }
    Ok(())
}

/// Run `exe args…` from `cwd` with no visible surface, as a bounded helper:
/// stdin closed, both streams captured whole, and the child's whole session
/// (uv/python spawned by `plexi plugin check`) killed when it exits or at
/// `timeout`. A timeout is reported to the model with what was written so
/// far; a run that could not be stopped, or whose output is incomplete, is
/// an error.
pub(crate) fn run_build_command(
    exe: &Path,
    args: &[String],
    cwd: &Path,
    timeout: Duration,
) -> Result<BuildCommandOutput, String> {
    let started = Instant::now();
    let what = format!("plexi {}", args.join(" "));
    let run = Helper::new(exe)
        .args(args)
        .current_dir(cwd)
        .run_any_exit(&what, timeout);
    let (exit_code, output, timed_out) = match run {
        Ok(exited) => (exited.status.code(), exited.output, false),
        Err(RunError::TimedOut { stdout, stderr, .. }) => (None, Output { stdout, stderr }, true),
        Err(error @ RunError::Start { .. }) => {
            return Err(format!(
                "spawn_failed: {} {}: {error}",
                exe.display(),
                args.join(" ")
            ));
        }
        Err(error) => return Err(format!("run_failed: {error}")),
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    Ok(BuildCommandOutput {
        exit_code,
        stdout: text::head_tail(&stdout, MAX_STREAM_CHARS, text::Style::Block),
        stderr: text::head_tail(&stderr, MAX_STREAM_CHARS, text::Style::Block),
        timed_out,
        duration_ms: started.elapsed().as_millis() as u64,
    })
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
        validate_build_args(&args(&["plugin", "init", "--global", "snake"])).unwrap();
        validate_build_args(&args(&["plugin", "check", "/tmp/apps/snake"])).unwrap();

        let empty = validate_build_args(&[]).unwrap_err();
        assert!(empty.starts_with("invalid_input"), "{empty}");
        let host = validate_build_args(&args(&["host", "start"])).unwrap_err();
        assert!(host.starts_with("command_not_allowed"), "{host}");
        let uninstall = validate_build_args(&args(&["plugin", "uninstall", "x"])).unwrap_err();
        assert!(uninstall.starts_with("command_not_allowed"), "{uninstall}");
        let open = validate_build_args(&args(&["plugin", "init", "snake", "--open"])).unwrap_err();
        assert!(open.contains("host.views.open"), "{open}");
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

    /// Stint 1626: a command that exits while a grandchild still holds its
    /// output returns at once with the whole capture, and the grandchild dies
    /// with it. The old runner waited out a drain bound and returned an
    /// "incomplete" marker instead of the output.
    #[cfg(unix)]
    #[test]
    fn an_exit_with_a_pipe_holding_grandchild_returns_whole_output_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let sleeper = dir.path().join("sleeper");
        let started = std::time::Instant::now();
        let out = run_build_command(
            &PathBuf::from("/bin/sh"),
            &args(&[
                "-c",
                &format!("sleep 30 & echo $! > '{}'; echo built", sleeper.display()),
            ]),
            &std::env::temp_dir(),
            Duration::from_secs(60),
        )
        .unwrap();
        let elapsed = started.elapsed();
        let sleeper: i32 = std::fs::read_to_string(&sleeper)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        // SAFETY: kill(pid, 0) only probes; SIGKILL cleans up a test process.
        let probe = || unsafe { libc::kill(sleeper, 0) } == 0;
        let gone_by = std::time::Instant::now() + Duration::from_secs(3);
        while probe() && std::time::Instant::now() < gone_by {
            std::thread::sleep(Duration::from_millis(20));
        }
        let alive = probe();
        unsafe { libc::kill(sleeper, libc::SIGKILL) };
        assert_eq!(out.exit_code, Some(0));
        assert_eq!(out.stdout.trim(), "built");
        assert!(!out.timed_out);
        assert!(elapsed < Duration::from_secs(3), "held for {elapsed:?}");
        assert!(!alive, "grandchild {sleeper} outlived the command");
    }

    #[test]
    fn missing_binary_fails_loudly() {
        let error = run_build_command(
            &PathBuf::from("/nonexistent/plexi"),
            &args(&["plugin", "check"]),
            &std::env::temp_dir(),
            Duration::from_secs(1),
        )
        .unwrap_err();
        assert!(error.starts_with("spawn_failed"), "{error}");
    }
}
