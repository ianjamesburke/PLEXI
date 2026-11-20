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
//! worker at spawn and never leaves it. On the pane lifecycle path only the
//! worker observes the shell (`waitid` with `WNOWAIT`), signals it, and
//! reaps it; the pane holds a launch key and can only ask. (The host's
//! legacy quit sweep still observes and signals by pid, stint 1660.) Exclusive, unreaped parenthood keeps `S`
//! from being reused while the worker may signal it. A wait error ends the
//! worker's authority for that launch for good. After the final group
//! signal the owner can only be reaped. A lost launch stays on record, with
//! no capability, so host stop never reports it complete. Host stop is a
//! request to the same owner ([`shutdown_panes`]): it seals admission in
//! the same decision that finds the worker, closes every pane and every
//! launch still being admitted, and reports what is unresolved at its
//! deadline; the worker keeps its responsibility while the host lives.
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
    /// An admission ended without a shell (its spawn failed).
    Abandoned,
    Close(u64),
    MastersClosed(u64),
    Sigchld,
    Shutdown {
        deadline: Instant,
        reply: Sender<ShutdownOutcome>,
    },
}

/// What host stop learned from the owner. `owned` lists every shell pid
/// the owner held during the stop, resolved or not, so a caller can tell
/// its own survivors from the owner's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShutdownOutcome {
    /// Every shell was reaped, and none was ever lost.
    Complete { owned: Vec<u32> },
    /// Work still owned or lost at the deadline, described for the log.
    Incomplete {
        owned: Vec<u32>,
        unresolved: Vec<String>,
    },
    /// The owner did not answer by the deadline.
    Unconfirmed,
}

impl ShutdownOutcome {
    /// How much work is unresolved or unknown: 0 only when complete.
    pub fn unresolved(&self) -> usize {
        match self {
            Self::Complete { .. } => 0,
            Self::Incomplete { unresolved, .. } => unresolved.len(),
            Self::Unconfirmed => 1,
        }
    }

    /// The shells the owner held, if it answered.
    pub fn owned(&self) -> Option<&[u32]> {
        match self {
            Self::Complete { owned } | Self::Incomplete { owned, .. } => {
                Some(owned)
            },
            Self::Unconfirmed => None,
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

/// Admission and the worker, decided together: host stop seals and finds
/// the worker under this lock, and a launch counts itself in flight under
/// it, so no launch is admitted unseen. It is never held across a spawn.
struct Admissions {
    worker: Option<Sender<Job>>,
    sealed: bool,
    /// Launches admitted but not yet in the worker's table.
    in_flight: usize,
}

static ADMISSIONS: Mutex<Admissions> = Mutex::new(Admissions {
    worker: None,
    sealed: false,
    in_flight: 0,
});
static NEXT_KEY: AtomicU64 = AtomicU64::new(1);
/// Launches whose pin was lost: (pid, description). Never acted on again,
/// never forgotten.
static LOST: Mutex<Vec<(u32, String)>> = Mutex::new(Vec::new());

fn admissions() -> MutexGuard<'static, Admissions> {
    ADMISSIONS.lock().unwrap_or_else(|e| e.into_inner())
}

/// The worker, started on first use; a pty only spawns once this succeeds.
pub(super) fn worker() -> io::Result<()> {
    sender(&mut admissions()).map(drop)
}

fn sender(admissions: &mut Admissions) -> io::Result<Sender<Job>> {
    if let Some(worker) = admissions.worker.as_ref() {
        return Ok(worker.clone());
    }
    let (worker, jobs) = mpsc::channel();
    // SIGCHLD arrives on a pipe; a small thread turns it into jobs.
    let (sender, mut sigchld) = UnixStream::pair()?;
    let registration =
        signal_hook::low_level::pipe::register(libc::SIGCHLD, sender)?;
    let started = std::thread::Builder::new()
        .name("pty-lifecycle".to_string())
        .spawn(move || run(jobs))
        .and_then(|_| {
            let forward = worker.clone();
            std::thread::Builder::new()
                .name("pty-sigchld".to_string())
                .spawn(move || forward_sigchld(&mut sigchld, &forward))
        });
    if let Err(error) = started {
        // Unregistering drops the pipe's write end; a started lifecycle
        // thread ends once `worker` (its last sender) drops here.
        signal_hook::low_level::unregister(registration);
        return Err(error);
    }
    admissions.worker = Some(worker.clone());
    Ok(worker)
}

fn forward_sigchld(sigchld: &mut UnixStream, forward: &Sender<Job>) {
    let mut bytes = [0u8; 64];
    loop {
        match sigchld.read(&mut bytes) {
            Ok(n) if n > 0 => {
                if forward.send(Job::Sigchld).is_err() {
                    return;
                }
            },
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {},
            Ok(_) => {
                log::error!(target: LOG_TARGET, "the SIGCHLD pipe closed");
                return;
            },
            Err(error) => {
                log::error!(target: LOG_TARGET, "reading the SIGCHLD pipe failed: {error}");
                return;
            },
        }
    }
}

/// Permission to spawn one shell, counted in flight until the worker has
/// it. Holds no lock, so a slow spawn never delays host stop.
pub(super) struct Admission {
    worker: Sender<Job>,
    admitted: bool,
}

pub(super) fn admit() -> io::Result<Admission> {
    let mut admissions = admissions();
    if admissions.sealed {
        return Err(io::Error::other("the host is stopping: no new panes"));
    }
    let worker = sender(&mut admissions)?;
    admissions.in_flight += 1;
    Ok(Admission {
        worker,
        admitted: false,
    })
}

impl Drop for Admission {
    fn drop(&mut self) {
        if !self.admitted {
            admissions().in_flight -= 1;
            let _ = self.worker.send(Job::Abandoned);
        }
    }
}

impl Admission {
    /// Hand the spawned shell to the worker. `wake` is the worker's end of
    /// the reader's notification socket; `reader` shuts that reader down.
    pub(super) fn own(
        mut self,
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
        // The worker takes it out of flight when it takes it in.
        self.admitted = true;
        if self.worker.send(Job::Admit(key, Box::new(entry))).is_err() {
            log::error!(target: LOG_TARGET, "pane {pane}: the lifecycle worker is gone");
        }
        LaunchKey {
            key,
            worker: self.worker.clone(),
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
    let worker = {
        let mut admissions = admissions();
        admissions.sealed = true;
        admissions.worker.clone()
    };
    // No worker under the seal: no launch was ever admitted, and none can be.
    let Some(worker) = worker else {
        return ShutdownOutcome::Complete { owned: Vec::new() };
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

struct Stopping {
    deadline: Instant,
    reply: Sender<ShutdownOutcome>,
    owned: Vec<u32>,
}

fn run(jobs: mpsc::Receiver<Job>) {
    let mut entries: HashMap<u64, Entry> = HashMap::new();
    // Set by the first host stop; launches admitted after it close at once.
    let mut sealed: Option<Instant> = None;
    let mut stopping: Option<Stopping> = None;
    loop {
        let now = Instant::now();
        let next = entries
            .values()
            .filter_map(|entry| match entry.owner {
                Owner::ReapOnly(..) => Some(now + REAP_RETRY),
                Owner::Pinned(_) => entry.kill_at,
            })
            .chain(stopping.as_ref().map(|stop| stop.deadline))
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
        match job {
            Some(Job::Admit(key, entry)) => {
                let pid = entry.owner.pid() as u32;
                entries.insert(key, *entry);
                admissions().in_flight -= 1;
                if let Some(stop) = stopping.as_mut() {
                    stop.owned.push(pid);
                }
                // A shell that exited before admission sent its SIGCHLD
                // before it was here to be observed.
                observe(&mut entries, key);
                if let Some(deadline) = sealed {
                    close_for_stop(&mut entries, key, deadline);
                }
            },
            Some(Job::Abandoned) => {},
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
                hang_up(&mut entries, key, sealed);
            },
            Some(Job::MastersClosed(key)) => {
                if let Some(entry) = entries.get_mut(&key) {
                    entry.masters_closed = true;
                }
                hang_up(&mut entries, key, sealed);
            },
            Some(Job::Shutdown { deadline, reply }) => {
                sealed = Some(deadline);
                let keys: Vec<u64> = entries.keys().copied().collect();
                for key in &keys {
                    close_for_stop(&mut entries, *key, deadline);
                }
                let mut owned: Vec<u32> =
                    entries.values().map(|e| e.owner.pid() as u32).collect();
                owned.extend(lost().iter().map(|(pid, _)| *pid));
                stopping = Some(Stopping {
                    deadline,
                    reply,
                    owned,
                });
            },
            None => {},
        }
        advance(&mut entries);
        let finished = stopping.as_ref().is_some_and(|stop| {
            (entries.is_empty() && admissions().in_flight == 0)
                || Instant::now() >= stop.deadline
        });
        if let Some(stop) = stopping.take_if(|_| finished) {
            let mut unresolved: Vec<String> =
                entries.iter().map(describe).collect();
            unresolved.extend(lost().iter().map(|(_, lost)| lost.clone()));
            let in_flight = admissions().in_flight;
            if in_flight > 0 {
                unresolved
                    .push(format!("{in_flight} launches still being admitted"));
            }
            let _ = stop.reply.send(if unresolved.is_empty() {
                ShutdownOutcome::Complete { owned: stop.owned }
            } else {
                ShutdownOutcome::Incomplete {
                    owned: stop.owned,
                    unresolved,
                }
            });
        }
    }
}

fn lost() -> MutexGuard<'static, Vec<(u32, String)>> {
    LOST.lock().unwrap_or_else(|e| e.into_inner())
}

/// Host stop closes a launch whatever its backend does: the backend may
/// never drop, so its reader is ended too, and the grace is cut short.
fn close_for_stop(
    entries: &mut HashMap<u64, Entry>,
    key: u64,
    deadline: Instant,
) {
    if let Some(entry) = entries.get_mut(&key) {
        entry.close_requested = true;
        let _ = entry.reader.send(Msg::Shutdown);
        if let Some(kill_at) = entry.kill_at.as_mut() {
            *kill_at = (*kill_at).min(early_kill(deadline));
        }
    }
    hang_up(entries, key, Some(deadline));
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
    let pid = entry.owner.pid();
    log::error!(
        target: LOG_TARGET,
        "pane {} launch {key} shell {pid} lost its pin ({error}); no further signals or waits",
        entry.pane
    );
    lost().push((
        pid as u32,
        format!(
            "pane {} launch {key} pid {pid}: pin lost ({error})",
            entry.pane
        ),
    ));
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
    let result = if pid > 1 && unsafe { libc::kill(-pid, signal) } != 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    };
    let exited = result.is_err()
        && (entry
            .status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
            || leader_exited(pid));
    entry
        .signals
        .push(format!("{signal}:{}", signal_outcome(&result, exited)));
}

/// How a group signal ended, for the close log. A group whose leader has
/// exited and has no live member answers ESRCH, or EPERM on macOS when only
/// the zombie leader is left: that is an already-exited group, not a failure.
fn signal_outcome(result: &io::Result<()>, leader_exited: bool) -> String {
    match result {
        Ok(()) => "ok".to_string(),
        Err(error)
            if error.raw_os_error() == Some(libc::ESRCH)
                || (leader_exited
                    && error.raw_os_error() == Some(libc::EPERM)) =>
        {
            "already exited".to_string()
        },
        Err(error) => error.to_string(),
    }
}

/// Whether the owned leader has exited (still unreaped), for the log only.
fn leader_exited(pid: libc::pid_t) -> bool {
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let rc = unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    rc == 0 && unsafe { info.si_pid() } == pid
}

#[cfg(test)]
mod tests {
    use super::signal_outcome;
    use std::io;

    /// The tester saw `1:Operation not permitted` for a group left with only
    /// its zombie leader; that is logged as already exited.
    #[test]
    fn a_zombie_only_group_logs_as_already_exited() {
        let eperm = || Err(io::Error::from_raw_os_error(libc::EPERM));
        let esrch = Err(io::Error::from_raw_os_error(libc::ESRCH));
        assert_eq!(signal_outcome(&eperm(), true), "already exited");
        assert_eq!(signal_outcome(&esrch, false), "already exited");
        assert_eq!(signal_outcome(&Ok(()), false), "ok");
        assert_ne!(signal_outcome(&eperm(), false), "already exited");
    }
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
    /// Held, a launch pauses after its admission, before its spawn.
    pub static ADMIT_HOLD: Mutex<()> = Mutex::new(());
    /// Launches that reached that pause.
    pub static ADMITTING: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);

    pub(crate) fn admission_pause() {
        ADMITTING.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        drop(ADMIT_HOLD.lock().unwrap_or_else(|e| e.into_inner()));
    }
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
