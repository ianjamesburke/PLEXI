//! A pane's shell, and its end (stint 1650).
//!
//! Contract: closing a pane hangs up its terminal (`pane_pty` closes the
//! master first), then signals only what the pane provably owns. That is its
//! shell `S`, a direct child kept waitable until the end (an unreaped zombie
//! once it exits), and the group `-S` the shell created with `setsid`. While
//! `S` is unreaped, neither number can be reused. Jobs the shell moved to
//! other groups get ordinary terminal hangup semantics; detached, `setsid`
//! or `nohup` work may survive by design. Nothing is found by scanning.
//!
//! One lifecycle worker, started before any shell exists, owns every
//! closing shell. It hangs up `-S`, sends SIGKILL to `-S` after a grace (a
//! leader that already exited does not cancel that: other members may
//! remain), and is the only reaper, reaping once after the last signal. A
//! shell that is not yet waitable stays on its list without delaying other
//! closes.
use std::io;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::PtyExitStatus;

/// What a hung-up shell gets to save state and exit on its own.
const HANGUP_GRACE: Duration = Duration::from_millis(500);
/// When a shell still not reapable after SIGKILL is logged as pending.
const KILL_WAIT: Duration = Duration::from_secs(1);
const POLL: Duration = Duration::from_millis(20);

pub(super) enum Job {
    Close(Closing),
    Finish {
        deadline: Instant,
        done: Sender<Vec<u32>>,
    },
}

pub(super) struct Closing {
    pane: u64,
    sid: libc::pid_t,
    /// Dropped once the shell is reaped (or never, if frozen).
    _guard: Option<Box<dyn Send>>,
}

/// The pane's shell. Shared by the pane's pty and its backend; when the
/// last handle drops, the shell is handed to the lifecycle worker. That
/// drop only sends on a channel: it never waits, so it is safe anywhere.
pub(super) struct Session {
    pane: u64,
    sid: libc::pid_t,
    worker: Sender<Job>,
    guard: Mutex<Option<Box<dyn Send>>>,
}

impl Session {
    /// `sid` must be a direct child that called `setsid`, not yet reaped.
    pub(super) fn new(
        pane: u64,
        sid: libc::pid_t,
        worker: Sender<Job>,
    ) -> Self {
        Self {
            pane,
            sid,
            worker,
            guard: Mutex::new(None),
        }
    }

    pub(super) fn pane(&self) -> u64 {
        self.pane
    }

    pub(super) fn pid(&self) -> u32 {
        self.sid as u32
    }

    /// Keep `guard` alive until the shell is reaped.
    pub(super) fn hold_until_reaped(&self, guard: Box<dyn Send>) {
        *self.guard.lock().unwrap_or_else(|e| e.into_inner()) = Some(guard);
    }

    /// The shell's exit status once it has exited, read with `WNOWAIT` so
    /// it stays waitable, and its pid pinned, until the worker reaps it.
    pub(super) fn exit_status(&self) -> Option<PtyExitStatus> {
        exit_status(self.sid)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let guard = self.guard.lock().unwrap_or_else(|e| e.into_inner()).take();
        let closing = Closing {
            pane: self.pane,
            sid: self.sid,
            _guard: guard,
        };
        if self.worker.send(Job::Close(closing)).is_err() {
            log::error!(
                "egui_term: pane {} shell {} left unreaped: the lifecycle worker is gone",
                self.pane,
                self.sid
            );
        }
    }
}

/// The lifecycle worker, started on first use. A pty is only spawned once
/// this succeeds, so every shell has an owner before it exists.
static WORKER: Mutex<Option<Sender<Job>>> = Mutex::new(None);

pub(super) fn worker() -> io::Result<Sender<Job>> {
    let mut slot = WORKER.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(worker) = slot.as_ref() {
        return Ok(worker.clone());
    }
    let (worker, jobs) = mpsc::channel();
    std::thread::Builder::new()
        .name("pty-lifecycle".to_string())
        .spawn(move || run(jobs))?;
    *slot = Some(worker.clone());
    Ok(worker)
}

/// Escalate every closing shell now and reap what can be reaped by
/// `deadline`, for host stop. Shells still unreaped are frozen: the worker
/// never reaps them, so they stay pinned for whatever cleanup follows.
/// Returns their pids.
pub fn finish_pane_closes(deadline: Instant) -> Vec<u32> {
    // No worker means no pane was ever spawned in this process.
    let Some(worker) = WORKER.lock().unwrap_or_else(|e| e.into_inner()).clone()
    else {
        return Vec::new();
    };
    let (done, finished) = mpsc::channel();
    if worker.send(Job::Finish { deadline, done }).is_err() {
        return Vec::new();
    }
    // The worker answers at its deadline; the margin covers one pass.
    let limit = deadline.saturating_duration_since(Instant::now())
        + Duration::from_secs(1);
    finished.recv_timeout(limit).unwrap_or_else(|error| {
        log::error!("egui_term: finishing pane closes got no answer: {error}");
        Vec::new()
    })
}

struct Entry {
    closing: Closing,
    kill_at: Instant,
    exited_on_hangup: Option<bool>,
    pending_logged: bool,
}

fn run(jobs: mpsc::Receiver<Job>) {
    let mut entries: Vec<Entry> = Vec::new();
    // Frozen at host stop: never reaped, guards kept.
    let mut frozen: Vec<Entry> = Vec::new();
    loop {
        let job = if entries.is_empty() {
            jobs.recv().ok()
        } else {
            match jobs.recv_timeout(POLL) {
                Ok(job) => Some(job),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        };
        #[cfg(any(test, feature = "testing"))]
        let _held = testing::HOLD.lock().unwrap_or_else(|e| e.into_inner());
        match job {
            Some(Job::Close(closing)) => {
                // The master is closed: the kernel has hung up the tty. Make
                // the hangup explicit for the owned group; a stopped member
                // acts on it only once continued.
                signal(closing.sid, libc::SIGHUP);
                signal(closing.sid, libc::SIGCONT);
                entries.push(Entry {
                    closing,
                    kill_at: Instant::now() + HANGUP_GRACE,
                    exited_on_hangup: None,
                    pending_logged: false,
                });
            },
            Some(Job::Finish { deadline, done }) => {
                for entry in &mut entries {
                    entry.kill_at = Instant::now();
                }
                while !entries.is_empty() && Instant::now() < deadline {
                    tick(&mut entries);
                    std::thread::sleep(POLL);
                }
                for entry in &entries {
                    log::warn!(
                        "egui_term: pane {} shell {} not reaped by host stop; left pinned",
                        entry.closing.pane,
                        entry.closing.sid
                    );
                }
                let pids =
                    entries.iter().map(|e| e.closing.sid as u32).collect();
                frozen.append(&mut entries);
                let _ = done.send(pids);
            },
            None => {},
        }
        tick(&mut entries);
    }
}

fn tick(entries: &mut Vec<Entry>) {
    let now = Instant::now();
    entries.retain_mut(|entry| {
        let Closing { pane, sid, .. } = entry.closing;
        if entry.exited_on_hangup.is_none() {
            if now < entry.kill_at {
                return true;
            }
            entry.exited_on_hangup = Some(exit_status(sid).is_some());
            signal(sid, libc::SIGKILL);
        }
        let escalation = if entry.exited_on_hangup == Some(true) {
            "shell exited on SIGHUP; SIGKILL sent to its group"
        } else {
            "shell escalated to SIGKILL"
        };
        match reap(sid) {
            Ok(Some(status)) => {
                log::info!(
                    "egui_term: pane {pane} closed: pid={sid} pgid={sid} master_fd_closed=true \
                     escalation=\"{escalation}\" reap=done status={status:?}{}",
                    if entry.pending_logged { " (after pending)" } else { "" }
                );
                false
            },
            Ok(None) => {
                if !entry.pending_logged && now >= entry.kill_at + KILL_WAIT {
                    entry.pending_logged = true;
                    log::info!(
                        "egui_term: pane {pane} closed: pid={sid} pgid={sid} master_fd_closed=true \
                         escalation=\"{escalation}\" reap=pending"
                    );
                }
                true
            },
            Err(error) => {
                // ECHILD and the like: the pin is lost. Signal nothing more.
                log::error!("egui_term: pane {pane} shell {sid} cannot be reaped: {error}");
                false
            },
        }
    });
}

fn signal(sid: libc::pid_t, signal: libc::c_int) {
    #[cfg(any(test, feature = "testing"))]
    testing::record(sid, testing::Event::Signal(-sid, signal));
    if sid > 1 && unsafe { libc::kill(-sid, signal) } != 0 {
        let error = io::Error::last_os_error();
        // An empty group (only the zombie leader, or nothing) is expected.
        if !matches!(
            error.raw_os_error(),
            Some(libc::ESRCH) | Some(libc::EPERM)
        ) {
            log::warn!(
                "egui_term: signal {signal} to group {sid} failed: {error}"
            );
        }
    }
}

/// Consume the shell's exit: `Ok(None)` while it is not yet waitable.
fn reap(sid: libc::pid_t) -> io::Result<Option<i32>> {
    #[cfg(any(test, feature = "testing"))]
    if testing::take_pending(sid) {
        testing::record(sid, testing::Event::ReapPending);
        return Ok(None);
    }
    loop {
        let mut status = 0;
        let rc = unsafe { libc::waitpid(sid, &mut status, libc::WNOHANG) };
        if rc == sid {
            #[cfg(any(test, feature = "testing"))]
            testing::record(sid, testing::Event::Reaped);
            return Ok(Some(status));
        }
        if rc == 0 {
            return Ok(None);
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EINTR) {
            return Err(error);
        }
    }
}

fn exit_status(sid: libc::pid_t) -> Option<PtyExitStatus> {
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let rc = unsafe {
        libc::waitid(
            libc::P_PID,
            sid as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if rc != 0 || unsafe { info.si_pid() } != sid {
        return None;
    }
    let status = unsafe { info.si_status() };
    Some(match info.si_code {
        libc::CLD_EXITED => PtyExitStatus::Code(status),
        libc::CLD_KILLED | libc::CLD_DUMPED => PtyExitStatus::Signal(status),
        _ => PtyExitStatus::Unknown,
    })
}

/// Observation and control of the worker for tests.
#[cfg(any(test, feature = "testing"))]
pub mod testing {
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Event {
        /// `kill(target, signal)` requested.
        Signal(libc::pid_t, libc::c_int),
        ReapPending,
        Reaped,
    }

    /// Held, the worker pauses before its next step.
    pub static HOLD: Mutex<()> = Mutex::new(());
    static EVENTS: Mutex<Vec<(libc::pid_t, Event)>> = Mutex::new(Vec::new());
    static PENDING: Mutex<Option<HashMap<libc::pid_t, u32>>> = Mutex::new(None);

    pub(super) fn record(sid: libc::pid_t, event: Event) {
        EVENTS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((sid, event));
    }

    /// Everything the worker did for shell `sid`, in order.
    pub fn events(sid: libc::pid_t) -> Vec<Event> {
        EVENTS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|(s, _)| *s == sid)
            .map(|(_, event)| *event)
            .collect()
    }

    pub(super) fn take_pending(sid: libc::pid_t) -> bool {
        let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
        match pending.get_or_insert_with(HashMap::new).get_mut(&sid) {
            Some(left) if *left > 0 => {
                *left -= 1;
                true
            },
            _ => false,
        }
    }

    /// The next `times` reaps of `sid` find it not yet waitable.
    pub fn set_pending(sid: libc::pid_t, times: u32) {
        PENDING
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_or_insert_with(HashMap::new)
            .insert(sid, times);
    }
}
