//! The one owner of every pane's shell, from spawn through reap (stint 1650).
//!
//! Contract: closing a pane hangs up its terminal. The pty closes both
//! masters first (`pane_pty`); then the owner sends HUP and CONT to the
//! shell's original group `-S`, allows a bounded grace, sends KILL to that
//! group, and reaps the shell once. Jobs in other groups get the platform's
//! terminal-hangup and job-control semantics; detached, `setsid` or remote
//! work may survive. A shell that exits on its own stays unreaped until its
//! pane closes, since its group can still hold jobs.
//!
//! Authority: the shell's [`std::process::Child`] moves into this module's
//! worker at spawn and never leaves it. Only the worker observes the shell
//! (`waitid` with `WNOWAIT`), signals it, and reaps it; the pane holds a
//! launch key and can only ask. Exclusive, unreaped parenthood keeps `S`
//! from being reused while the worker may signal it. A wait error ends the
//! worker's authority for that launch for good. After the final group
//! signal the owner can only be reaped. Host stop is a request to the same
//! owner ([`shutdown_panes`]): it seals admission, closes every pane, and
//! reports what is still unresolved at its deadline; the worker keeps its
//! responsibility while the host lives.
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::process::Child;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use alacritty_terminal::event_loop::{EventLoopSender, Msg};

use super::PtyExitStatus;

/// The log target for close outcomes; the host admits it at info.
pub const LOG_TARGET: &str = "egui_term::pane_lifecycle";
/// What a hung-up shell gets to save state and exit on its own.
const HANGUP_GRACE: Duration = Duration::from_millis(500);
/// Retry interval for a shell not yet waitable after its final signal.
const REAP_RETRY: Duration = Duration::from_millis(50);
/// After this, a shell still not reapable is logged as pending.
const REAP_PENDING: Duration = Duration::from_secs(1);

enum Job {
    Admit(u64, Box<Entry>),
    Close(u64),
    MastersClosed(u64),
    Sigchld,
    Shutdown {
        deadline: Instant,
        reply: Sender<ShutdownOutcome>,
    },
}

/// What host stop learned from the owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShutdownOutcome {
    /// Every shell was reaped.
    Complete,
    /// Shells still owned at the deadline, described for the log.
    Incomplete(Vec<String>),
    /// The owner did not answer by the deadline.
    Unconfirmed,
}

impl ShutdownOutcome {
    /// How much work is unresolved or unknown: 0 only when complete.
    pub fn unresolved(&self) -> usize {
        match self {
            Self::Complete => 0,
            Self::Incomplete(left) => left.len(),
            Self::Unconfirmed => 1,
        }
    }
}

/// The shell itself. Not `Clone`; only the worker holds it.
enum Owner {
    /// Unreaped and still allowed to signal its group.
    Pinned(Child),
    /// The final group signal is sent: it can only be reaped.
    ReapOnly(Child, Instant),
}

impl Owner {
    fn pid(&self) -> libc::pid_t {
        match self {
            Owner::Pinned(child) | Owner::ReapOnly(child, _) => {
                child.id() as libc::pid_t
            },
        }
    }
}

struct Entry {
    pane: u64,
    owner: Owner,
    /// Published here; the reader and the backend only read it.
    status: Arc<Mutex<Option<PtyExitStatus>>>,
    /// Nonblocking: a full socket means a wake is already pending.
    wake: UnixStream,
    reader: EventLoopSender,
    close_requested: bool,
    masters_closed: bool,
    kill_at: Option<Instant>,
    signals: Vec<String>,
}

static WORKER: Mutex<Option<Sender<Job>>> = Mutex::new(None);
/// Held across a spawn and its admission; set once host stop begins.
static SEALED: Mutex<bool> = Mutex::new(false);
static NEXT_KEY: AtomicU64 = AtomicU64::new(1);

/// The worker, started on first use; a pty only spawns once this succeeds.
pub(super) fn worker() -> io::Result<()> {
    sender().map(drop)
}

fn sender() -> io::Result<Sender<Job>> {
    let mut slot = WORKER.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(worker) = slot.as_ref() {
        return Ok(worker.clone());
    }
    let (worker, jobs) = mpsc::channel();
    // SIGCHLD arrives on a pipe; a small thread turns it into jobs.
    let (sender, mut sigchld) = UnixStream::pair()?;
    let registration =
        signal_hook::low_level::pipe::register(libc::SIGCHLD, sender)?;
    let forward = worker.clone();
    let spawned = std::thread::Builder::new()
        .name("pty-sigchld".to_string())
        .spawn(move || {
            let mut byte = [0u8; 64];
            while matches!(sigchld.read(&mut byte), Ok(n) if n > 0) {
                if forward.send(Job::Sigchld).is_err() {
                    return;
                }
            }
        });
    if let Err(error) = spawned {
        signal_hook::low_level::unregister(registration);
        return Err(error);
    }
    std::thread::Builder::new()
        .name("pty-lifecycle".to_string())
        .spawn(move || run(jobs))?;
    *slot = Some(worker.clone());
    Ok(worker)
}

/// Permission to spawn a shell, held until it is admitted. Host stop cannot
/// seal admission while one is held, so every spawned shell is owned.
pub(super) struct Admission {
    _sealed: MutexGuard<'static, bool>,
    worker: Sender<Job>,
}

pub(super) fn admit() -> io::Result<Admission> {
    let worker = sender()?;
    let sealed = SEALED.lock().unwrap_or_else(|e| e.into_inner());
    if *sealed {
        return Err(io::Error::other("the host is stopping: no new panes"));
    }
    Ok(Admission {
        _sealed: sealed,
        worker,
    })
}

impl Admission {
    /// Hand the spawned shell to the worker. `wake` is the worker's end of
    /// the reader's notification socket; `reader` shuts that reader down.
    pub(super) fn own(
        self,
        pane: u64,
        child: Child,
        status: Arc<Mutex<Option<PtyExitStatus>>>,
        wake: UnixStream,
        reader: EventLoopSender,
    ) -> LaunchKey {
        let key = NEXT_KEY.fetch_add(1, Ordering::Relaxed);
        #[cfg(any(test, feature = "testing"))]
        testing::arm_next(key);
        let entry = Entry {
            pane,
            owner: Owner::Pinned(child),
            status,
            wake,
            reader,
            close_requested: false,
            masters_closed: false,
            kill_at: None,
            signals: Vec::new(),
        };
        if self.worker.send(Job::Admit(key, Box::new(entry))).is_err() {
            log::error!(target: LOG_TARGET, "pane {pane}: the lifecycle worker is gone");
        }
        LaunchKey {
            key,
            worker: self.worker,
        }
    }
}

/// A pane's handle on its launch: it can only ask. Dropping it asks the
/// worker to close the launch.
pub(super) struct LaunchKey {
    key: u64,
    worker: Sender<Job>,
}

/// Lets the pty report that both masters are closed; carries no authority.
pub(super) struct MastersNotice {
    key: u64,
    worker: Sender<Job>,
}

impl MastersNotice {
    pub(super) fn send(&self) {
        let _ = self.worker.send(Job::MastersClosed(self.key));
    }
}

impl LaunchKey {
    pub(super) fn notice(&self) -> MastersNotice {
        MastersNotice {
            key: self.key,
            worker: self.worker.clone(),
        }
    }

    #[cfg(any(test, feature = "testing"))]
    pub(super) fn key(&self) -> u64 {
        self.key
    }
}

impl Drop for LaunchKey {
    fn drop(&mut self) {
        let _ = self.worker.send(Job::Close(self.key));
    }
}

/// For host stop: seal admission, close every pane, and collect outcomes
/// until `deadline`. Never signals anything itself.
pub fn shutdown_panes(deadline: Instant) -> ShutdownOutcome {
    let worker = WORKER.lock().unwrap_or_else(|e| e.into_inner()).clone();
    *SEALED.lock().unwrap_or_else(|e| e.into_inner()) = true;
    let Some(worker) = worker else {
        return ShutdownOutcome::Complete;
    };
    let (reply, outcome) = mpsc::channel();
    if worker.send(Job::Shutdown { deadline, reply }).is_err() {
        return ShutdownOutcome::Unconfirmed;
    }
    // The worker answers at its deadline; the margin covers one pass.
    let limit = deadline.saturating_duration_since(Instant::now())
        + Duration::from_millis(250);
    outcome
        .recv_timeout(limit)
        .unwrap_or(ShutdownOutcome::Unconfirmed)
}

fn run(jobs: mpsc::Receiver<Job>) {
    let mut entries: HashMap<u64, Entry> = HashMap::new();
    let mut stopping: Option<(Instant, Sender<ShutdownOutcome>)> = None;
    loop {
        let now = Instant::now();
        let next = entries
            .values()
            .filter_map(|entry| match entry.owner {
                Owner::ReapOnly(..) => Some(now + REAP_RETRY),
                Owner::Pinned(_) => entry.kill_at,
            })
            .chain(stopping.as_ref().map(|(deadline, _)| *deadline))
            .min();
        let job = match next {
            None => match jobs.recv() {
                Ok(job) => Some(job),
                Err(_) => return,
            },
            Some(at) => {
                match jobs.recv_timeout(at.saturating_duration_since(now)) {
                    Ok(job) => Some(job),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            },
        };
        #[cfg(any(test, feature = "testing"))]
        let _held = testing::HOLD.lock().unwrap_or_else(|e| e.into_inner());
        let shutting_down = stopping.as_ref().map(|(deadline, _)| *deadline);
        match job {
            Some(Job::Admit(key, entry)) => {
                entries.insert(key, *entry);
                // A shell that exited before admission sent its SIGCHLD
                // before it was here to be observed.
                observe(&mut entries, key);
            },
            Some(Job::Sigchld) => {
                let keys: Vec<u64> = entries.keys().copied().collect();
                for key in keys {
                    observe(&mut entries, key);
                }
            },
            Some(Job::Close(key)) => {
                if let Some(entry) = entries.get_mut(&key) {
                    entry.close_requested = true;
                }
                hang_up(&mut entries, key, shutting_down);
            },
            Some(Job::MastersClosed(key)) => {
                if let Some(entry) = entries.get_mut(&key) {
                    entry.masters_closed = true;
                }
                hang_up(&mut entries, key, shutting_down);
            },
            Some(Job::Shutdown { deadline, reply }) => {
                let keys: Vec<u64> = entries.keys().copied().collect();
                for key in keys {
                    if let Some(entry) = entries.get_mut(&key) {
                        entry.close_requested = true;
                        // The backend may never drop: end its reader so the
                        // masters close.
                        let _ = entry.reader.send(Msg::Shutdown);
                        if let Some(kill_at) = entry.kill_at.as_mut() {
                            *kill_at = (*kill_at).min(early_kill(deadline));
                        }
                    }
                    hang_up(&mut entries, key, Some(deadline));
                }
                stopping = Some((deadline, reply));
            },
            None => {},
        }
        advance(&mut entries);
        if let Some((deadline, _)) = &stopping {
            if entries.is_empty() || Instant::now() >= *deadline {
                let left: Vec<String> = entries.iter().map(describe).collect();
                if let Some((_, reply)) = stopping.take() {
                    let _ = reply.send(if left.is_empty() {
                        ShutdownOutcome::Complete
                    } else {
                        ShutdownOutcome::Incomplete(left)
                    });
                }
            }
        }
    }
}

/// During host stop the grace is cut short so the kill lands before the
/// deadline.
fn early_kill(deadline: Instant) -> Instant {
    deadline
        .checked_sub(Duration::from_millis(150))
        .unwrap_or(deadline)
        .max(Instant::now())
}

fn describe((key, entry): (&u64, &Entry)) -> String {
    let state = match (&entry.owner, entry.masters_closed, entry.kill_at) {
        (Owner::ReapOnly(..), _, _) => "killed, not yet reaped",
        (Owner::Pinned(_), false, _) => "masters still open",
        (Owner::Pinned(_), true, None) => "not closed",
        (Owner::Pinned(_), true, Some(_)) => "hung up",
    };
    format!(
        "pane {} launch {key} pid {}: {state}",
        entry.pane,
        entry.owner.pid()
    )
}

/// Observe whether the shell exited, without reaping. A status is published
/// once, then the reader is woken. A wait error retires the launch.
fn observe(entries: &mut HashMap<u64, Entry>, key: u64) {
    let Some(entry) = entries.get(&key) else {
        return;
    };
    let Owner::Pinned(_) = entry.owner else {
        return;
    };
    let published = entry
        .status
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_some();
    if published {
        return;
    }
    match probe(key, entry.owner.pid(), Site::Observe) {
        Ok(Some(status)) => publish(entry, status),
        Ok(None) => {},
        Err(error) => lose(entries, key, &error),
    }
}

/// Once the pane is closing and both masters are closed: HUP and CONT to
/// the original group, then the kill is due after the grace.
fn hang_up(
    entries: &mut HashMap<u64, Entry>,
    key: u64,
    shutting_down: Option<Instant>,
) {
    let Some(entry) = entries.get(&key) else {
        return;
    };
    if !entry.close_requested
        || !entry.masters_closed
        || entry.kill_at.is_some()
    {
        return;
    }
    if let Err(error) = probe(key, entry.owner.pid(), Site::Hangup) {
        return lose(entries, key, &error);
    }
    let Some(entry) = entries.get_mut(&key) else {
        return;
    };
    signal(key, entry, libc::SIGHUP);
    // A stopped member acts on the hangup only once continued.
    signal(key, entry, libc::SIGCONT);
    let grace = Instant::now() + HANGUP_GRACE;
    entry.kill_at = Some(
        shutting_down.map_or(grace, |deadline| grace.min(early_kill(deadline))),
    );
}

fn advance(entries: &mut HashMap<u64, Entry>) {
    let now = Instant::now();
    let keys: Vec<u64> = entries.keys().copied().collect();
    for key in keys {
        let due = entries.get(&key).is_some_and(|entry| {
            matches!(entry.owner, Owner::Pinned(_))
                && entry.kill_at.is_some_and(|at| now >= at)
        });
        if due {
            let pid = entries[&key].owner.pid();
            if let Err(error) = probe(key, pid, Site::Escalate) {
                lose(entries, key, &error);
                continue;
            }
            let Some(mut entry) = entries.remove(&key) else {
                continue;
            };
            signal(key, &mut entry, libc::SIGKILL);
            // The last signal: from here the owner can only be reaped.
            entry.owner = match entry.owner {
                Owner::Pinned(child) => Owner::ReapOnly(child, now),
                owner => owner,
            };
            entries.insert(key, entry);
        }
        reap(entries, key);
    }
}

/// Consume the shell's exit once its final signal is sent.
fn reap(entries: &mut HashMap<u64, Entry>, key: u64) {
    let Some(entry) = entries.get_mut(&key) else {
        return;
    };
    let Owner::ReapOnly(_, since) = entry.owner else {
        return;
    };
    let pid = entry.owner.pid();
    match wait(key, pid) {
        Ok(Some(status)) => {
            let Some(entry) = entries.remove(&key) else {
                return;
            };
            #[cfg(any(test, feature = "testing"))]
            {
                testing::record(key, testing::Event::Reaped);
                drop(
                    testing::AFTER_REAP
                        .lock()
                        .unwrap_or_else(|e| e.into_inner()),
                );
            }
            log::info!(
                target: LOG_TARGET,
                "pane {} closed: launch={key} pid={pid} pgid={pid} master_fd_closed={} \
                 signals=[{}] reap=done status={status:?}",
                entry.pane,
                entry.masters_closed,
                entry.signals.join(", ")
            );
        },
        Ok(None) => {
            if since.elapsed() >= REAP_PENDING
                && !entry.signals.iter().any(|s| s == "pending")
            {
                entry.signals.push("pending".to_string());
                log::info!(
                    target: LOG_TARGET,
                    "pane {} closed: launch={key} pid={pid} signals=[{}] reap=pending",
                    entry.pane,
                    entry.signals.join(", ")
                );
            }
        },
        Err(error) => lose(entries, key, &error),
    }
}

/// The launch's authority is gone (a wait failed): publish an unknown
/// status if none was, wake the reader, and never act on it again.
fn lose(entries: &mut HashMap<u64, Entry>, key: u64, error: &io::Error) {
    let Some(entry) = entries.remove(&key) else {
        return;
    };
    #[cfg(any(test, feature = "testing"))]
    testing::record(key, testing::Event::Lost);
    log::error!(
        target: LOG_TARGET,
        "pane {} launch {key} shell {} lost its pin ({error}); no further signals or waits",
        entry.pane,
        entry.owner.pid()
    );
    let unpublished = entry
        .status
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_none();
    if unpublished {
        publish(&entry, PtyExitStatus::Unknown);
    }
}

fn publish(entry: &Entry, status: PtyExitStatus) {
    *entry.status.lock().unwrap_or_else(|e| e.into_inner()) = Some(status);
    match (&entry.wake).write(&[0]) {
        Ok(_) => {},
        // Full: a wake is already pending. Gone: the reader has finished.
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::BrokenPipe
            ) => {},
        Err(error) => log::warn!(
            target: LOG_TARGET,
            "pane {}: waking its reader failed: {error}",
            entry.pane
        ),
    }
}

fn signal(key: u64, entry: &mut Entry, signal: libc::c_int) {
    let pid = entry.owner.pid();
    #[cfg(any(test, feature = "testing"))]
    testing::record(key, testing::Event::Signal(-pid, signal));
    #[cfg(not(any(test, feature = "testing")))]
    let _ = key;
    let outcome = if pid > 1 && unsafe { libc::kill(-pid, signal) } != 0 {
        io::Error::last_os_error().to_string()
    } else {
        "ok".to_string()
    };
    entry.signals.push(format!("{signal}:{outcome}"));
}

/// Where the worker probes a shell; named so tests can fail one probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Site {
    Observe,
    Hangup,
    Escalate,
}

/// `waitid(WNOWAIT)`: the shell's status once exited, leaving it waitable.
fn probe(
    key: u64,
    pid: libc::pid_t,
    site: Site,
) -> io::Result<Option<PtyExitStatus>> {
    #[cfg(any(test, feature = "testing"))]
    {
        testing::record(key, testing::Event::Probe(site));
        if testing::take_injected(key, site) {
            return Err(io::Error::from_raw_os_error(libc::ECHILD));
        }
    }
    #[cfg(not(any(test, feature = "testing")))]
    let _ = (key, site);
    loop {
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let rc = unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if rc != 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(error);
        }
        if unsafe { info.si_pid() } != pid {
            return Ok(None);
        }
        let status = unsafe { info.si_status() };
        return Ok(Some(match info.si_code {
            libc::CLD_EXITED => PtyExitStatus::Code(status),
            libc::CLD_KILLED | libc::CLD_DUMPED => {
                PtyExitStatus::Signal(status)
            },
            _ => PtyExitStatus::Unknown,
        }));
    }
}

/// The consuming wait: `Ok(None)` while the shell is not yet waitable.
fn wait(key: u64, pid: libc::pid_t) -> io::Result<Option<i32>> {
    #[cfg(any(test, feature = "testing"))]
    if testing::take_pending(key) {
        testing::record(key, testing::Event::ReapPending);
        return Ok(None);
    }
    #[cfg(not(any(test, feature = "testing")))]
    let _ = key;
    loop {
        let mut status = 0;
        let rc = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
        if rc == pid {
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

/// Observation and control of the worker for tests, by launch key.
#[cfg(any(test, feature = "testing"))]
pub mod testing {
    use std::collections::{HashMap, HashSet};
    use std::sync::Mutex;

    pub use super::Site;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Event {
        Probe(Site),
        /// `kill(target, signal)` requested.
        Signal(libc::pid_t, libc::c_int),
        ReapPending,
        Reaped,
        Lost,
    }

    /// Held, the worker pauses before its next step.
    pub static HOLD: Mutex<()> = Mutex::new(());
    /// Held, the worker pauses right after a successful consuming wait.
    pub static AFTER_REAP: Mutex<()> = Mutex::new(());
    /// Held, a pty pauses before closing its masters.
    pub static READER_HOLD: Mutex<()> = Mutex::new(());
    static EVENTS: Mutex<Vec<(u64, Event)>> = Mutex::new(Vec::new());
    static PENDING: Mutex<Option<HashMap<u64, u32>>> = Mutex::new(None);
    static INJECT: Mutex<Option<HashSet<(u64, Site)>>> = Mutex::new(None);

    pub(super) fn record(key: u64, event: Event) {
        EVENTS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((key, event));
    }

    /// Everything the worker did for launch `key`, in order.
    pub fn events(key: u64) -> Vec<Event> {
        EVENTS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|(k, _)| *k == key)
            .map(|(_, event)| *event)
            .collect()
    }

    pub(super) fn take_pending(key: u64) -> bool {
        let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
        match pending.get_or_insert_with(HashMap::new).get_mut(&key) {
            Some(left) if *left > 0 => {
                *left -= 1;
                true
            },
            _ => false,
        }
    }

    /// The next `times` reaps of launch `key` find it not yet waitable.
    pub fn set_pending(key: u64, times: u32) {
        PENDING
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_or_insert_with(HashMap::new)
            .insert(key, times);
    }

    pub(super) fn take_injected(key: u64, site: Site) -> bool {
        INJECT
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_or_insert_with(HashSet::new)
            .remove(&(key, site))
    }

    /// The next probe of launch `key` at `site` fails once with ECHILD.
    pub fn fail_probe_once(key: u64, site: Site) {
        INJECT
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_or_insert_with(HashSet::new)
            .insert((key, site));
    }

    static NEXT: Mutex<Option<Site>> = Mutex::new(None);

    /// The next launch's first probe at `site` fails once with ECHILD,
    /// armed before its key exists. For isolated tests.
    pub fn fail_next_launch_probe_once(site: Site) {
        *NEXT.lock().unwrap_or_else(|e| e.into_inner()) = Some(site);
    }

    pub(super) fn arm_next(key: u64) {
        if let Some(site) =
            NEXT.lock().unwrap_or_else(|e| e.into_inner()).take()
        {
            fail_probe_once(key, site);
        }
    }
}
