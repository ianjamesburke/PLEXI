//! The terminal session of one pane, and ending it (stint 1650).
//!
//! The pane's shell is a session leader (`pane_pty` `setsid`s it), so its
//! pid is also its session id `S`. Job control puts each job in its own
//! process group inside `S`, so ending the shell's group alone leaves every
//! job running. Ending the session hangs up every group in `S` and
//! escalates to SIGKILL after a bounded grace, then reaps the shell.
//!
//! What a signal can reach: the shell stays an unreaped zombie until the
//! end, so the numbers `S` and group `S` cannot be reused and `kill(-S)` is
//! bound to this pane. A process leaves a session only by `setsid`, into a
//! new session named after itself, and never returns; so a group read before
//! its member's session is confirmed as `S` was a group of `S`, and a group
//! never changes session. On Linux each process is signalled through a
//! pidfd opened before its session is read, so a reused pid is never hit.
//! macOS has no process handle: a job group that empties and whose number
//! the pid allocator wraps around to (`PID_MAX` 99999) between the scan and
//! the `kill` would be hit. The scan is followed by the signal at once.
use std::time::{Duration, Instant};

use super::PtyExitStatus;

/// What a hung-up program gets to save state and exit on its own.
const HANGUP_GRACE: Duration = Duration::from_millis(500);
/// Longest wait to observe SIGKILLed processes gone.
const KILL_WAIT: Duration = Duration::from_secs(1);
const POLL: Duration = Duration::from_millis(10);

pub(super) struct Session {
    pane: u64,
    sid: libc::pid_t,
}

impl Session {
    pub(super) fn new(pane: u64, sid: libc::pid_t) -> Self {
        Self { pane, sid }
    }

    pub(super) fn pane(&self) -> u64 {
        self.pane
    }

    pub(super) fn pid(&self) -> u32 {
        self.sid as u32
    }

    /// The shell's exit status once it has exited, read with `WNOWAIT` so
    /// the shell stays waitable (and its pid pinned) until the session ends.
    pub(super) fn exit_status(&self) -> Option<PtyExitStatus> {
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let rc = unsafe {
            libc::waitid(
                libc::P_PID,
                self.sid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if rc != 0 || unsafe { info.si_pid() } != self.sid {
            return None;
        }
        let status = unsafe { info.si_status() };
        Some(match info.si_code {
            libc::CLD_EXITED => PtyExitStatus::Code(status),
            libc::CLD_KILLED | libc::CLD_DUMPED => {
                PtyExitStatus::Signal(status)
            },
            _ => PtyExitStatus::Unknown,
        })
    }
}

/// Runs where the last handle drops: the PTY reader thread on a close, the
/// backend's close thread after a natural exit. Never the UI thread.
impl Drop for Session {
    fn drop(&mut self) {
        let (pane, sid) = (self.pane, self.sid);
        let groups = members(sid).map(|members| {
            let mut groups = members;
            groups.sort_unstable();
            groups.dedup();
            groups
        });
        let outcome = if !live(sid) {
            "already empty"
        } else {
            signal_session(sid, libc::SIGHUP);
            // A stopped job only acts on its hangup once continued, as the
            // kernel does for an orphaned stopped group.
            signal_session(sid, libc::SIGCONT);
            if wait_empty(sid, HANGUP_GRACE) {
                "ended on SIGHUP"
            } else {
                signal_session(sid, libc::SIGKILL);
                if wait_empty(sid, KILL_WAIT) {
                    "escalated to SIGKILL"
                } else {
                    log::error!(
                        "egui_term: pane {pane} session {sid} survived SIGKILL: {:?}",
                        members(sid)
                    );
                    "survived SIGKILL"
                }
            }
        };
        let reaped = reap(sid);
        log::info!(
            "egui_term: pane {pane} closed: session={sid} groups={groups:?} \
             outcome={outcome} master_fd_closed=true shell_reaped={reaped}"
        );
    }
}

/// Reap the shell without blocking: it is dead (or a zombie) unless it
/// survived SIGKILL, which is logged above.
fn reap(sid: libc::pid_t) -> bool {
    loop {
        let rc =
            unsafe { libc::waitpid(sid, std::ptr::null_mut(), libc::WNOHANG) };
        if rc == sid {
            return true;
        }
        let error = std::io::Error::last_os_error();
        if rc < 0 && error.raw_os_error() == Some(libc::EINTR) {
            continue;
        }
        if rc < 0 {
            log::error!("egui_term: reaping shell {sid} failed: {error}");
        }
        return false;
    }
}

/// Whether `S` has a live member. An incomplete listing counts as live, so
/// escalation is never skipped on a read failure.
fn live(sid: libc::pid_t) -> bool {
    match members(sid) {
        Ok(members) => !members.is_empty(),
        Err(error) => {
            log::error!(
                "egui_term: cannot list session {sid} members: {error}"
            );
            true
        },
    }
}

fn wait_empty(sid: libc::pid_t, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        if !live(sid) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(POLL);
    }
}

fn send(target: libc::pid_t, signal: libc::c_int) {
    if unsafe { libc::kill(target, signal) } != 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            log::warn!(
                "egui_term: signal {signal} to {target} failed: {error}"
            );
        }
    }
}

#[cfg(target_os = "macos")]
fn signal_session(sid: libc::pid_t, signal: libc::c_int) {
    send(-sid, signal);
    let groups = match members(sid) {
        Ok(groups) => groups,
        Err(error) => {
            log::error!(
                "egui_term: cannot list session {sid} members: {error}"
            );
            return;
        },
    };
    let mut sent = vec![sid];
    for pgid in groups {
        if !sent.contains(&pgid) {
            send(-pgid, signal);
            sent.push(pgid);
        }
    }
}

/// The group of each live (non-zombie) member of session `sid`. Each
/// member's group is read before its session is confirmed (module docs).
#[cfg(target_os = "macos")]
fn members(sid: libc::pid_t) -> std::io::Result<Vec<libc::pid_t>> {
    let estimate = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if estimate < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // Headroom for processes started between the two calls.
    let mut pids = vec![0 as libc::pid_t; estimate as usize + 64];
    let bytes =
        (pids.len() * std::mem::size_of::<libc::pid_t>()) as libc::c_int;
    let count =
        unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
    if count < 0 {
        return Err(std::io::Error::last_os_error());
    }
    pids.truncate(count as usize);
    Ok(pids
        .into_iter()
        .filter(|pid| *pid > 0)
        .filter_map(|pid| {
            let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
            let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
            let written = unsafe {
                libc::proc_pidinfo(
                    pid,
                    libc::PROC_PIDTBSDINFO,
                    0,
                    (&mut info as *mut libc::proc_bsdinfo).cast(),
                    size,
                )
            };
            // A zombie has no task left, so the call fails for it.
            let live = written == size && info.pbi_status != libc::SZOMB;
            (live && unsafe { libc::getsid(pid) } == sid)
                .then_some(info.pbi_pgid as libc::pid_t)
        })
        .collect())
}

#[cfg(not(target_os = "macos"))]
fn signal_session(sid: libc::pid_t, signal: libc::c_int) {
    use std::os::fd::{FromRawFd, OwnedFd};
    send(-sid, signal);
    let result = for_each_process(|pid| {
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        if fd < 0 {
            return Ok(()); // exited since listed
        }
        let pidfd = unsafe { OwnedFd::from_raw_fd(fd as libc::c_int) };
        // Read after the pidfd binds the process: if the pid was reused in
        // between, the pidfd names the dead process and the signal fails.
        if let Some(stat) = read_stat(pid)? {
            if stat.state != b'Z' && stat.session == sid {
                use std::os::fd::AsRawFd;
                let rc = unsafe {
                    libc::syscall(
                        libc::SYS_pidfd_send_signal,
                        pidfd.as_raw_fd(),
                        signal,
                        std::ptr::null::<libc::siginfo_t>(),
                        0,
                    )
                };
                if rc < 0 {
                    let error = std::io::Error::last_os_error();
                    if error.raw_os_error() != Some(libc::ESRCH) {
                        log::warn!("egui_term: signal {signal} to {pid} failed: {error}");
                    }
                }
            }
        }
        Ok(())
    });
    if let Err(error) = result {
        log::error!("egui_term: cannot signal session {sid} members: {error}");
    }
}

#[cfg(not(target_os = "macos"))]
fn members(sid: libc::pid_t) -> std::io::Result<Vec<libc::pid_t>> {
    let mut members = Vec::new();
    for_each_process(|pid| {
        if let Some(stat) = read_stat(pid)? {
            if stat.state != b'Z' && stat.session == sid {
                members.push(stat.pgrp);
            }
        }
        Ok(())
    })?;
    Ok(members)
}

#[cfg(not(target_os = "macos"))]
fn for_each_process(
    mut visit: impl FnMut(libc::pid_t) -> std::io::Result<()>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir("/proc")? {
        if let Some(pid) =
            entry?.file_name().to_str().and_then(|n| n.parse().ok())
        {
            visit(pid)?;
        }
    }
    Ok(())
}

/// `None` when the process exited since it was listed. Any other failure,
/// unreadable or unparseable, is an error: the listing is incomplete.
#[cfg(not(target_os = "macos"))]
fn read_stat(pid: libc::pid_t) -> std::io::Result<Option<Stat>> {
    match std::fs::read(format!("/proc/{pid}/stat")) {
        Ok(stat) => parse_stat(&stat).map(Some),
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound
                || error.raw_os_error() == Some(libc::ESRCH) =>
        {
            Ok(None)
        },
        Err(error) => Err(error),
    }
}

#[cfg(any(test, not(target_os = "macos")))]
#[derive(Debug, PartialEq)]
struct Stat {
    state: u8,
    pgrp: libc::pid_t,
    session: libc::pid_t,
}

/// Parse `/proc/<pid>/stat` (proc_pid_stat(5)): `pid (comm) state ppid pgrp
/// session ...`. `comm` is arbitrary bytes, spaces and parentheses
/// included, so the fields start after the last `)`. A record that does not
/// parse is an error, never a missing process.
#[cfg(any(test, not(target_os = "macos")))]
fn parse_stat(stat: &[u8]) -> std::io::Result<Stat> {
    let invalid = || {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "unparseable /proc stat {:?}",
                String::from_utf8_lossy(stat)
            ),
        )
    };
    let close = stat.iter().rposition(|b| *b == b')').ok_or_else(invalid)?;
    let mut fields = stat[close + 1..]
        .split(|b| b.is_ascii_whitespace())
        .filter(|field| !field.is_empty());
    let state = match fields.next() {
        Some([state]) => *state,
        _ => return Err(invalid()),
    };
    let mut number = || -> std::io::Result<libc::pid_t> {
        std::str::from_utf8(fields.next().ok_or_else(invalid)?)
            .ok()
            .and_then(|field| field.parse().ok())
            .ok_or_else(invalid)
    };
    let _ppid = number()?;
    let pgrp = number()?;
    let session = number()?;
    Ok(Stat {
        state,
        pgrp,
        session,
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_stat, Stat};

    #[test]
    fn stat_with_a_non_utf8_name_with_spaces_and_parens_parses() {
        let stat = b"4242 (a) \xff\xfe (b) S 1 4240 4100 34816 4240 4194304 0";
        assert_eq!(
            parse_stat(stat).unwrap(),
            Stat {
                state: b'S',
                pgrp: 4240,
                session: 4100
            }
        );
    }

    #[test]
    fn truncated_stat_is_an_error_not_a_missing_process() {
        assert!(parse_stat(b"4242 (sh) S 1 4240").is_err());
        assert!(parse_stat(b"4242 (sh").is_err());
    }
}
