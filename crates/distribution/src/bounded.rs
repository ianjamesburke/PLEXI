//! Run a child process with a hard time limit (stint 1605).
//!
//! Output goes to anonymous temp files, not pipes, so nothing a descendant
//! keeps open can block the caller. On Unix the child leads its own process
//! group, and at the limit the whole group is killed while the child is still
//! unreaped (so the group id cannot be reused), then the child is reaped. A
//! process that detaches into its own session, as `host start` does for the
//! host, is outside the group and survives.

use crate::{Error, IoContext, Result};
use std::io::{Read, Seek};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

pub struct Finished {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Run `command`, named `what` in errors, for at most `limit`.
pub fn run(mut command: Command, what: &str, limit: Duration) -> Result<Finished> {
    let stdout = tempfile::tempfile().context(format!("create output file for {what}"))?;
    let stderr = tempfile::tempfile().context(format!("create error file for {what}"))?;
    command
        .stdin(Stdio::null())
        .stdout(
            stdout
                .try_clone()
                .context(format!("share output file for {what}"))?,
        )
        .stderr(
            stderr
                .try_clone()
                .context(format!("share error file for {what}"))?,
        );
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut child = command.spawn().context(format!("start {what}"))?;
    let deadline = Instant::now() + limit;
    let status = loop {
        if let Some(status) = child.try_wait().context(format!("wait for {what}"))? {
            break status;
        }
        if Instant::now() >= deadline {
            #[cfg(unix)]
            // SAFETY: the child is unreaped, so its pid is still its group id.
            unsafe {
                libc::killpg(child.id() as libc::pid_t, libc::SIGKILL);
            }
            // Windows kills only the child here; capture through files means
            // a surviving descendant cannot block the caller.
            #[cfg(windows)]
            let _ = child.kill();
            let reaped = child.wait();
            log::warn!(
                "distribution: {what} exceeded {}s; killed, reaped={reaped:?}",
                limit.as_secs()
            );
            return Err(Error::Invalid(format!(
                "{what} did not finish within {}s and was stopped",
                limit.as_secs()
            )));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let read = |mut file: std::fs::File, which: &str| -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        file.rewind()
            .and_then(|()| file.read_to_end(&mut bytes))
            .context(format!("read {which} of {what}"))?;
        Ok(bytes)
    };
    Ok(Finished {
        status,
        stdout: read(stdout, "output")?,
        stderr: read(stderr, "errors")?,
    })
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
