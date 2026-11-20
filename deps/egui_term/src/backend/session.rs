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
//! closing shell. A close registers with it at once ([`CloseOnDrop`]), so
//! host stop always sees it. Once the pty has closed its masters, the
//! worker hangs up `-S` and sends SIGKILL to `-S` after a grace (a leader
//! that already exited does not cancel that: other members may remain). It
//! is the only reaper, reaping once after the last signal. Any wait error
//! means the pin is lost, and the shell is never signalled again. After
//! host stop ([`finish_pane_closes`]) the worker signals and reaps nothing.
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::PtyExitStatus;

/// What a hung-up shell gets to save state and exit on its own.
const HANGUP_GRACE: Duration = Duration::from_millis(500);
/// When a shell still not reapable after SIGKILL is logged as pending.
const KILL_WAIT: Duration = Duration::from_secs(1);
const POLL: Duration = Duration::from_millis(20);

pub(super) enum Job {
    Close(Arc<Session>),
    Finish {
        deadline: Instant,
        done: Sender<Vec<u32>>,
    },
}

/// The pane's shell: a direct child that called `setsid`, not yet reaped.
pub(super) struct Session {
    pane: u64,
    sid: libc::pid_t,
    /// Set by the pty once both masters are closed. No signal before that.
    masters_closed: AtomicBool,
    guard: Mutex<Option<Box<dyn Send>>>,
}

impl Session {
    pub(super) fn new(pane: u64, sid: libc::pid_t) -> Self {
        Self {
            pane,
            sid,
            masters_closed: AtomicBool::new(false),
            guard: Mutex::new(None),
        }
    }

    pub(super) fn pid(&self) -> u32 {
        self.sid as u32
    }

    pub(super) fn masters_closed(&self) {
        self.masters_closed.store(true, Ordering::Release);
    }

    /// Keep `guard` alive until the shell is reaped.
    pub(super) fn hold_until_reaped(&self, guard: Box<dyn Send>) {
        *self.guard.lock().unwrap_or_else(|e| e.into_inner()) = Some(guard);
    }

    /// The shell's exit status once it has exited, read with `WNOWAIT` so
    /// it stays waitable, and its pid pinned, until the worker reaps it.
    /// An error means the pin is lost.
    pub(super) fn exit_status(&self) -> io::Result<Option<PtyExitStatus>> {
        exit_status(self.sid)
    }
}

/// Hands the shell to the worker when dropped: the backend's close, or a
/// constructor that failed after the shell was spawned. Only sends on a
/// channel, so it never waits.
pub(super) struct CloseOnDrop {
    pub(super) session: Arc<Session>,
    pub(super) worker: Sender<Job>,
}

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        if self.worker.send(Job::Close(self.session.clone())).is_err() {
            log::error!(
                "egui_term: pane {} shell {} left unreaped: the lifecycle worker is gone",
                self.session.pane,
                self.session.sid
            );
        }
    }
}

static WORKER: Mutex<Option<Sender<Job>>> = Mutex::new(None);
/// Set by host stop. Every signal and reap holds this lock and checks it,
/// so once it is set the worker can no longer release or signal any pid.
static STOPPED: Mutex<bool> = Mutex::new(false);

/// The lifecycle worker, started on first use. A shell is only spawned
/// once this succeeds, so every shell has an owner before it exists.
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

/// For host stop: escalate every closing shell now and reap what can be
/// reaped by `deadline`, then stop the worker for good, so the sweep that
/// follows only meets pinned pids. Returns the shells left unreaped.
pub fn finish_pane_closes(deadline: Instant) -> Vec<u32> {
    let worker = WORKER.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let mut left = Vec::new();
    if let Some(worker) = worker {
        let (done, finished) = mpsc::channel();
        if worker.send(Job::Finish { deadline, done }).is_ok() {
            // The worker answers at its deadline; the margin covers one pass.
            let limit = deadline.saturating_duration_since(Instant::now())
                + Duration::from_secs(1);
            match finished.recv_timeout(limit) {
                Ok(pids) => left = pids,
                Err(error) => log::error!(
                    "egui_term: finishing pane closes got no answer ({error}); stopping the worker"
                ),
            }
        }
    }
    *STOPPED.lock().unwrap_or_else(|e| e.into_inner()) = true;
    left
}

/// Run `act` unless host stop has stopped the worker.
fn unless_stopped<T>(act: impl FnOnce() -> T) -> Option<T> {
    let stopped = STOPPED.lock().unwrap_or_else(|e| e.into_inner());
    (!*stopped).then(act)
}

struct Entry {
    session: Arc<Session>,
    /// Set when the hangup is sent; the kill is due then.
    kill_at: Option<Instant>,
    exited_on_hangup: Option<bool>,
    pending_logged: bool,
}

fn run(jobs: mpsc::Receiver<Job>) {
    let mut entries: Vec<Entry> = Vec::new();
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
            Some(Job::Close(session)) => entries.push(Entry {
                session,
                kill_at: None,
                exited_on_hangup: None,
                pending_logged: false,
            }),
            Some(Job::Finish { deadline, done }) => {
                while !entries.is_empty() && Instant::now() < deadline {
                    tick(&mut entries, true);
                    std::thread::sleep(POLL);
                }
                let left: Vec<u32> =
                    entries.iter().map(|e| e.session.pid()).collect();
                if !left.is_empty() {
                    log::warn!(
                        "egui_term: shells {left:?} not reaped by host stop; left pinned"
                    );
                }
                // Kept, never acted on again: their guards stay alive.
                *STOPPED.lock().unwrap_or_else(|e| e.into_inner()) = true;
                let _ = done.send(left);
            },
            None => {},
        }
        tick(&mut entries, false);
    }
}

fn tick(entries: &mut Vec<Entry>, finishing: bool) {
    let now = Instant::now();
    entries.retain_mut(|entry| {
        let session = entry.session.clone();
        let (pane, sid) = (session.pane, session.sid);
        if entry.kill_at.is_none() {
            if !session.masters_closed.load(Ordering::Acquire) {
                return true;
            }
            // The master is closed: the kernel has hung up the tty. Make the
            // hangup explicit for the owned group; a stopped member acts on
            // it only once continued.
            match unless_stopped(|| pinned(&session).map(|_| {
                signal(sid, libc::SIGHUP);
                signal(sid, libc::SIGCONT);
            })) {
                None => return true,
                Some(Err(())) => return false,
                Some(Ok(())) => {},
            }
            entry.kill_at = Some(now + HANGUP_GRACE);
        }
        let kill_at = entry.kill_at.unwrap_or(now);
        if entry.exited_on_hangup.is_none() {
            if now < kill_at && !finishing {
                return true;
            }
            match unless_stopped(|| pinned(&session).map(|exited| {
                signal(sid, libc::SIGKILL);
                exited
            })) {
                None => return true,
                Some(Err(())) => return false,
                Some(Ok(exited)) => entry.exited_on_hangup = Some(exited),
            }
        }
        let escalation = if entry.exited_on_hangup == Some(true) {
            "shell exited on SIGHUP; SIGKILL sent to its group"
        } else {
            "shell escalated to SIGKILL"
        };
        match unless_stopped(|| reap(sid)) {
            None => true,
            Some(Ok(Some(status))) => {
                drop(session.guard.lock().unwrap_or_else(|e| e.into_inner()).take());
                log::info!(
                    "egui_term: pane {pane} closed: pid={sid} pgid={sid} master_fd_closed=true \
                     escalation=\"{escalation}\" reap=done status={status:?}{}",
                    if entry.pending_logged { " (after pending)" } else { "" }
                );
                false
            },
            Some(Ok(None)) => {
                if !entry.pending_logged && now >= kill_at + KILL_WAIT {
                    entry.pending_logged = true;
                    log::info!(
                        "egui_term: pane {pane} closed: pid={sid} pgid={sid} master_fd_closed=true \
                         escalation=\"{escalation}\" reap=pending"
                    );
                }
                true
            },
            Some(Err(error)) => {
                log::error!("egui_term: pane {pane} shell {sid} cannot be reaped: {error}");
                false
            },
        }
    });
}

/// Whether the shell is still pinned (waitable): `Ok(exited)`. A wait
/// error means the pin is lost: logged, and nothing more is signalled.
fn pinned(session: &Session) -> Result<bool, ()> {
    session.exit_status().map(|status| status.is_some()).map_err(|error| {
        log::error!(
            "egui_term: pane {} shell {} lost its pin ({error}); no further signals",
            session.pane,
            session.sid
        );
    })
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

fn exit_status(sid: libc::pid_t) -> io::Result<Option<PtyExitStatus>> {
    #[cfg(any(test, feature = "testing"))]
    if testing::lost(sid) {
        return Err(io::Error::from_raw_os_error(libc::ECHILD));
    }
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let rc = unsafe {
        libc::waitid(
            libc::P_PID,
            sid as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if rc != 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::EINTR) {
            return exit_status(sid);
        }
        return Err(error);
    }
    if unsafe { info.si_pid() } != sid {
        return Ok(None);
    }
    let status = unsafe { info.si_status() };
    Ok(Some(match info.si_code {
        libc::CLD_EXITED => PtyExitStatus::Code(status),
        libc::CLD_KILLED | libc::CLD_DUMPED => PtyExitStatus::Signal(status),
        _ => PtyExitStatus::Unknown,
    }))
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
    /// Held, a pty pauses before closing its masters.
    pub static READER_HOLD: Mutex<()> = Mutex::new(());
    static EVENTS: Mutex<Vec<(libc::pid_t, Event)>> = Mutex::new(Vec::new());
    static PENDING: Mutex<Option<HashMap<libc::pid_t, u32>>> = Mutex::new(None);
    static LOST: Mutex<Vec<libc::pid_t>> = Mutex::new(Vec::new());

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

    pub(super) fn lost(sid: libc::pid_t) -> bool {
        LOST.lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&sid)
    }

    /// Every wait on `sid` fails with ECHILD from now on.
    pub fn lose_pin(sid: libc::pid_t) {
        LOST.lock().unwrap_or_else(|e| e.into_inner()).push(sid);
    }
}
