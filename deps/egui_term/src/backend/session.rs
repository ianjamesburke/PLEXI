//! Ending the terminal session of a closed pane (stint 1650).
//!
//! The pane's shell is a session leader (alacritty `setsid`s it), so its pid
//! is also its session id. Job control puts each job in its own process group
//! inside that session, so signalling the shell's group alone leaves every
//! job running. Teardown therefore closes the master, then hangs up every
//! group in the session and escalates to SIGKILL after a bounded grace.
//!
//! The master must close first. alacritty's `Pty::drop` SIGHUPs the shell pid
//! and then blocks in `wait` with the master still open, while XNU makes an
//! exiting session leader wait for its tty output to drain. With no reader
//! left the two deadlock and the whole tree outlives the pane.
use std::collections::BTreeSet;
use std::os::fd::{AsRawFd, RawFd};
use std::time::{Duration, Instant};

/// What a hung-up program gets to save state and exit on its own.
const HANGUP_GRACE: Duration = Duration::from_millis(500);
/// Longest wait to observe SIGKILLed processes gone.
const KILL_WAIT: Duration = Duration::from_secs(1);
const POLL: Duration = Duration::from_millis(10);

/// End the session `sid` of pane `pane` after releasing `master_fds`. Runs on
/// the pane's PTY reader thread and is bounded by the grace and kill wait.
pub(super) fn end(pane: u64, sid: libc::pid_t, master_fds: &[RawFd]) {
    let master_released = master_fds.iter().all(|fd| release(*fd));
    let groups = session_groups(sid);
    let outcome = if groups.is_empty() {
        "already empty"
    } else {
        signal(&groups, libc::SIGHUP);
        // A stopped job only acts on its hangup once continued, as the
        // kernel does for an orphaned stopped group.
        signal(&groups, libc::SIGCONT);
        if wait_empty(sid, HANGUP_GRACE) {
            "ended on SIGHUP"
        } else {
            signal(&session_groups(sid), libc::SIGKILL);
            if wait_empty(sid, KILL_WAIT) {
                "escalated to SIGKILL"
            } else {
                log::error!(
                    "egui_term: pane {pane} session {sid} groups {:?} survived SIGKILL",
                    session_groups(sid)
                );
                "survived SIGKILL"
            }
        }
    };
    log::info!(
        "egui_term: pane {pane} closed: session={sid} groups={groups:?} outcome={outcome} \
         master_fd_closed={master_released}"
    );
}

/// Drop this process's reference to the PTY master behind `fd`. alacritty's
/// `Pty` still owns the number and closes it later, so a plain `close` would
/// let that later close hit a number another thread has reused. `dup2`
/// swaps `/dev/null` in atomically instead.
fn release(fd: RawFd) -> bool {
    let null = match std::fs::File::open("/dev/null") {
        Ok(null) => null,
        Err(error) => {
            log::error!("egui_term: cannot open /dev/null to release pty master fd {fd}: {error}");
            return false;
        },
    };
    if unsafe { libc::dup2(null.as_raw_fd(), fd) } < 0 {
        log::error!(
            "egui_term: releasing pty master fd {fd} failed: {}",
            std::io::Error::last_os_error()
        );
        return false;
    }
    // `dup2` clears close-on-exec; the master fd had it set.
    unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
    true
}

fn signal(groups: &BTreeSet<libc::pid_t>, signal: libc::c_int) {
    for group in groups {
        if unsafe { libc::kill(-group, signal) } != 0 {
            let error = std::io::Error::last_os_error();
            // The group emptied since it was listed.
            if error.raw_os_error() != Some(libc::ESRCH) {
                log::warn!("egui_term: signal {signal} to process group {group} failed: {error}");
            }
        }
    }
}

fn wait_empty(sid: libc::pid_t, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        if session_groups(sid).is_empty() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(POLL);
    }
}

/// Process groups with a live (non-zombie) member in session `sid`. A group
/// never spans sessions, so signalling these reaches nothing outside it. If
/// the process table cannot be read, falls back to the shell's own group.
fn session_groups(sid: libc::pid_t) -> BTreeSet<libc::pid_t> {
    let own = unsafe { libc::getpgrp() };
    match live_members(sid) {
        Ok(groups) => {
            groups.into_iter().filter(|g| *g > 1 && *g != own).collect()
        },
        Err(error) => {
            log::error!(
                "egui_term: cannot list session {sid} members: {error}"
            );
            BTreeSet::from([sid])
        },
    }
}

#[cfg(target_os = "macos")]
fn live_members(sid: libc::pid_t) -> std::io::Result<Vec<libc::pid_t>> {
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
        .filter(|pid| *pid > 0 && unsafe { libc::getsid(*pid) } == sid)
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
            (written == size && info.pbi_status != libc::SZOMB)
                .then_some(info.pbi_pgid as libc::pid_t)
        })
        .collect())
}

#[cfg(not(target_os = "macos"))]
fn live_members(sid: libc::pid_t) -> std::io::Result<Vec<libc::pid_t>> {
    let mut groups = Vec::new();
    for entry in std::fs::read_dir("/proc")? {
        let Some(pid) = entry?
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        // proc(5): `pid (comm) state ppid pgrp session ...`; comm may hold
        // spaces and parentheses, so fields start after the last `)`.
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        else {
            continue; // exited since listed
        };
        let Some((_, fields)) = stat.rsplit_once(')') else {
            continue;
        };
        let fields: Vec<&str> = fields.split_whitespace().collect();
        if let [state, _ppid, pgrp, session, ..] = fields[..] {
            if state != "Z" && session.parse() == Ok(sid) {
                if let Ok(pgrp) = pgrp.parse() {
                    groups.push(pgrp);
                }
            }
        }
    }
    Ok(groups)
}
