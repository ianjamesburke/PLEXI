//! The one way to run a helper process: bounded in time, never interactive,
//! and never Ok with incomplete work (stints 1605, 1610).
//!
//! `Helper` exposes only arguments, environment and working directory. stdin
//! is `/dev/null` and output goes to temp files, not pipes, so nothing a
//! descendant keeps open can block the caller. On Unix the child leads a new
//! session with no controlling terminal, so it can neither read nor write the
//! caller's terminal; a session leader also leads its own process group.
//!
//! When the child exits, its exit is observed without reaping it, its group is
//! killed while the zombie still holds the group id, then it is reaped: a
//! background job it started dies with it. At the limit the group is killed
//! the same way. A process that called `setsid` itself, as the host does under
//! `host start`, is outside the group and survives. If a kill fails, or the
//! killed child is not reaped within `REAP_LIMIT`, the run still returns, with
//! `RunError::NotStopped`.
//!
//! Output past `CAPTURE_LIMIT` is `RunError::OutputTooLarge`, never cut bytes.
//! Windows kills only the child; descendants there are stint 1613.

use std::ffi::OsStr;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// The most read back from each output stream. More than this is an error.
pub const CAPTURE_LIMIT: u64 = 1024 * 1024;

/// How long a killed child gets to be reaped before the run gives up on it.
const REAP_LIMIT: Duration = Duration::from_secs(5);

/// The longest pause between exit checks. Checks start at 1 ms and back off
/// to this, so a fast helper does not pay a fixed delay.
const MAX_POLL: Duration = Duration::from_millis(20);

/// Both output streams of a helper, each read whole.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Output {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// A helper that exited by itself inside its limit, with any exit code.
#[derive(Debug)]
pub struct Exited {
    pub status: ExitStatus,
    pub output: Output,
}

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("could not start {what}: {source}")]
    Start {
        what: String,
        source: std::io::Error,
    },
    #[error("{what} failed ({status}){}", stderr_suffix(stderr))]
    Failed {
        what: String,
        status: ExitStatus,
        stderr: Vec<u8>,
    },
    #[error("{what} did not finish within {limit:?} and was stopped")]
    TimedOut {
        what: String,
        limit: Duration,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
    #[error("{what} could not be stopped ({cause}); it may still be running")]
    NotStopped { what: String, cause: String },
    #[error("{what} wrote more than {limit} bytes to {stream}; its output is incomplete")]
    OutputTooLarge {
        what: String,
        stream: &'static str,
        limit: u64,
    },
    #[error("{action} for {what}: {source}")]
    Io {
        what: String,
        action: &'static str,
        source: std::io::Error,
    },
}

fn stderr_suffix(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let text = text.trim();
    if text.is_empty() {
        String::new()
    } else {
        format!(": {text}")
    }
}

/// A helper process to run once, bounded. Built like a `Command`, but only
/// arguments, environment and working directory can be set.
pub struct Helper {
    command: Command,
}

impl Helper {
    #[expect(
        clippy::disallowed_methods,
        reason = "helper: the one bounded launch every helper goes through"
    )]
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        Self {
            command: Command::new(program),
        }
    }

    pub fn arg(&mut self, arg: impl AsRef<OsStr>) -> &mut Self {
        self.command.arg(arg);
        self
    }

    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.command.args(args);
        self
    }

    pub fn env(&mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> &mut Self {
        self.command.env(key, value);
        self
    }

    pub fn env_remove(&mut self, key: impl AsRef<OsStr>) -> &mut Self {
        self.command.env_remove(key);
        self
    }

    pub fn env_clear(&mut self) -> &mut Self {
        self.command.env_clear();
        self
    }

    pub fn current_dir(&mut self, dir: impl AsRef<Path>) -> &mut Self {
        self.command.current_dir(dir);
        self
    }

    /// The environment changes set so far, as `Command::get_envs` reports
    /// them. For tests of a constructor's policy.
    pub fn get_envs(&self) -> std::process::CommandEnvs<'_> {
        self.command.get_envs()
    }

    /// Run to completion within `limit`. Ok only for exit 0 with both streams
    /// read whole; a non-zero exit is `RunError::Failed`.
    pub fn run(&mut self, what: &str, limit: Duration) -> Result<Output, RunError> {
        let exited = self.run_any_exit(what, limit)?;
        if !exited.status.success() {
            return Err(RunError::Failed {
                what: what.to_string(),
                status: exited.status,
                stderr: exited.output.stderr,
            });
        }
        Ok(exited.output)
    }

    /// As `run`, but any exit code is Ok, for callers that read the code.
    pub fn run_any_exit(&mut self, what: &str, limit: Duration) -> Result<Exited, RunError> {
        let io = |action: &'static str| {
            move |source| RunError::Io {
                what: what.to_string(),
                action,
                source,
            }
        };
        // Named so the read-back opens its own handle: a positioned read on
        // the child's handle would share (on Windows, move) the writer's
        // offset.
        let stdout = tempfile::NamedTempFile::new().map_err(io("create output file"))?;
        let stderr = tempfile::NamedTempFile::new().map_err(io("create error file"))?;
        self.command
            .stdin(Stdio::null())
            .stdout(
                stdout
                    .as_file()
                    .try_clone()
                    .map_err(io("share output file"))?,
            )
            .stderr(
                stderr
                    .as_file()
                    .try_clone()
                    .map_err(io("share error file"))?,
            );
        // SAFETY: between fork and exec the closure calls only setsid(), which
        // is async-signal-safe and neither allocates nor locks.
        #[cfg(unix)]
        unsafe {
            std::os::unix::process::CommandExt::pre_exec(&mut self.command, || {
                // A forked child never leads a process group, so setsid
                // succeeds.
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = self.command.spawn().map_err(|source| RunError::Start {
            what: what.to_string(),
            source,
        })?;
        let deadline = Instant::now() + limit;
        let mut pause = Duration::from_millis(1);
        let status = loop {
            let exited = exited_unreaped(&mut child).map_err(io("wait"))?;
            let step = next_step(Instant::now(), deadline, exited, pause);
            if let Step::Wait(wait) = step {
                std::thread::sleep(wait);
                pause = (pause * 2).min(MAX_POLL);
                continue;
            }
            if step == Step::Finish {
                break finish_session(child, what)?;
            }
            {
                let captured = |file: &tempfile::NamedTempFile| {
                    read_capped(file.path(), CAPTURE_LIMIT)
                        .map(|(bytes, _)| bytes)
                        .unwrap_or_default()
                };
                return Err(stop(
                    child,
                    what,
                    limit,
                    kill_tree,
                    REAP_LIMIT,
                    captured(&stdout),
                    captured(&stderr),
                ));
            }
        };
        let read = |file: tempfile::NamedTempFile, stream: &'static str| {
            let (bytes, over) =
                read_capped(file.path(), CAPTURE_LIMIT).map_err(io("read output"))?;
            if over {
                return Err(RunError::OutputTooLarge {
                    what: what.to_string(),
                    stream,
                    limit: CAPTURE_LIMIT,
                });
            }
            Ok(bytes)
        };
        let output = Output {
            stdout: read(stdout, "stdout")?,
            stderr: read(stderr, "stderr")?,
        };
        Ok(Exited { status, output })
    }
}

/// A `git` helper that never prompts, and the only way any module starts
/// git. Besides the helper's missing terminal, every other prompt route is
/// shut: terminal prompts (`GIT_TERMINAL_PROMPT=0`), askpass programs (an
/// empty `GIT_ASKPASS` stops git looking further), and ssh's `SSH_ASKPASS`
/// (cleared and refused). Credentials must come from an ssh key or agent or
/// a credential helper; `git_error` says so when they are missing.
pub fn git() -> Helper {
    let mut helper = Helper::new("git");
    helper
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .env("SSH_ASKPASS_REQUIRE", "never")
        .env_remove("SSH_ASKPASS");
    helper
}

/// A git helper's error, plus how to set up credentials when git failed for
/// want of ones it was not allowed to prompt for.
pub fn git_error(error: &RunError) -> String {
    const AUTH_MARKERS: &[&str] = &[
        "terminal prompts disabled",
        "could not read Username",
        "could not read Password",
        "Authentication failed",
        "Permission denied (publickey",
        "Host key verification failed",
        "Could not read from remote repository",
    ];
    let mut message = error.to_string();
    if let RunError::Failed { stderr, .. } = error {
        let stderr = String::from_utf8_lossy(stderr);
        if AUTH_MARKERS.iter().any(|marker| stderr.contains(marker)) {
            message.push_str(
                "\ngit could not authenticate and Plexi does not prompt for credentials. \
For an ssh URL, add an ssh key for the host to your ssh agent and accept its host key \
once with `ssh -T <host>`; for an https GitHub URL, run `gh auth login` then \
`gh auth setup-git`. Then retry.",
            );
        }
    }
    message
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// The child exited inside its limit.
    Finish,
    /// The limit has passed, whether or not the child has exited since: an
    /// exit seen after the limit is not a success.
    Stop,
    /// Check again after this long, never past the limit.
    Wait(Duration),
}

fn next_step(now: Instant, deadline: Instant, exited: bool, pause: Duration) -> Step {
    if now >= deadline {
        Step::Stop
    } else if exited {
        Step::Finish
    } else {
        Step::Wait(pause.min(deadline - now))
    }
}

/// Whether the child has exited, without reaping it, so its pid (and group
/// id) stay reserved until the group is killed.
#[cfg(unix)]
fn exited_unreaped(child: &mut Child) -> std::io::Result<bool> {
    // SAFETY: a zeroed siginfo_t is a valid out-parameter for waitid.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    // SAFETY: waitid writes only into `info`; WNOWAIT leaves the child
    // unreaped, and the pid names our own unreaped child.
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            child.id() as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result == -1 {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::Interrupted {
            return Ok(false);
        }
        return Err(error);
    }
    // A child that has not exited leaves si_pid zero.
    #[cfg(target_vendor = "apple")]
    let pid = info.si_pid;
    // SAFETY: waitid filled `info` for a child state change.
    #[cfg(not(target_vendor = "apple"))]
    let pid = unsafe { info.si_pid() };
    Ok(pid != 0)
}

#[cfg(windows)]
fn exited_unreaped(child: &mut Child) -> std::io::Result<bool> {
    child.try_wait().map(|status| status.is_some())
}

/// The child exited by itself: kill what is left of its session, then reap
/// it.
fn finish_session(mut child: Child, what: &str) -> Result<ExitStatus, RunError> {
    let killed = kill_tree(&mut child);
    let status = child.wait().map_err(|source| RunError::Io {
        what: what.to_string(),
        action: "reap",
        source,
    })?;
    if let Err(error) = killed {
        return Err(RunError::NotStopped {
            what: what.to_string(),
            cause: format!("it exited ({status}) but its session could not be ended: {error}"),
        });
    }
    Ok(status)
}

/// How long the processes left in a helper's session get to die.
#[cfg(unix)]
const SESSION_KILL_LIMIT: Duration = Duration::from_secs(5);

/// Kill every process in the child's session, whatever process group it moved
/// to, and confirm none is left alive. The child leads the session, so its
/// pid is the session id; it is unreaped, so the id cannot be reused. A
/// process that called setsid itself has left the session and is not touched.
#[cfg(unix)]
fn kill_tree(child: &mut Child) -> std::io::Result<()> {
    let session = child.id() as libc::pid_t;
    // SAFETY: kill only sends a signal; the pid names our unreaped child.
    unsafe { libc::kill(session, libc::SIGKILL) };
    let deadline = Instant::now() + SESSION_KILL_LIMIT;
    loop {
        let alive = live_session_members(session)?;
        if alive.is_empty() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(std::io::Error::other(format!(
                "processes {alive:?} in its session are still alive {SESSION_KILL_LIMIT:?} after SIGKILL"
            )));
        }
        for pid in alive {
            // SAFETY: kill only sends a signal to a member of our session.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Live processes whose session is `session`, other than its leader. Exited
/// processes waiting to be reaped are not alive.
#[cfg(unix)]
fn live_session_members(session: libc::pid_t) -> std::io::Result<Vec<libc::pid_t>> {
    Ok(all_pids()?
        .into_iter()
        // SAFETY: getsid only reads the session id of a pid.
        .filter(|&pid| pid != session && unsafe { libc::getsid(pid) } == session)
        .filter(|&pid| !is_zombie(pid))
        .collect())
}

#[cfg(target_os = "macos")]
fn all_pids() -> std::io::Result<Vec<libc::pid_t>> {
    // SAFETY: a null buffer asks only for the number of pids.
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if count < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // Room for processes started between the two calls.
    let mut pids = vec![0 as libc::pid_t; count as usize + 64];
    // SAFETY: the buffer is valid for its full byte size; the call writes at
    // most that many bytes and returns the number of pids written.
    let written = unsafe {
        libc::proc_listallpids(
            pids.as_mut_ptr().cast(),
            std::mem::size_of_val(pids.as_slice()) as libc::c_int,
        )
    };
    if written < 0 {
        return Err(std::io::Error::last_os_error());
    }
    pids.truncate(written as usize);
    Ok(pids)
}

#[cfg(target_os = "macos")]
fn is_zombie(pid: libc::pid_t) -> bool {
    // SAFETY: a zeroed proc_bsdinfo is a valid out-parameter.
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    // SAFETY: proc_pidinfo writes at most `size` bytes into `info`.
    let written = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size,
        )
    };
    // Unreadable counts as alive, so the caller looks again rather than
    // declaring the session empty.
    written == size && info.pbi_status == libc::SZOMB
}

#[cfg(not(target_os = "macos"))]
#[cfg(unix)]
fn all_pids() -> std::io::Result<Vec<libc::pid_t>> {
    Ok(std::fs::read_dir("/proc")?
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
        .collect())
}

#[cfg(not(target_os = "macos"))]
#[cfg(unix)]
fn is_zombie(pid: libc::pid_t) -> bool {
    // The state follows the last ')' of /proc/<pid>/stat, past the command.
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
        stat.rsplit_once(')')
            .is_some_and(|(_, rest)| rest.trim_start().starts_with('Z'))
    })
}

/// Windows kills only the child; capture through files means a surviving
/// descendant cannot block the caller.
#[cfg(windows)]
fn kill_tree(child: &mut Child) -> std::io::Result<()> {
    match child.try_wait() {
        Ok(Some(_)) => Ok(()),
        _ => child.kill(),
    }
}

/// Stop a child that ran past `limit`: kill it with `kill`, then reap it,
/// waiting at most `reap_limit` so a failed kill never holds the caller. Only
/// a kill that succeeded followed by a reap is `TimedOut`.
fn stop(
    mut child: Child,
    what: &str,
    limit: Duration,
    kill: fn(&mut Child) -> std::io::Result<()>,
    reap_limit: Duration,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
) -> RunError {
    let killed = kill(&mut child);
    let reap_deadline = Instant::now() + reap_limit;
    let reaped = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < reap_deadline => std::thread::sleep(MAX_POLL),
            Ok(None) => break Err(format!("still running {reap_limit:?} after the kill")),
            Err(error) => break Err(format!("wait failed: {error}")),
        }
    };
    let cause = match (killed, reaped) {
        (Ok(()), Ok(status)) => {
            log::warn!("distribution: {what} exceeded {limit:?}; killed and reaped ({status})");
            return RunError::TimedOut {
                what: what.to_string(),
                limit,
                stdout,
                stderr,
            };
        }
        (Err(error), Ok(status)) => {
            format!("killing it failed: {error}; it then exited ({status})")
        }
        (Err(error), Err(reap)) => format!("killing it failed: {error}; {reap}"),
        (Ok(()), Err(reap)) => reap,
    };
    log::error!("distribution: {what} exceeded {limit:?} and could not be stopped: {cause}");
    RunError::NotStopped {
        what: what.to_string(),
        cause: format!("it exceeded {limit:?}; {cause}"),
    }
}

/// Read up to `cap` bytes of `path` from offset 0 through a handle of its
/// own, so the read never shares an offset with a process still writing.
/// The flag is true when the file held more than `cap` bytes.
fn read_capped(path: &Path, cap: u64) -> std::io::Result<(Vec<u8>, bool)> {
    #[cfg(unix)]
    use std::os::unix::fs::FileExt;
    #[cfg(windows)]
    use std::os::windows::fs::FileExt;
    let file = std::fs::File::open(path)?;
    // One byte past the cap tells an overflow from a capture that fits.
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
    let over = bytes.len() as u64 > cap;
    bytes.truncate(cap as usize);
    Ok((bytes, over))
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
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::transaction::process_alive;

    fn sh(script: &str) -> Helper {
        let mut helper = Helper::new("/bin/sh");
        helper.args(["-c", script]);
        helper
    }

    fn pid_in(path: &Path) -> u32 {
        std::fs::read_to_string(path)
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    }

    fn wait_for_file(path: &Path) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while std::fs::read_to_string(path).map_or(true, |s| !s.ends_with('\n')) {
            assert!(
                Instant::now() < deadline,
                "{} never written",
                path.display()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn dies_within(pid: u32, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        while process_alive(pid) {
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        true
    }

    fn kill(pid: u32) {
        // SAFETY: test cleanup of a process this test started.
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
    }

    /// The child leads a new session, so it has no controlling terminal and
    /// leads its own process group; a login shell run here cannot touch the
    /// caller's terminal. Ids are read while the child still runs.
    #[test]
    fn the_child_leads_its_own_session() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("child");
        let script = format!("echo $$ > '{}'; sleep 2", file.display());
        let run =
            std::thread::spawn(move || sh(&script).run("session probe", Duration::from_secs(10)));
        wait_for_file(&file);
        let child = pid_in(&file) as libc::pid_t;
        // SAFETY: getsid and getpgid only read ids.
        let (session, group, ours) =
            unsafe { (libc::getsid(child), libc::getpgid(child), libc::getsid(0)) };
        run.join().unwrap().unwrap();
        assert_eq!(session, child, "child leads a new session");
        assert_eq!(group, child, "child leads its process group");
        assert_ne!(session, ours, "child left the caller's session");
    }

    /// Stint 1610: a background job the child started dies when the child
    /// exits, so nothing keeps writing into the capture after the run.
    #[test]
    fn a_background_job_dies_with_the_child() {
        let dir = tempfile::tempdir().unwrap();
        let sleeper = dir.path().join("sleeper");
        let started = Instant::now();
        let output = sh(&format!(
            "sleep 600 & echo $! > '{}'; echo done",
            sleeper.display()
        ))
        .run("test child", Duration::from_secs(30))
        .unwrap();
        let sleeper = pid_in(&sleeper);
        let died = dies_within(sleeper, Duration::from_secs(5));
        kill(sleeper);
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "done");
        assert!(died, "background job {sleeper} outlived the child");
    }

    /// Stint 1610: a descendant that moved to its own process group inside
    /// the session dies with the child; the session ends, not just the group.
    #[test]
    fn a_descendant_in_another_group_dies_with_the_child() {
        let dir = tempfile::tempdir().unwrap();
        let mover = dir.path().join("mover");
        sh(&format!(
            "perl -e 'setpgrp(0, 0); sleep 600' & echo $! > '{}'; sleep 0.3",
            mover.display()
        ))
        .run("test child", Duration::from_secs(30))
        .unwrap();
        let mover = pid_in(&mover);
        let died = dies_within(mover, Duration::from_secs(5));
        kill(mover);
        assert!(
            died,
            "descendant {mover} in its own group outlived the child"
        );
    }

    /// Stint 1610: the limit is checked before an exit is accepted, so an
    /// exit seen after it is a timeout; a wait never runs past the limit.
    #[test]
    fn the_limit_is_checked_before_an_exit_is_accepted() {
        let now = Instant::now();
        let pause = Duration::from_millis(20);
        assert_eq!(next_step(now, now, true, pause), Step::Stop);
        assert_eq!(next_step(now + pause, now, true, pause), Step::Stop);
        assert_eq!(
            next_step(now, now + Duration::from_secs(1), true, pause),
            Step::Finish
        );
        assert_eq!(
            next_step(now, now + Duration::from_millis(5), false, pause),
            Step::Wait(Duration::from_millis(5))
        );
        assert_eq!(
            next_step(now, now + Duration::from_secs(1), false, pause),
            Step::Wait(pause)
        );
    }

    /// With no time at all, even a child that exits at once is a timeout.
    #[test]
    fn a_zero_limit_never_succeeds() {
        let error = sh("exit 0").run("test child", Duration::ZERO).unwrap_err();
        assert!(matches!(error, RunError::TimedOut { .. }), "{error}");
    }

    /// A process that left the session by calling setsid itself, as the host
    /// does under `host start`, survives the child's exit.
    #[test]
    fn a_process_that_detached_survives_the_child() {
        let dir = tempfile::tempdir().unwrap();
        let daemon = dir.path().join("daemon");
        sh(&format!(
            "perl -MPOSIX -e 'POSIX::setsid(); sleep 600' & echo $! > '{}'; sleep 0.3",
            daemon.display()
        ))
        .run("test child", Duration::from_secs(30))
        .unwrap();
        let daemon = pid_in(&daemon);
        std::thread::sleep(Duration::from_millis(300));
        let alive = process_alive(daemon);
        kill(daemon);
        assert!(alive, "detached process {daemon} was killed");
    }

    /// Stint 1610: output of exactly the cap is whole; one byte more is an
    /// error, never cut bytes.
    #[test]
    fn output_past_the_cap_is_an_error() {
        let fits = sh(&format!("head -c {CAPTURE_LIMIT} /dev/zero"))
            .run("test child", Duration::from_secs(30))
            .unwrap();
        assert_eq!(fits.stdout.len() as u64, CAPTURE_LIMIT);
        let error = sh(&format!("head -c {} /dev/zero", CAPTURE_LIMIT + 1))
            .run("test child", Duration::from_secs(30))
            .unwrap_err();
        assert!(
            matches!(
                error,
                RunError::OutputTooLarge {
                    stream: "stdout",
                    ..
                }
            ),
            "{error}"
        );
    }

    /// A non-zero exit is `Failed` with its stderr from `run`, and Ok with
    /// the code from `run_any_exit`.
    #[test]
    fn a_failed_exit_is_an_error_unless_the_caller_reads_the_code() {
        let error = sh("echo why >&2; exit 3")
            .run("test child", Duration::from_secs(10))
            .unwrap_err();
        let text = error.to_string();
        assert!(
            matches!(&error, RunError::Failed { status, .. } if status.code() == Some(3)),
            "{text}"
        );
        assert!(text.contains("why"), "{text}");
        let exited = sh("echo why >&2; exit 3")
            .run_any_exit("test child", Duration::from_secs(10))
            .unwrap();
        assert_eq!(exited.status.code(), Some(3));
        assert_eq!(exited.output.stderr, b"why\n");
    }

    /// Stint 1610: git never prompts. Every prompt route is shut.
    #[test]
    fn git_runs_with_every_prompt_route_shut() {
        let helper = git();
        let envs: std::collections::HashMap<_, _> = helper
            .get_envs()
            .map(|(key, value)| (key.to_owned(), value.map(|v| v.to_owned())))
            .collect();
        let env = |key: &str| envs.get(OsStr::new(key)).cloned();
        assert_eq!(env("GIT_TERMINAL_PROMPT"), Some(Some("0".into())));
        assert_eq!(env("GIT_ASKPASS"), Some(Some("".into())));
        assert_eq!(env("SSH_ASKPASS_REQUIRE"), Some(Some("never".into())));
        assert_eq!(env("SSH_ASKPASS"), Some(None));
    }

    /// Stint 1610: a git auth failure says what failed and how to set up
    /// credentials; other failures carry no auth advice.
    #[test]
    fn a_git_auth_failure_says_how_to_set_up_credentials() {
        let failure = |stderr: &str| {
            sh(&format!("echo \"{stderr}\" >&2; exit 128"))
                .run(
                    "git clone https://example.invalid/r.git",
                    Duration::from_secs(10),
                )
                .unwrap_err()
        };
        let auth = git_error(&failure(
            "fatal: could not read Username for https://example.invalid: terminal prompts disabled",
        ));
        assert!(
            auth.starts_with("git clone https://example.invalid/r.git failed"),
            "{auth}"
        );
        assert!(auth.contains("gh auth login"), "{auth}");
        assert!(auth.contains("ssh key"), "{auth}");
        let other = git_error(&failure("error: pathspec v9 did not match"));
        assert!(!other.contains("gh auth"), "{other}");
    }

    /// A child the kill cannot stop does not hold the run: the reap gives up
    /// at its own bound and the error says the child may still be running.
    /// The kill is stubbed to fail, as a real one rarely does.
    #[test]
    fn a_failed_kill_returns_instead_of_waiting_forever() {
        let child = Command::new("/bin/sh")
            .args(["-c", "sleep 600"])
            .spawn()
            .unwrap();
        let pid = child.id();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let refuse = |_: &mut Child| Err(std::io::Error::other("refused"));
            let error = stop(
                child,
                "test child",
                Duration::from_millis(1),
                refuse,
                Duration::from_millis(300),
                Vec::new(),
                Vec::new(),
            );
            let _ = tx.send(error);
        });
        let error = rx.recv_timeout(Duration::from_secs(10));
        kill(pid);
        let error = error.expect("stop returned within its reap bound");
        let text = error.to_string();
        assert!(matches!(error, RunError::NotStopped { .. }), "{text}");
        assert!(text.contains("refused"), "{text}");
        assert!(text.contains("may still be running"), "{text}");
    }

    /// Stint 1610: a refused kill stays `NotStopped` even when the child then
    /// exits by itself: its descendants may still be alive.
    #[test]
    fn a_failed_kill_is_not_stopped_even_if_the_child_then_exits() {
        let child = Command::new("/bin/sh")
            .args(["-c", "sleep 0.2"])
            .spawn()
            .unwrap();
        let refuse = |_: &mut Child| Err(std::io::Error::other("refused"));
        let error = stop(
            child,
            "test child",
            Duration::from_millis(1),
            refuse,
            Duration::from_secs(5),
            Vec::new(),
            Vec::new(),
        );
        let text = error.to_string();
        assert!(matches!(error, RunError::NotStopped { .. }), "{text}");
        assert!(text.contains("refused"), "{text}");
    }

    /// A hung child and its descendants are killed at the limit and reaped,
    /// and what it wrote before the hang is in the error.
    #[test]
    fn a_hung_child_and_its_descendants_are_killed_at_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let (child, sleeper) = (dir.path().join("child"), dir.path().join("sleeper"));
        let started = Instant::now();
        let error = sh(&format!(
            "echo $$ > '{}'; sleep 600 & echo $! > '{}'; echo partial; wait",
            child.display(),
            sleeper.display()
        ))
        .run("test child", Duration::from_millis(500))
        .unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(10), "{error}");
        let RunError::TimedOut { stdout, .. } = &error else {
            panic!("expected TimedOut: {error}");
        };
        assert_eq!(stdout, b"partial\n");
        let (child, sleeper) = (pid_in(&child), pid_in(&sleeper));
        let sleeper_died = dies_within(sleeper, Duration::from_secs(5));
        kill(sleeper);
        assert!(!process_alive(child), "child {child} survived");
        assert!(sleeper_died, "descendant {sleeper} survived");
    }
}
