//! Run a child process with a hard time limit (stint 1605).
//!
//! Output goes to anonymous temp files, not pipes, so nothing a descendant
//! keeps open can block the caller, and stdin is `/dev/null`. On Unix the
//! child leads a new session with no controlling terminal, so it can neither
//! read nor write the caller's terminal; a session leader also leads its own
//! process group. At the limit the whole group is killed while the child is
//! still unreaped (so the group id cannot be reused), then the child is
//! reaped. A process that detaches into its own session, as `host start` does
//! for the host, is outside the group and survives. If the kill fails and the
//! child does not exit, the run still returns, with an error saying the child
//! may still be running. Output past `CAPTURE_LIMIT` is cut and the cut is
//! reported, so a caller that keeps the output can refuse an incomplete one.

use crate::{Error, IoContext, Result};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// The most read back from each output stream. A descendant that outlives
/// the child may keep writing; the read stops here instead of following it.
pub const CAPTURE_LIMIT: u64 = 1024 * 1024;

/// How long a killed child gets to be reaped before the run gives up on it.
const REAP_LIMIT: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub struct Finished {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// More than `CAPTURE_LIMIT` bytes were written to stdout; `stdout` holds
    /// only the first `CAPTURE_LIMIT`.
    pub stdout_truncated: bool,
    /// As `stdout_truncated`, for stderr.
    pub stderr_truncated: bool,
}

impl Finished {
    /// The whole of stdout, or `Error::OutputTruncated` when it was cut. For
    /// callers that keep the output, such as an installed file.
    pub fn complete_stdout(&self, what: &str) -> Result<&[u8]> {
        if self.stdout_truncated {
            return Err(Error::OutputTruncated {
                what: what.to_string(),
                limit: CAPTURE_LIMIT,
            });
        }
        Ok(&self.stdout)
    }
}

/// Run `command`, named `what` in errors, for at most `limit`.
pub fn run(mut command: Command, what: &str, limit: Duration) -> Result<Finished> {
    // Named so the read-back opens its own handle: a positioned read on the
    // child's handle would share (on Windows, move) the writer's offset.
    let stdout =
        tempfile::NamedTempFile::new().context(format!("create output file for {what}"))?;
    let stderr = tempfile::NamedTempFile::new().context(format!("create error file for {what}"))?;
    command
        .stdin(Stdio::null())
        .stdout(
            stdout
                .as_file()
                .try_clone()
                .context(format!("share output file for {what}"))?,
        )
        .stderr(
            stderr
                .as_file()
                .try_clone()
                .context(format!("share error file for {what}"))?,
        );
    // SAFETY: between fork and exec the closure calls only setsid(), which is
    // async-signal-safe and neither allocates nor locks.
    #[cfg(unix)]
    unsafe {
        std::os::unix::process::CommandExt::pre_exec(&mut command, || {
            // A forked child never leads a process group, so setsid succeeds.
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().context(format!("start {what}"))?;
    let deadline = Instant::now() + limit;
    let status = loop {
        if let Some(status) = child.try_wait().context(format!("wait for {what}"))? {
            break status;
        }
        if Instant::now() >= deadline {
            return Err(stop(child, what, limit, kill_tree, REAP_LIMIT));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let read = |file: tempfile::NamedTempFile, which: &str| {
        read_capped(file.path(), CAPTURE_LIMIT).context(format!("read {which} of {what}"))
    };
    let (stdout, stdout_truncated) = read(stdout, "output")?;
    let (stderr, stderr_truncated) = read(stderr, "errors")?;
    if stdout_truncated || stderr_truncated {
        log::warn!(
            "distribution: {what} wrote more than {CAPTURE_LIMIT} bytes; capture cut (stdout={stdout_truncated} stderr={stderr_truncated})"
        );
    }
    Ok(Finished {
        status,
        stdout,
        stderr,
        stdout_truncated,
        stderr_truncated,
    })
}

/// Kill the child's whole process group. The child is unreaped, so its pid is
/// still its group id. A group already gone is not a failure.
#[cfg(unix)]
fn kill_tree(child: &mut std::process::Child) -> std::io::Result<()> {
    // SAFETY: killpg only sends a signal; the pid names an unreaped child.
    if unsafe { libc::killpg(child.id() as libc::pid_t, libc::SIGKILL) } == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::ESRCH) => Ok(()),
        _ => Err(error),
    }
}

/// Windows kills only the child here; capture through files means a
/// surviving descendant cannot block the caller.
#[cfg(windows)]
fn kill_tree(child: &mut std::process::Child) -> std::io::Result<()> {
    child.kill()
}

/// Stop a child that ran past `limit`: kill it with `kill`, then reap it,
/// waiting at most `reap_limit` so a failed kill never holds the caller.
fn stop(
    mut child: std::process::Child,
    what: &str,
    limit: Duration,
    kill: fn(&mut std::process::Child) -> std::io::Result<()>,
    reap_limit: Duration,
) -> Error {
    let killed = kill(&mut child);
    let reap_deadline = Instant::now() + reap_limit;
    let reaped = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < reap_deadline => {
                std::thread::sleep(Duration::from_millis(20))
            }
            Ok(None) => break Err(format!("still running {reap_limit:?} after the kill")),
            Err(error) => break Err(format!("wait failed: {error}")),
        }
    };
    match (killed, reaped) {
        (_, Ok(status)) => {
            log::warn!("distribution: {what} exceeded {limit:?}; killed and reaped ({status})");
            Error::TimedOut {
                what: what.to_string(),
                limit,
            }
        }
        (killed, Err(reap)) => {
            let cause = match killed {
                Ok(()) => reap,
                Err(error) => format!("kill failed: {error}; {reap}"),
            };
            log::error!(
                "distribution: {what} exceeded {limit:?} and could not be stopped: {cause}"
            );
            Error::NotStopped {
                what: what.to_string(),
                limit,
                cause,
            }
        }
    }
}

/// Read up to `cap` bytes of `path` from offset 0 through a handle of its
/// own, so the read never shares an offset with a process still writing.
/// The flag is true when the file held more than `cap` bytes.
fn read_capped(path: &std::path::Path, cap: u64) -> std::io::Result<(Vec<u8>, bool)> {
    #[cfg(unix)]
    use std::os::unix::fs::FileExt;
    #[cfg(windows)]
    use std::os::windows::fs::FileExt;
    let file = std::fs::File::open(path)?;
    // One byte past the cap tells a cut capture from one that fits exactly.
    let probe = cap + 1;
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    while (bytes.len() as u64) < probe {
        let want = chunk.len().min((probe - bytes.len() as u64) as usize);
        let offset = bytes.len() as u64;
        #[cfg(unix)]
        let got = file.read_at(&mut chunk[..want], offset);
        #[cfg(windows)]
        let got = file.seek_read(&mut chunk[..want], offset);
        match got {
            Ok(0) => break,
            Ok(n) => bytes.extend_from_slice(&chunk[..n]),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    let truncated = bytes.len() as u64 > cap;
    bytes.truncate(cap as usize);
    Ok((bytes, truncated))
}

#[cfg(test)]
mod capture_tests {
    use super::*;
    use std::io::Write;

    /// The read stops at the cap and leaves the writer's offset alone, so a
    /// later write lands after, not over, what was already written. Runs on
    /// every platform; unrun on Windows so far.
    #[test]
    fn a_capped_read_never_moves_the_writers_offset() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut writer = file.as_file().try_clone().unwrap();
        writer.write_all(b"0123456789").unwrap();
        assert_eq!(
            read_capped(file.path(), 4).unwrap(),
            (b"0123".to_vec(), true)
        );
        writer.write_all(b"ab").unwrap();
        assert_eq!(
            read_capped(file.path(), 100).unwrap(),
            (b"0123456789ab".to_vec(), false)
        );
    }

    /// A capture that fits exactly is complete; one byte more is reported as
    /// cut, and `complete_stdout` refuses it (stint 1610).
    #[test]
    fn a_cut_capture_is_reported_and_refused() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), b"0123").unwrap();
        assert_eq!(
            read_capped(file.path(), 4).unwrap(),
            (b"0123".to_vec(), false)
        );
        std::fs::write(file.path(), b"01234").unwrap();
        let (stdout, stdout_truncated) = read_capped(file.path(), 4).unwrap();
        assert!(stdout_truncated);
        let finished = Finished {
            status: ExitStatus::default(),
            stdout,
            stderr: Vec::new(),
            stdout_truncated,
            stderr_truncated: false,
        };
        let error = finished.complete_stdout("completions").unwrap_err();
        assert!(matches!(error, Error::OutputTruncated { .. }), "{error}");
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::transaction::process_alive;

    fn sh(script: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", script]);
        command
    }

    fn pid_in(path: &std::path::Path) -> u32 {
        std::fs::read_to_string(path)
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    }

    /// The child leads a new session, so it has no controlling terminal and
    /// leads its own process group; a login shell run here cannot touch the
    /// caller's terminal. A descendant keeps the session and group ids after
    /// the child exits, so they are read from it.
    #[test]
    fn the_child_leads_its_own_session() {
        let finished = run(
            sh("sleep 30 & echo $$ $!"),
            "session probe",
            Duration::from_secs(5),
        )
        .unwrap();
        let text = String::from_utf8_lossy(&finished.stdout);
        let pids: Vec<libc::pid_t> = text
            .split_whitespace()
            .map(|p| p.parse().unwrap())
            .collect();
        let (child, sleeper) = (pids[0], pids[1]);
        // SAFETY: getsid and getpgid only read ids; kill cleans up a process
        // this test started.
        let (session, group, ours) = unsafe {
            (
                libc::getsid(sleeper),
                libc::getpgid(sleeper),
                libc::getsid(0),
            )
        };
        unsafe { libc::kill(sleeper, libc::SIGKILL) };
        assert!(finished.status.success(), "{finished:?}");
        assert_eq!(session, child, "child leads a new session: {text}");
        assert_eq!(group, child, "child leads its process group: {text}");
        assert_ne!(session, ours, "child left the caller's session: {text}");
    }

    /// A descendant that keeps stdout open does not hold the caller: capture
    /// is a file, so the run returns as soon as the child exits.
    #[test]
    fn a_descendant_holding_output_does_not_block() {
        let dir = tempfile::tempdir().unwrap();
        let sleeper = dir.path().join("sleeper");
        let started = Instant::now();
        let finished = run(
            sh(&format!(
                "sleep 600 & echo $! > '{}'; echo done",
                sleeper.display()
            )),
            "test child",
            Duration::from_secs(30),
        )
        .unwrap();
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(finished.status.success());
        assert_eq!(String::from_utf8_lossy(&finished.stdout).trim(), "done");
        let pid = pid_in(&sleeper);
        // SAFETY: test cleanup of a process this test started.
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
    }

    /// A descendant that keeps writing after the child exits neither holds
    /// the run nor grows the capture past the cap.
    #[test]
    fn a_descendant_that_keeps_writing_is_capped() {
        let dir = tempfile::tempdir().unwrap();
        let writer = dir.path().join("writer");
        let started = Instant::now();
        let finished = run(
            sh(&format!(
                "yes plexi-bounded-capture & echo $! > '{}'; sleep 0.3",
                writer.display()
            )),
            "test child",
            Duration::from_secs(30),
        )
        .unwrap();
        let elapsed = started.elapsed();
        let pid = pid_in(&writer);
        // SAFETY: test cleanup of a process this test started.
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
        assert!(elapsed < Duration::from_secs(10), "{elapsed:?}");
        assert!(finished.status.success());
        assert!(!finished.stdout.is_empty());
        assert_eq!(finished.stdout.len() as u64, CAPTURE_LIMIT);
        assert!(finished.stdout_truncated);
        assert!(finished.stdout.starts_with(b"plexi-bounded-capture\n"));
    }

    /// A child the kill cannot stop does not hold the run: the reap gives up
    /// at its own bound and the error says the child may still be running
    /// (stint 1610). The kill is stubbed to fail, as a real one rarely does.
    #[test]
    fn a_failed_kill_returns_instead_of_waiting_forever() {
        let child = sh("sleep 600").spawn().unwrap();
        let pid = child.id();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let refuse = |_: &mut std::process::Child| Err(std::io::Error::other("refused"));
            let error = stop(
                child,
                "test child",
                Duration::from_millis(1),
                refuse,
                Duration::from_millis(300),
            );
            let _ = tx.send(error);
        });
        let error = rx.recv_timeout(Duration::from_secs(10));
        // SAFETY: test cleanup of a process this test started.
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
        let error = error.expect("stop returned within its reap bound");
        let text = error.to_string();
        assert!(matches!(error, Error::NotStopped { .. }), "{text}");
        assert!(text.contains("refused"), "{text}");
        assert!(text.contains("may still be running"), "{text}");
    }

    /// A hung child and its descendants are killed at the limit and reaped.
    #[test]
    fn a_hung_child_and_its_descendants_are_killed_at_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let (child, sleeper) = (dir.path().join("child"), dir.path().join("sleeper"));
        let started = Instant::now();
        let error = run(
            sh(&format!(
                "echo $$ > '{}'; sleep 600 & echo $! > '{}'; wait",
                child.display(),
                sleeper.display()
            )),
            "test child",
            Duration::from_millis(500),
        )
        .err()
        .unwrap()
        .to_string();
        assert!(started.elapsed() < Duration::from_secs(10), "{error}");
        assert!(error.contains("did not finish within"), "{error}");
        let (child, sleeper) = (pid_in(&child), pid_in(&sleeper));
        let deadline = Instant::now() + Duration::from_secs(5);
        while process_alive(sleeper) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!process_alive(child), "child {child} survived");
        assert!(!process_alive(sleeper), "descendant {sleeper} survived");
    }
}
