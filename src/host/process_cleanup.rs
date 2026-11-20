//! Pane-owned process teardown. PTY launch uses setsid (alacritty), so the
//! recorded root pid is also its process-group id. The host marker follows
//! children across nohup, setsid and reparenting; it is never inherited by a
//! host launched through `plexi host start`.
use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
#[cfg(unix)]
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Mutex,
};

pub(crate) const SESSION_ENV: &str = "PLEXI_HOST_SESSION";
static SESSION: OnceLock<String> = OnceLock::new();
#[cfg(unix)]
#[derive(Clone)]
struct RootProcess {
    pgid: i32,
    started: u64,
    marker: String,
}
#[cfg(unix)]
static ROOTS: Mutex<BTreeMap<u32, RootProcess>> = Mutex::new(BTreeMap::new());

pub(crate) fn session_marker() -> &'static str {
    SESSION.get_or_init(|| uuid::Uuid::new_v4().to_string())
}

const SESSION_RECORD: &str = "pane-session";
/// Cap on the shared quit cleanup deadline `D` for pane processes and tmux
/// sessions together. These are the deadlines cleanup is allocated, not a
/// wall-clock ceiling: tmux is allocated until `max(D, tmux_start +
/// TMUX_MIN_BUDGET)` (`cleanup_end`), and `tmux_start` is normally within
/// `POST_KILL_MIN` of the kill. Process scans, refreshes, survivor logging
/// and helper spawns are not themselves bounded, so cleanup can end later
/// than its allocation; there is no hard ceiling on its wall-clock time.
const QUIT_CLEANUP_CAP: Duration = Duration::from_millis(2500);
/// What tmux cleanup is always left, however long the pane processes took
/// (stint 1526). Cleanup asks every session on the default server for its
/// marker, one spawn per session, so a user with many unmarked sessions can
/// still use this up before the host's marked ones are reached.
const TMUX_MIN_BUDGET: Duration = Duration::from_millis(150);
/// Longest wait to observe SIGKILLed processes gone.
const POST_KILL_WAIT: Duration = Duration::from_millis(250);
/// What that observation is always left after the grace: SIGKILL death is
/// near instant, and a zombie already counts as gone (stint 1523).
#[cfg(unix)]
const POST_KILL_MIN: Duration = Duration::from_millis(50);
#[cfg(unix)]
const ROOTS_RECORD: &str = "pane-roots";

/// Persist just enough identity for `host stop` to clean panes when its socket
/// is already unavailable.  The PID check prevents a stale record from ever
/// selecting another host's panes.
pub(crate) fn record_session_marker(profile: &Path) {
    let path = profile.join(SESSION_RECORD);
    if let Err(error) = std::fs::write(
        &path,
        format!("{}\n{}\n", std::process::id(), session_marker()),
    ) {
        log::warn!("quit: could not record pane session marker at {path:?}: {error}");
    }
    #[cfg(unix)]
    {
        let roots = profile.join(ROOTS_RECORD);
        if let Err(error) = std::fs::remove_file(&roots) {
            if error.kind() != std::io::ErrorKind::NotFound {
                log::warn!("quit: could not clear stale pane roots at {roots:?}: {error}");
            }
        }
    }
}

pub(crate) fn recorded_session_marker(profile: &Path, host_pid: u32) -> Option<String> {
    let path = profile.join(SESSION_RECORD);
    let content = std::fs::read_to_string(&path).ok()?;
    let mut lines = content.lines();
    let recorded_pid = lines.next()?.parse::<u32>().ok()?;
    let marker = lines.next()?.to_string();
    (recorded_pid == host_pid && !marker.is_empty()).then_some(marker)
}

#[derive(Debug)]
pub(crate) struct PaneProcessGroup {
    #[cfg(unix)]
    pid: u32,
    #[cfg(unix)]
    profile: Option<PathBuf>,
}
impl PaneProcessGroup {
    #[cfg(not(test))]
    pub(crate) fn record(pid: u32) -> Self {
        let profile = crate::config::config_dir();
        Self::record_for_profile(pid, session_marker(), &profile)
    }

    #[cfg(test)]
    pub(crate) fn record(pid: u32) -> Self {
        Self::record_for(pid, session_marker())
    }

    #[cfg(test)]
    pub(crate) fn record_for(pid: u32, marker: &str) -> Self {
        Self::record_for_profile_inner(pid, marker, None)
    }

    pub(crate) fn record_for_profile(pid: u32, marker: &str, profile: &Path) -> Self {
        Self::record_for_profile_inner(pid, marker, Some(profile))
    }

    fn record_for_profile_inner(pid: u32, marker: &str, profile: Option<&Path>) -> Self {
        #[cfg(unix)]
        let persisted_profile = profile.map(Path::to_path_buf);
        #[cfg(unix)]
        {
            let pgid = unsafe { libc::getpgid(pid as i32) };
            if pgid == pid as i32 && pid > 1 {
                let mut system = sysinfo::System::new();
                system.refresh_processes_specifics(
                    sysinfo::ProcessesToUpdate::Some(&[sysinfo::Pid::from_u32(pid)]),
                    true,
                    sysinfo::ProcessRefreshKind::new(),
                );
                if let Some(process) = system.process(sysinfo::Pid::from_u32(pid)) {
                    let root = RootProcess {
                        pgid,
                        started: process.start_time(),
                        marker: marker.into(),
                    };
                    let mut roots = ROOTS.lock().unwrap_or_else(|e| e.into_inner());
                    roots.insert(pid, root.clone());
                    if let Some(profile) = persisted_profile.as_deref() {
                        persist_pane_root(profile, pid, &root);
                    }
                } else {
                    log::warn!("pane process identity unavailable pid={pid}");
                }
                log::info!("pane process group registered pid={pid} pgid={pgid}");
            } else {
                log::warn!("pane process group not isolated pid={pid} pgid={pgid}");
            }
        }
        #[cfg(not(unix))]
        let _ = (pid, marker, profile);
        Self {
            #[cfg(unix)]
            pid,
            #[cfg(unix)]
            profile: persisted_profile,
        }
    }
}
#[cfg(unix)]
impl Drop for PaneProcessGroup {
    fn drop(&mut self) {
        let mut roots = ROOTS.lock().unwrap_or_else(|e| e.into_inner());
        let root = roots.remove(&self.pid);
        if let (Some(profile), Some(root)) = (self.profile.as_deref(), root) {
            remove_persisted_pane_root(profile, self.pid, &root);
        }
    }
}

#[cfg(unix)]
fn pane_roots_path(profile: &Path) -> PathBuf {
    profile.join(ROOTS_RECORD)
}

#[cfg(unix)]
fn read_persisted_pane_roots(profile: &Path) -> BTreeMap<u32, RootProcess> {
    let path = pane_roots_path(profile);
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return BTreeMap::new();
    };
    contents
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let pid = fields.next()?.parse().ok()?;
            let pgid = fields.next()?.parse().ok()?;
            let started = fields.next()?.parse().ok()?;
            let marker = fields.next()?.to_owned();
            (fields.next().is_none() && !marker.is_empty()).then_some((
                pid,
                RootProcess {
                    pgid,
                    started,
                    marker,
                },
            ))
        })
        .collect()
}

#[cfg(unix)]
fn write_persisted_pane_roots(profile: &Path, roots: &BTreeMap<u32, RootProcess>) {
    let path = pane_roots_path(profile);
    let contents = roots
        .iter()
        .map(|(pid, root)| format!("{pid}\t{}\t{}\t{}\n", root.pgid, root.started, root.marker))
        .collect::<String>();
    if let Err(error) = std::fs::write(&path, contents) {
        log::warn!("quit: could not persist pane roots at {path:?}: {error}");
    }
}

#[cfg(unix)]
fn persist_pane_root(profile: &Path, pid: u32, root: &RootProcess) {
    let mut roots = read_persisted_pane_roots(profile);
    roots.insert(pid, root.clone());
    write_persisted_pane_roots(profile, &roots);
}

#[cfg(unix)]
fn remove_persisted_pane_root(profile: &Path, pid: u32, root: &RootProcess) {
    let mut roots = read_persisted_pane_roots(profile);
    if roots.get(&pid).is_some_and(|saved| {
        saved.pgid == root.pgid && saved.started == root.started && saved.marker == root.marker
    }) {
        roots.remove(&pid);
        write_persisted_pane_roots(profile, &roots);
    }
}

/// Restore the roots recorded by the host so a separate `plexi host stop`
/// process can identify panes when macOS hides their environment.
#[cfg(unix)]
pub(crate) fn restore_pane_roots(profile: &Path, marker: &str) -> usize {
    let roots = read_persisted_pane_roots(profile)
        .into_iter()
        .filter(|(_, root)| root.marker == marker)
        .collect::<BTreeMap<_, _>>();
    let restored = roots.len();
    ROOTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .extend(roots);
    restored
}

#[cfg(all(test, unix))]
pub(crate) fn forget_pane_root_for_test(pid: u32) {
    ROOTS.lock().unwrap_or_else(|e| e.into_inner()).remove(&pid);
}

#[cfg(not(unix))]
pub(crate) fn restore_pane_roots(_profile: &Path, _marker: &str) -> usize {
    0
}

/// Outcome and per-phase cost of one teardown, logged by `exit_host` so a
/// quit-latency regression is visible in the log without guessing.
#[derive(Default, Debug)]
pub(crate) struct KillReport {
    pub groups: usize,
    pub escalated: usize,
    pub grace: Duration,
    pub tmux: Duration,
    pub scan: Duration,
    pub signal: Duration,
    pub grace_waited: Duration,
    pub kill: Duration,
    pub remaining: usize,
    /// The pids counted in `remaining`.
    pub survivors: Vec<u32>,
}

/// `default_grace` is the caller's budget between HUP/TERM and SIGKILL;
/// `PLEXI_QUIT_GRACE_MS` overrides it when set. After the marker sweep, the
/// terminal panes are closed by their shells' owner (`stop_terminal_panes`);
/// work it reports unresolved counts as remaining, which withholds the
/// clean stamp. The sweep runs first so its descendant search can start
/// from the shells of panes still open; a pane already closing may be
/// reaped by its owner during the sweep.
pub(crate) fn stop_pane_processes(default_grace: Duration) -> KillReport {
    let mut report = stop_pane_processes_for_marker(session_marker(), default_grace);
    report.remaining =
        stop_terminal_panes(&report.survivors, Instant::now() + PANE_SHUTDOWN_BUDGET);
    report
}

/// How long host stop waits for the terminal panes' owner (stint 1650):
/// enough for each pane's hangup, a shortened grace and the reap.
const PANE_SHUTDOWN_BUDGET: Duration = Duration::from_millis(750);

/// Ask egui_term's lifecycle worker, the one owner of every terminal
/// pane's shell, to close them all by `deadline`. Returns what is left: the
/// sweep's `survivors` the owner does not account for, plus the work the
/// owner reports unresolved or could not confirm.
fn stop_terminal_panes(survivors: &[u32], deadline: Instant) -> usize {
    #[cfg(unix)]
    {
        let started = Instant::now();
        let outcome = egui_term::shutdown_panes(deadline);
        if outcome.unresolved() == 0 {
            log::info!(
                "quit: terminal panes closed by their owner in {}ms",
                started.elapsed().as_millis()
            );
        } else {
            log::error!(
                "quit: terminal panes not all closed after {}ms: {outcome:?}",
                started.elapsed().as_millis()
            );
        }
        left_after_owner(survivors, &outcome)
    }
    #[cfg(not(unix))]
    {
        let _ = deadline;
        survivors.len()
    }
}

/// A sweep survivor that is a shell the owner held is the owner's to
/// report: the owner kept it unreaped, so the pid names the same process.
#[cfg(unix)]
fn left_after_owner(survivors: &[u32], owner: &egui_term::ShutdownOutcome) -> usize {
    let theirs = owner.owned().unwrap_or_default();
    survivors.iter().filter(|pid| !theirs.contains(pid)).count() + owner.unresolved()
}

/// Stop pane groups belonging to `marker`, bounded across process and tmux
/// cleanup. This is also the socket-failure fallback used by `host stop`.
pub(crate) fn stop_pane_processes_for_marker(marker: &str, default_grace: Duration) -> KillReport {
    stop_pane_processes_with_tmux(marker, default_grace, "tmux", TMUX_MIN_BUDGET)
}

/// `tmux_min` is the tmux minimum: `TMUX_MIN_BUDGET` in production. Tests
/// pass their own so that what they prove does not depend on how fast a
/// process spawns on the machine running them.
fn stop_pane_processes_with_tmux(
    marker: &str,
    default_grace: Duration,
    tmux: &str,
    tmux_min: Duration,
) -> KillReport {
    let grace = match std::env::var("PLEXI_QUIT_GRACE_MS") {
        Ok(value) => match value.parse::<u64>() {
            Ok(ms) => Duration::from_millis(ms.min(30_000)),
            Err(error) => {
                log::warn!("quit: invalid PLEXI_QUIT_GRACE_MS: {error}");
                default_grace
            }
        },
        Err(_) => default_grace,
    };
    let start = Instant::now();
    // The shared deadline D = min(grace + POST_KILL_WAIT, QUIT_CLEANUP_CAP).
    // The tmux reserve comes out of the post-kill wait, not the grace: pane
    // processes are signaled, given their grace and killed by D minus the
    // tmux minimum, so a grace that runs to the cap no longer starves tmux.
    let deadline = start + (grace + POST_KILL_WAIT).min(QUIT_CLEANUP_CAP);
    let process_deadline = deadline
        .checked_sub(tmux_min)
        .map_or(start, |end| end.max(start));
    crate::app::quit::trace("children_begin");
    let mut report = stop_marked_processes(marker, grace, process_deadline);
    // Pane HUP/TERM must never wait on a stuck tmux server. Tmux gets the rest
    // of the shared budget, and never less than TMUX_MIN_BUDGET.
    crate::app::quit::trace("tmux_begin");
    let tmux_start = Instant::now();
    let tmux_deadline = cleanup_end(deadline, tmux_start, tmux_min);
    log::info!(
        "quit: tmux cleanup budget {}ms",
        tmux_deadline
            .saturating_duration_since(tmux_start)
            .as_millis()
    );
    if tmux_deadline > deadline {
        log::info!(
            "quit: tmux cleanup allocated {}ms past the shared deadline: the post-kill \
             observation floor or a slow process scan used the tmux share",
            (tmux_deadline - deadline).as_millis()
        );
    }
    stop_marked_tmux_sessions(tmux, marker, tmux_deadline);
    report.tmux = tmux_start.elapsed();
    crate::app::quit::trace("tmux_end");
    KillReport { grace, ..report }
}

/// The deadline tmux cleanup is allocated: the shared deadline, unless tmux
/// would get less than its minimum from `tmux_start` (stint 1526). This is an
/// allocation, not a guarantee the cleanup has finished by then.
fn cleanup_end(deadline: Instant, tmux_start: Instant, tmux_min: Duration) -> Instant {
    deadline.max(tmux_start + tmux_min)
}

/// How long to watch SIGKILLed processes go: at most `POST_KILL_WAIT`, never
/// past `process_deadline`, but never less than `POST_KILL_MIN` either, so a
/// process killed a moment ago is not counted as surviving (stint 1523).
#[cfg(unix)]
fn post_kill_deadline(killed: Instant, process_deadline: Instant) -> Instant {
    (killed + POST_KILL_WAIT).min(process_deadline.max(killed + POST_KILL_MIN))
}

/// Run a shutdown helper bounded by the quit deadline, so an unresponsive
/// tmux server (or a descendant holding its output) never stalls host exit.
fn quit_helper(
    program: &str,
    args: &[&str],
    deadline: Instant,
) -> Result<plexi_distribution::bounded::Exited, plexi_distribution::bounded::RunError> {
    let what = format!("{program} {}", args.first().copied().unwrap_or_default());
    let limit = deadline.saturating_duration_since(Instant::now());
    if limit.is_zero() {
        return Err(plexi_distribution::bounded::RunError::TimedOut {
            what,
            limit,
            stdout: Vec::new(),
            stderr: Vec::new(),
        });
    }
    plexi_distribution::bounded::Helper::new(program)
        .args(args)
        .run_any_exit(&what, limit)
}

/// An existing tmux server does not necessarily propagate a client's env to
/// sessions. Only an explicit matching session environment proves ownership;
/// never kill an unmarked shared server or a user-selected custom socket.
fn stop_marked_tmux_sessions(tmux: &str, marker: &str, deadline: Instant) {
    let sessions = match quit_helper(tmux, &["list-sessions", "-F", "#{session_id}"], deadline) {
        Ok(exited) if exited.status.success() => exited.output,
        Ok(_) => return, // No default server is a normal state.
        Err(plexi_distribution::bounded::RunError::Start { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            return;
        }
        Err(e) => {
            log::warn!("quit: tmux session discovery failed: {e}");
            return;
        }
    };
    let expected = format!("{SESSION_ENV}={marker}");
    for session in String::from_utf8_lossy(&sessions.stdout).lines() {
        if Instant::now() >= deadline {
            log::warn!("quit: tmux session cleanup deadline elapsed");
            break;
        }
        let env = match quit_helper(
            tmux,
            &["show-environment", "-t", session, SESSION_ENV],
            deadline,
        ) {
            Ok(exited) => exited,
            Err(e) => {
                log::warn!("quit: tmux environment query failed session={session}: {e}");
                continue;
            }
        };
        if env.status.success() && String::from_utf8_lossy(&env.output.stdout).trim() == expected {
            match quit_helper(tmux, &["kill-session", "-t", session], deadline) {
                Ok(exited) if exited.status.success() => {
                    log::info!("quit: stopped marked tmux session={session}")
                }
                Ok(exited) => log::warn!(
                    "quit: tmux kill-session failed session={session} status={}",
                    exited.status
                ),
                Err(e) => log::warn!("quit: tmux kill-session failed session={session}: {e}"),
            }
        }
    }
}

#[cfg(unix)]
fn signal_target_unavailable(code: Option<i32>) -> bool {
    matches!(code, Some(libc::EPERM) | Some(libc::ESRCH))
}

#[cfg(unix)]
fn stop_marked_processes(marker: &str, grace: Duration, total_deadline: Instant) -> KillReport {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let start = Instant::now();
    let session = marker;
    let marker = std::ffi::OsString::from(format!("{SESSION_ENV}={marker}"));
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::new()
            .with_environ(UpdateKind::Always)
            .with_exe(UpdateKind::Always),
    );
    let own_pid = std::process::id();
    let own_group = unsafe { libc::getpgrp() };
    let mut owned: BTreeSet<u32> = system
        .processes()
        .iter()
        .filter(|(pid, p)| pid.as_u32() != own_pid && p.environ().contains(&marker))
        .map(|(pid, _)| pid.as_u32())
        .collect();
    let recorded = ROOTS.lock().unwrap_or_else(|e| e.into_inner()).clone();
    for (pid, root) in &recorded {
        if root.marker == session
            && system
                .process(sysinfo::Pid::from_u32(*pid))
                .is_some_and(|process| process.start_time() == root.started)
        {
            owned.insert(*pid);
        }
    }
    // Snapshot descendants before signaling any parent. This also includes
    // children that scrubbed their environment but have not been reparented.
    loop {
        let before = owned.len();
        for (pid, process) in system.processes() {
            if process
                .parent()
                .is_some_and(|p| owned.contains(&p.as_u32()))
                && pid.as_u32() != own_pid
            {
                owned.insert(pid.as_u32());
            }
        }
        if before == owned.len() {
            break;
        }
    }
    let identities: BTreeMap<u32, u64> = owned
        .iter()
        .filter_map(|pid| {
            system
                .process(sysinfo::Pid::from_u32(*pid))
                .map(|p| (*pid, p.start_time()))
        })
        .collect();
    let process_groups: BTreeMap<u32, i32> = system
        .processes()
        .keys()
        .map(|p| (p.as_u32(), unsafe { libc::getpgid(p.as_u32() as i32) }))
        .collect();
    let groups: BTreeSet<i32> = owned
        .iter()
        .filter_map(|pid| {
            let group = *process_groups.get(pid)?;
            if let Some(root) = recorded.get(pid) {
                let recorded_group = root.pgid;
                if recorded_group != group {
                    log::warn!("quit: pane root {pid} changed group {recorded_group} -> {group}");
                }
            }
            // Never signal a group containing a process outside this host's set.
            (group > 1
                && group != own_group
                && process_groups
                    .iter()
                    .all(|(p, g)| *g != group || owned.contains(p)))
            .then_some(group)
        })
        .collect();
    let signal = |target: i32, signal: i32| {
        // A group may have exited or ceased to be signalable since the
        // snapshot. Do not escalate it, and keep these expected races quiet.
        if unsafe { libc::kill(target, 0) } != 0 {
            let error = std::io::Error::last_os_error();
            if signal_target_unavailable(error.raw_os_error()) {
                log::debug!("quit: skipping signal={signal} target={target}: {error}");
                return;
            }
        }
        if unsafe { libc::kill(target, signal) } != 0 {
            let error = std::io::Error::last_os_error();
            if signal_target_unavailable(error.raw_os_error()) {
                log::debug!("quit: skipping signal={signal} target={target}: {error}");
            } else {
                log::warn!("quit: signal={signal} target={target} failed: {error}");
            }
        }
    };
    let scanned = Instant::now();
    crate::app::quit::trace("children_scanned");
    // HUP first, like a terminal going away: interactive shells ignore TERM
    // but exit (saving history) on HUP. TERM follows for daemons that only
    // honor TERM.
    for sig in [libc::SIGHUP, libc::SIGTERM] {
        crate::app::quit::trace(if sig == libc::SIGHUP {
            "children_hup"
        } else {
            "children_term"
        });
        for group in &groups {
            signal(-*group, sig);
        }
        for pid in &owned {
            if !process_groups
                .get(pid)
                .is_some_and(|group| groups.contains(group))
            {
                signal(*pid as i32, sig);
            }
        }
    }
    let signaled = Instant::now();
    // The grace runs to `total_deadline` at most (the shared deadline minus
    // the tmux minimum); it is never shortened by more than that.
    let deadline = (signaled + grace).min(total_deadline);
    // Pids already reported as zombies, so each is logged once (stint 1523).
    let mut zombies = BTreeSet::new();
    let tracked: Vec<_> = identities
        .keys()
        .map(|pid| sysinfo::Pid::from_u32(*pid))
        .collect();
    let survivors = loop {
        let live = refresh_owned_alive(&mut system, &tracked, &identities, &mut zombies);
        if live.is_empty() || Instant::now() >= deadline {
            break live;
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    let waited = Instant::now();
    crate::app::quit::trace("children_grace_end");
    let mut escalated = BTreeSet::new();
    for pid in &survivors {
        let pid = *pid;
        let group = unsafe { libc::getpgid(pid as i32) };
        // Only executable identity; never argv or environment (which can hold secrets).
        let executable = system
            .process(sysinfo::Pid::from_u32(pid))
            .and_then(|process| process.exe())
            .map(|path| path.as_os_str().to_string_lossy().into_owned());
        log::info!(
            "quit_survivor: pid={pid} pgid={group} executable={executable:?} signal=SIGKILL"
        );
        if groups.contains(&group) {
            escalated.insert(group);
        }
        signal(pid as i32, libc::SIGKILL);
    }
    // Group delivery also catches children forked during the grace interval.
    for group in &escalated {
        signal(-*group, libc::SIGKILL);
    }
    // SIGKILL delivery is not process death. Observe owned identities gone
    // before reporting cleanup complete (`post_kill_deadline`).
    let reap_deadline = post_kill_deadline(Instant::now(), total_deadline);
    let remaining = loop {
        let live = refresh_owned_alive(&mut system, &tracked, &identities, &mut zombies);
        if live.is_empty() || Instant::now() >= reap_deadline {
            break live;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    for pid in &remaining {
        let process = system.process(sysinfo::Pid::from_u32(*pid));
        log::info!(
            "quit_survivor_remaining: pid={pid} status={:?} parent={:?} zombie={}",
            process.map(|p| p.status()),
            process.and_then(|p| p.parent()),
            process_is_zombie(*pid)
        );
    }
    if !remaining.is_empty() {
        log::error!("quit: owned processes remain after SIGKILL pids={remaining:?}");
    }
    crate::app::quit::trace("children_kill_end");
    KillReport {
        groups: groups.len(),
        escalated: escalated.len(),
        scan: scanned - start,
        signal: signaled - scanned,
        grace_waited: waited - signaled,
        kill: waited.elapsed(),
        remaining: remaining.len(),
        survivors: remaining,
        ..KillReport::default()
    }
}

#[cfg(unix)]
fn refresh_owned_alive(
    system: &mut sysinfo::System,
    tracked: &[sysinfo::Pid],
    identities: &BTreeMap<u32, u64>,
    zombies: &mut BTreeSet<u32>,
) -> Vec<u32> {
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(tracked),
        true,
        sysinfo::ProcessRefreshKind::new(),
    );
    identities
        .iter()
        .filter_map(|(pid, started)| {
            // An exited root is gone. Peek without reaping (WNOWAIT): the
            // root's owner (alacritty's PTY loop for a terminal) is its one
            // reaper, and consuming the status here made its `try_wait` fail
            // with ECHILD on every clean host stop (stint 1509). A zombie
            // descendant has also already stopped running.
            if root_has_exited(*pid) {
                return None;
            }
            if process_is_zombie(*pid) {
                let process = system.process(sysinfo::Pid::from_u32(*pid));
                let reported = process.map(|p| p.status());
                if reported.is_some_and(|status| status != sysinfo::ProcessStatus::Zombie)
                    && zombies.insert(*pid)
                {
                    log::info!(
                        "quit: pid={pid} executable={:?} is a zombie (sysinfo reports {reported:?}); counted gone",
                        process.map(|p| p.name().to_string_lossy().into_owned())
                    );
                }
                return None;
            }
            system
                .process(sysinfo::Pid::from_u32(*pid))
                .filter(|p| {
                    p.start_time() == *started && p.status() != sysinfo::ProcessStatus::Zombie
                })
                .map(|_| *pid)
        })
        .collect()
}

/// Whether `pid` has died and only waits to be reaped by its parent. sysinfo
/// cannot say on macOS: a zombie has no task left, so `proc_pidinfo` fails
/// with ESRCH, and sysinfo then falls back to `kill(pid, 0)`, which still
/// succeeds, and reports the zombie as running. A process killed at quit
/// stayed "alive" until the deadline and the quit was stamped unclean.
#[cfg(target_os = "macos")]
fn process_is_zombie(pid: u32) -> bool {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    // SAFETY: `info` is a correctly sized, writable proc_bsdinfo buffer.
    let written = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size,
        )
    };
    if written == size {
        return info.pbi_status == libc::SZOMB;
    }
    written == 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
}

/// Elsewhere sysinfo reads the process state fresh on every refresh.
#[cfg(all(unix, not(target_os = "macos")))]
fn process_is_zombie(_pid: u32) -> bool {
    false
}

/// Whether our child `pid` has exited, leaving it waitable for its owner.
#[cfg(unix)]
fn root_has_exited(pid: u32) -> bool {
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let rc = unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    rc == 0 && unsafe { info.si_pid() } as u32 == pid
}

#[cfg(not(unix))]
fn stop_marked_processes(_marker: &str, _grace: Duration, _total_deadline: Instant) -> KillReport {
    log::warn!("quit: explicit pane process-group teardown requires Unix");
    KillReport::default()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::process::CommandExt;

    /// Stint 1650: a shell the sweep counted while it was still exiting,
    /// and the owner then reaped, is not left; others the sweep counted are,
    /// and so is everything the owner left unresolved or never confirmed.
    #[test]
    fn owner_reaped_shells_are_not_counted_as_sweep_survivors() {
        use egui_term::ShutdownOutcome::{Complete, Incomplete, Unconfirmed};
        let complete = Complete {
            owned: vec![10, 11],
        };
        assert_eq!(left_after_owner(&[10], &complete), 0);
        assert_eq!(left_after_owner(&[10, 99], &complete), 1);
        let incomplete = Incomplete {
            owned: vec![10],
            unresolved: vec!["pane 1".into()],
        };
        assert_eq!(left_after_owner(&[10, 99], &incomplete), 2);
        assert_eq!(left_after_owner(&[10], &Unconfirmed), 2);
    }

    #[test]
    fn exited_or_unowned_signal_targets_are_expected() {
        assert!(signal_target_unavailable(Some(libc::ESRCH)));
        assert!(signal_target_unavailable(Some(libc::EPERM)));
        assert!(!signal_target_unavailable(Some(libc::EINVAL)));
        assert!(!signal_target_unavailable(None));
    }

    #[test]
    #[ignore = "requires-pty"]
    fn unresponsive_quit_helper_is_killed_at_deadline() {
        let start = Instant::now();
        let error =
            quit_helper("/bin/sleep", &["600"], start + Duration::from_millis(30)).unwrap_err();
        assert!(
            matches!(
                error,
                plexi_distribution::bounded::RunError::TimedOut { .. }
            ),
            "{error}"
        );
        assert!(start.elapsed() < crate::testing::load_aware_timeout(Duration::from_secs(1)));
    }

    /// Stint 1626: a helper that exits while a descendant still holds its
    /// stdout returns at once with its output; the descendant does not hold
    /// host exit until it ends.
    #[test]
    fn a_quit_helper_whose_descendant_holds_stdout_returns_at_once() {
        let start = Instant::now();
        let exited = quit_helper(
            "/bin/sh",
            &["-c", "sleep 10 & echo listed"],
            start + Duration::from_secs(8),
        )
        .unwrap();
        assert!(exited.status.success());
        assert_eq!(String::from_utf8_lossy(&exited.output.stdout), "listed\n");
        assert!(
            start.elapsed() < Duration::from_secs(4),
            "held for {:?}",
            start.elapsed()
        );
    }

    #[test]
    #[ignore = "requires-pty"]
    fn unresponsive_tmux_stays_within_total_pane_cleanup_cap() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let tmux = dir.path().join("tmux");
        // `exec` ensures the deadline kill leaves no orphaned sleep fixture.
        std::fs::write(&tmux, "#!/bin/sh\nexec sleep 600\n").unwrap();
        std::fs::set_permissions(&tmux, std::fs::Permissions::from_mode(0o755)).unwrap();
        let marker = uuid::Uuid::new_v4().to_string();
        let ready = dir.path().join("ready");
        let mut command = std::process::Command::new("/bin/sh");
        command
            .args([
                "-c",
                "trap '' HUP TERM; : > \"$READY\"; while :; do sleep 600; done",
            ])
            .env("READY", &ready)
            .env(SESSION_ENV, &marker);
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        let mut child = FixtureChild(command.spawn().unwrap());
        // Signals sent before the shell installs its trap would just kill it.
        let ready_deadline =
            Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(10));
        while !ready.exists() {
            assert!(
                Instant::now() < ready_deadline,
                "fixture never became ready"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        let registration = PaneProcessGroup::record_for(child.0.id(), &marker);
        let start = Instant::now();
        let report = stop_pane_processes_with_tmux(
            &marker,
            Duration::from_millis(30),
            tmux.to_str().unwrap(),
            TMUX_MIN_BUDGET,
        );
        assert!(
            start.elapsed() < crate::testing::load_aware_timeout(Duration::from_millis(450)),
            "cleanup exceeded its 280ms cap"
        );
        assert!(
            report.tmux < crate::testing::load_aware_timeout(Duration::from_millis(300)),
            "tmux exceeded remaining budget: {:?}",
            report.tmux
        );
        assert_eq!(report.groups, 1, "marked pane must be signaled before tmux");
        assert_eq!(
            report.escalated, 1,
            "HUP/TERM-ignoring pane must be killed by the shared deadline"
        );
        match child.0.wait() {
            Ok(status) => assert!(!status.success()),
            Err(error) => assert_eq!(error.raw_os_error(), Some(libc::ECHILD)),
        }
        drop(registration);
    }

    /// Stint 1526: when the grace runs to the shared cap and a SIGKILL is
    /// needed, tmux cleanup is still allocated its minimum and leaves no
    /// marked session. Without the reserve it is allocated nothing, so the
    /// session survives whatever the minimum is. The test passes a generous,
    /// load-scaled minimum so it never depends on how fast three process
    /// spawns are on the runner. The fake `tmux` keeps its sessions in a
    /// file (no tmux server).
    #[test]
    fn tmux_cleanup_keeps_its_minimum_budget_after_a_sigkill() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let marker = uuid::Uuid::new_v4().to_string();
        let sessions = dir.path().join("sessions");
        std::fs::write(&sessions, "$1\n").unwrap();
        let tmux = dir.path().join("tmux");
        std::fs::write(
            &tmux,
            format!(
                "#!/bin/sh\nsleep 0.005\ncase \"$1\" in\n\
                 list-sessions) cat \"{s}\" ;;\n\
                 show-environment) echo \"{env}={marker}\" ;;\n\
                 kill-session) grep -vx -- \"$3\" \"{s}\" > \"{s}.new\"; mv \"{s}.new\" \"{s}\" ;;\n\
                 esac\n",
                s = sessions.display(),
                env = SESSION_ENV,
            ),
        )
        .unwrap();
        std::fs::set_permissions(&tmux, std::fs::Permissions::from_mode(0o755)).unwrap();
        let ready = dir.path().join("ready");
        let mut command = std::process::Command::new("/bin/sh");
        command
            .args([
                "-c",
                "trap '' HUP TERM; : > \"$READY\"; while :; do sleep 600; done",
            ])
            .env("READY", &ready)
            .env(SESSION_ENV, &marker);
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        let mut child = FixtureChild(command.spawn().unwrap());
        let ready_deadline =
            Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(10));
        while !ready.exists() {
            assert!(
                Instant::now() < ready_deadline,
                "fixture never became ready"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        let registration = PaneProcessGroup::record_for(child.0.id(), &marker);
        let start = Instant::now();
        // A grace at the shared cap: before stint 1526 it consumed the whole
        // budget and tmux cleanup got none.
        let tmux_min = crate::testing::load_aware_timeout(Duration::from_secs(2));
        let report = stop_pane_processes_with_tmux(
            &marker,
            QUIT_CLEANUP_CAP,
            tmux.to_str().unwrap(),
            tmux_min,
        );
        let elapsed = start.elapsed();
        assert_eq!(report.escalated, 1, "the pane needed a SIGKILL");
        assert_eq!(
            std::fs::read_to_string(&sessions).unwrap().trim(),
            "",
            "the marked tmux session must be stopped (tmux ran {:?})",
            report.tmux
        );
        assert!(
            elapsed < crate::testing::load_aware_timeout(QUIT_CLEANUP_CAP) + tmux_min,
            "cleanup took {elapsed:?}"
        );
        match child.0.wait() {
            Ok(status) => assert!(!status.success()),
            Err(error) => assert_eq!(error.raw_os_error(), Some(libc::ECHILD)),
        }
        drop(registration);
    }

    /// Stint 1526, with chosen instants: the post-kill observation stops at
    /// the process deadline when that comes first, keeps its floor when the
    /// deadline has already passed, and never runs past its own maximum.
    #[test]
    fn post_kill_wait_is_capped_by_the_process_deadline_with_a_floor() {
        let killed = Instant::now();
        let deadline = killed + Duration::from_millis(120);
        assert_eq!(post_kill_deadline(killed, deadline), deadline);
        let passed = killed
            .checked_sub(Duration::from_millis(30))
            .unwrap_or(killed);
        assert_eq!(post_kill_deadline(killed, passed), killed + POST_KILL_MIN);
        let far = killed + Duration::from_secs(5);
        assert_eq!(post_kill_deadline(killed, far), killed + POST_KILL_WAIT);
    }

    /// Stint 1526: tmux is allocated until the shared deadline unless that
    /// would leave it less than its minimum.
    #[test]
    fn cleanup_ends_at_the_deadline_unless_the_tmux_floor_needs_more() {
        let tmux_start = Instant::now();
        let roomy = tmux_start + Duration::from_millis(400);
        assert_eq!(cleanup_end(roomy, tmux_start, TMUX_MIN_BUDGET), roomy);
        let tight = tmux_start + Duration::from_millis(20);
        assert_eq!(
            cleanup_end(tight, tmux_start, TMUX_MIN_BUDGET),
            tmux_start + TMUX_MIN_BUDGET
        );
    }

    struct FixtureChild(std::process::Child);
    impl Drop for FixtureChild {
        fn drop(&mut self) {
            match self.0.try_wait() {
                Ok(Some(_)) => return,
                Err(error) if error.raw_os_error() == Some(libc::ECHILD) => return,
                Ok(None) | Err(_) => {}
            }
            // Isolated fixture groups only, including assertion-failure cleanup.
            unsafe {
                libc::kill(-(self.0.id() as i32), libc::SIGKILL);
            }
            let _ = self.0.wait();
        }
    }

    /// A process that died but is not yet reaped by its parent (a zombie that
    /// is not our child, so `waitid` cannot see it) is gone, not alive.
    /// sysinfo reports such a zombie as running on macOS after a refresh,
    /// which left Ian's quit "owned processes remain after SIGKILL".
    #[test]
    fn an_unreaped_zombie_that_is_not_our_child_counts_as_gone() {
        use std::io::BufRead;
        // perl forks a child that lives one second, prints its pid, and never
        // reaps it: the child then stays a zombie of perl, not of this process.
        let mut parent = std::process::Command::new("/usr/bin/perl")
            .args([
                "-e",
                "$|=1; my $c = fork(); if ($c == 0) { sleep 1; exit 0 } print \"$c\\n\"; sleep 30",
            ])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut line = String::new();
        std::io::BufReader::new(parent.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let zombie: u32 = line.trim().parse().unwrap();
        // Snapshot while it runs, as the quit path does before SIGKILL.
        let mut system = sysinfo::System::new();
        system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            true,
            sysinfo::ProcessRefreshKind::new(),
        );
        let started = system
            .process(sysinfo::Pid::from_u32(zombie))
            .map(|p| p.start_time())
            .expect("the child is listed while it runs");
        std::thread::sleep(Duration::from_millis(1500));
        let identities = BTreeMap::from([(zombie, started)]);
        let tracked = [sysinfo::Pid::from_u32(zombie)];
        let live = refresh_owned_alive(&mut system, &tracked, &identities, &mut BTreeSet::new());
        let _ = parent.kill();
        let _ = parent.wait();
        assert!(
            live.is_empty(),
            "an unreaped zombie counted as alive: {live:?}"
        );
    }

    /// A child leading its own process group, carrying `marker`, that ignores
    /// HUP and TERM, forked by a perl parent this host does not own and that
    /// never reaps it. The parent leads its own group too, so dropping the
    /// returned fixture kills it; the guard kills the child. Returns once the
    /// child has installed its ignores.
    #[cfg(target_os = "macos")]
    fn unreaped_marked_child(marker: &str) -> (FixtureChild, KillOnDrop, u32) {
        use std::io::BufRead;
        let script = format!(
            "$|=1; my $c = fork(); if ($c == 0) {{ setpgrp(0, 0); $ENV{{{SESSION_ENV}}} = q({marker}); \
             exec(q(/usr/bin/perl), q(-e), q($|=1; $SIG{{HUP}}=$SIG{{TERM}}=q(IGNORE); print qq(ready\n); sleep 60)) }} \
             print \"pid $c\\n\"; sleep 60"
        );
        let mut command = std::process::Command::new("/usr/bin/perl");
        command
            .args(["-e", &script])
            .env_remove(SESSION_ENV)
            .stdout(std::process::Stdio::piped());
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        let mut parent = FixtureChild(command.spawn().unwrap());
        let lines = std::io::BufReader::new(parent.0.stdout.take().unwrap()).lines();
        let (mut child, mut ready) = (None, false);
        for line in lines {
            let line = line.unwrap();
            if let Some(pid) = line.strip_prefix("pid ") {
                child = Some(pid.trim().parse().unwrap());
            } else if line.trim() == "ready" {
                ready = true;
            }
            if child.is_some() && ready {
                break;
            }
        }
        let child = child.expect("the fixture printed its child's pid");
        (parent, KillOnDrop(child), child)
    }

    /// Kills a fixture process that is not our child when the test ends.
    #[cfg(target_os = "macos")]
    struct KillOnDrop(u32);
    #[cfg(target_os = "macos")]
    impl Drop for KillOnDrop {
        fn drop(&mut self) {
            unsafe {
                libc::kill(self.0 as i32, libc::SIGKILL);
            }
        }
    }

    /// Stint 1523, the stamp decision: a survivor killed by SIGKILL whose
    /// parent is alive and never reaps it is a zombie, not an owned process
    /// left running; the kill path reports nothing remaining and the clean
    /// shutdown is stamped.
    /// macOS only: elsewhere `process_is_zombie` defers to sysinfo, which
    /// reads the state fresh, so there is nothing to check.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_killed_survivor_left_unreaped_still_allows_the_clean_stamp() {
        let marker = uuid::Uuid::new_v4().to_string();
        let (_parent, _child_guard, child) = unreaped_marked_child(&marker);
        let _owned = PaneProcessGroup::record_for(child, &marker);
        let report = stop_marked_processes(
            &marker,
            Duration::from_millis(50),
            Instant::now() + Duration::from_secs(2),
        );
        let zombie = process_is_zombie(child);
        let profile = tempfile::tempdir().unwrap();
        let stamped = crate::app::quit::record_quit_integrity(report.remaining, profile.path());
        assert!(
            report.escalated >= 1,
            "the survivor needed SIGKILL: {report:?}"
        );
        assert!(zombie, "the survivor was killed and left unreaped");
        assert_eq!(
            report.remaining, 0,
            "a killed survivor was counted as running"
        );
        assert!(stamped, "the clean shutdown was not stamped");
    }

    /// Stint 1523, the mirror: an owned process that is still running is
    /// counted, and the clean shutdown is not stamped.
    #[test]
    fn a_running_owned_process_refuses_the_clean_stamp() {
        let marker = uuid::Uuid::new_v4().to_string();
        let mut running = FixtureChild(
            std::process::Command::new("/bin/sleep")
                .arg("60")
                .env(SESSION_ENV, &marker)
                .spawn()
                .unwrap(),
        );
        let pid = running.0.id();
        let mut system = sysinfo::System::new();
        system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            true,
            sysinfo::ProcessRefreshKind::new(),
        );
        let started = system
            .process(sysinfo::Pid::from_u32(pid))
            .map(|p| p.start_time())
            .unwrap();
        let live = refresh_owned_alive(
            &mut system,
            &[sysinfo::Pid::from_u32(pid)],
            &BTreeMap::from([(pid, started)]),
            &mut BTreeSet::new(),
        );
        let profile = tempfile::tempdir().unwrap();
        let stamped = crate::app::quit::record_quit_integrity(live.len(), profile.path());
        let _ = running.0.kill();
        assert_eq!(live, vec![pid]);
        assert!(!stamped, "a running process was stamped clean");
        assert_eq!(
            std::fs::read_dir(profile.path()).unwrap().count(),
            0,
            "no stamp written"
        );
    }

    /// Stint 1509: the kill path never consumes a pane root's exit status.
    /// The root's owner (alacritty's PTY loop, for a terminal pane) reaps it;
    /// when the kill path reaped first, that owner's `try_wait` failed with
    /// ECHILD and logged "Error checking child process termination" on every
    /// clean host stop.
    #[test]
    #[ignore = "requires-pty"]
    fn kill_path_leaves_the_roots_exit_status_to_its_owner() {
        use std::io::BufRead;
        let marker = uuid::Uuid::new_v4().to_string();
        let mut command = std::process::Command::new("/bin/sh");
        command
            .args(["-c", "trap 'exit 0' TERM; echo ready; while :; do :; done"])
            .env(SESSION_ENV, &marker)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        let mut child = FixtureChild(command.spawn().unwrap());
        let mut ready = String::new();
        std::io::BufReader::new(child.0.stdout.take().unwrap())
            .read_line(&mut ready)
            .unwrap();
        assert_eq!(ready.trim(), "ready");
        let _root = PaneProcessGroup::record_for(child.0.id(), &marker);
        let budget = crate::testing::load_aware_timeout(Duration::from_secs(2));
        let report = stop_marked_processes(&marker, budget, Instant::now() + budget);
        assert_eq!(report.groups, 1);
        assert_eq!(report.remaining, 0, "the stopped root counts as gone");
        let status = child
            .0
            .try_wait()
            .expect("the owner can still read its child's exit status")
            .expect("the child has exited");
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(
            status.signal(),
            Some(libc::SIGHUP),
            "stopped by the kill path's HUP"
        );
    }

    #[test]
    #[ignore = "requires-pty"]
    fn term_cooperates_while_another_hosts_group_survives() {
        use std::io::BufRead;
        let marker = uuid::Uuid::new_v4().to_string();
        let other_marker = uuid::Uuid::new_v4().to_string();
        let spawn = |marker: &str| {
            let mut command = std::process::Command::new("/bin/sh");
            command
                .args([
                    "-c",
                    "trap '' HUP; trap 'exit 0' TERM; echo ready; while :; do :; done",
                ])
                .env(SESSION_ENV, marker)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null());
            unsafe {
                command.pre_exec(|| {
                    if libc::setsid() < 0 {
                        Err(std::io::Error::last_os_error())
                    } else {
                        Ok(())
                    }
                });
            }
            let mut child = FixtureChild(command.spawn().unwrap());
            let mut ready = String::new();
            std::io::BufReader::new(child.0.stdout.take().unwrap())
                .read_line(&mut ready)
                .unwrap();
            assert_eq!(ready.trim(), "ready");
            child
        };
        let mut owned = spawn(&marker);
        let mut unrelated = spawn(&other_marker);
        let _owned = PaneProcessGroup::record_for(owned.0.id(), &marker);
        let _other = PaneProcessGroup::record_for(unrelated.0.id(), &other_marker);
        let budget = crate::testing::load_aware_timeout(Duration::from_secs(2));
        let report = stop_marked_processes(&marker, budget, Instant::now() + budget);
        assert_eq!(report.groups, 1);
        assert_eq!(
            report.escalated, 0,
            "TERM-cooperative child must not need KILL"
        );
        match owned.0.wait() {
            Ok(status) => assert!(status.success()),
            Err(error) => assert_eq!(error.raw_os_error(), Some(libc::ECHILD)),
        }
        assert!(
            unrelated.0.try_wait().unwrap().is_none(),
            "another host owns this group"
        );
    }

    #[test]
    #[ignore = "requires-pty"]
    fn kill_switch_stops_hup_and_term_ignoring_detached_groups() {
        let marker = uuid::Uuid::new_v4().to_string();
        let mut children = Vec::new();
        let mut registrations = Vec::new();
        for _ in 0..2 {
            let mut command = std::process::Command::new("/bin/sh");
            command
                .args([
                    "-c",
                    "trap '' HUP TERM; echo ready; while :; do sleep 600; done",
                ])
                .env(SESSION_ENV, &marker)
                .stdout(std::process::Stdio::piped());
            unsafe {
                command.pre_exec(|| {
                    if libc::setsid() < 0 {
                        Err(std::io::Error::last_os_error())
                    } else {
                        Ok(())
                    }
                });
            }
            let mut child = command.spawn().expect("spawn isolated group");
            use std::io::BufRead;
            let mut line = String::new();
            std::io::BufReader::new(child.stdout.take().unwrap())
                .read_line(&mut line)
                .unwrap();
            assert_eq!(line.trim(), "ready");
            registrations.push(PaneProcessGroup::record_for(child.id(), &marker));
            children.push(FixtureChild(child));
        }
        let report = stop_marked_processes(
            &marker,
            Duration::from_millis(30),
            Instant::now() + Duration::from_secs(1),
        );
        assert_eq!(report.groups, 2);
        assert_eq!(report.escalated, 2);
        for mut child in children {
            match child.0.wait() {
                Ok(status) => assert!(!status.success()),
                Err(error) => assert_eq!(error.raw_os_error(), Some(libc::ECHILD)),
            }
        }
    }

    /// Pane roots are interactive shells on a pty, which ignore SIGTERM. Quit
    /// must end them with HUP inside the grace, not by SIGKILL at its end.
    #[test]
    #[ignore = "requires-pty"]
    fn interactive_zsh_pane_root_exits_on_hup_without_escalation() {
        use std::io::{Read, Write};
        use std::os::fd::FromRawFd;
        use std::os::unix::process::ExitStatusExt;
        let Some(zsh) = ["/bin/zsh", "/usr/bin/zsh"]
            .into_iter()
            .map(std::path::Path::new)
            .find(|path| path.exists())
        else {
            eprintln!("skipping: zsh is not installed");
            return;
        };
        let (mut master, slave) = unsafe {
            let (mut master, mut slave) = (0, 0);
            let rc = libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
            assert_eq!(rc, 0, "openpty: {}", std::io::Error::last_os_error());
            (
                std::fs::File::from_raw_fd(master),
                std::fs::File::from_raw_fd(slave),
            )
        };
        let marker = uuid::Uuid::new_v4().to_string();
        let rc_dir = tempfile::tempdir().unwrap();
        let mut command = std::process::Command::new(zsh);
        command
            .args(["-f", "-i"])
            .env(SESSION_ENV, &marker)
            .env("ZDOTDIR", rc_dir.path())
            .stdin(slave.try_clone().unwrap())
            .stdout(slave.try_clone().unwrap())
            .stderr(slave);
        unsafe {
            // Same shape as a pane: own session, pty as controlling terminal.
            command.pre_exec(|| {
                if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        let mut child = FixtureChild(command.spawn().expect("spawn zsh -i"));
        drop(command); // Close our copies of the slave so EOF reaches the reader.
        let (tx, rx) = std::sync::mpsc::channel();
        let mut reader = master.try_clone().unwrap();
        std::thread::spawn(move || {
            let mut buf = [0u8; 1024];
            while let Ok(n @ 1..) = reader.read(&mut buf) {
                if tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        // The expansion only appears once zsh has read and run the line, so
        // its signal dispositions are installed.
        master.write_all(b"echo READY_$((40+2))\n").unwrap();
        let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(10));
        let mut output = Vec::new();
        while !String::from_utf8_lossy(&output).contains("READY_42") {
            let left = deadline.saturating_duration_since(Instant::now());
            output.extend(rx.recv_timeout(left).expect("zsh -i never became ready"));
        }
        // Precondition: the fixture reproduces the bug (TERM alone is ignored).
        unsafe { libc::kill(child.0.id() as i32, libc::SIGTERM) };
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(
            unsafe { libc::kill(child.0.id() as i32, 0) },
            0,
            "zsh -i died on SIGTERM"
        );
        let registration = PaneProcessGroup::record_for(child.0.id(), &marker);

        let start = Instant::now();
        let report = stop_marked_processes(
            &marker,
            Duration::from_secs(5),
            Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(5)),
        );
        let elapsed = start.elapsed();
        drop(registration);
        assert_eq!(report.groups, 1);
        assert_eq!(report.escalated, 0, "zsh -i needed SIGKILL");
        assert!(
            elapsed < crate::testing::load_aware_timeout(Duration::from_secs(2)),
            "teardown took {elapsed:?}"
        );
        match child.0.wait() {
            Ok(status) => assert_ne!(status.signal(), Some(libc::SIGKILL), "{status:?}"),
            Err(error) => assert_eq!(error.raw_os_error(), Some(libc::ECHILD)),
        }
    }

    #[test]
    #[ignore = "requires-pty"]
    fn scrubbed_restart_coordinator_is_not_killed_by_old_session_marker() {
        const FIXTURE_ENV: &str = "PLEXI_TEST_RESTART_COORDINATOR_FIXTURE";
        const EVIDENCE_ENV: &str = "PLEXI_TEST_RESTART_COORDINATOR_EVIDENCE";
        if std::env::var(FIXTURE_ENV).as_deref() == Ok("marked-parent") {
            let _marker = std::env::var(SESSION_ENV).expect("marked fixture marker");
            let evidence = std::path::PathBuf::from(
                std::env::var(EVIDENCE_ENV).expect("fixture evidence path"),
            );
            let env = crate::cli::host::restart_coordinator_env(std::env::vars().collect(), None);
            let mut command = std::process::Command::new("/bin/sleep");
            command
                .arg("600")
                .env_clear()
                .envs(env)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            let coordinator = crate::cli::host::spawn_restart_coordinator(&mut command)
                .expect("spawn reparented coordinator");
            std::fs::write(&evidence, coordinator.to_string()).expect("write coordinator pid");
            // The outer test's marker sweep must kill this marked parent while
            // the scrubbed, double-forked coordinator remains alive.
            loop {
                std::thread::sleep(Duration::from_secs(1));
            }
        }

        let old_marker = uuid::Uuid::new_v4().to_string();
        let evidence = tempfile::NamedTempFile::new().expect("fixture evidence");
        let mut parent = std::process::Command::new(std::env::current_exe().unwrap());
        parent
            .args([
                "--ignored",
                "--exact",
                "host::process_cleanup::tests::scrubbed_restart_coordinator_is_not_killed_by_old_session_marker",
                "--nocapture",
            ])
            .env(SESSION_ENV, &old_marker)
            .env(FIXTURE_ENV, "marked-parent")
            .env(EVIDENCE_ENV, evidence.path())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        unsafe {
            parent.pre_exec(|| {
                if libc::setsid() == -1 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        let mut parent = parent.spawn().expect("spawn marked parent fixture");
        let parent_pid = parent.id() as i32;
        let _registration = PaneProcessGroup::record_for(parent.id(), &old_marker);
        let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(5));
        let coordinator = loop {
            if let Ok(pid) = std::fs::read_to_string(evidence.path()) {
                if let Ok(pid) = pid.trim().parse::<u32>() {
                    break pid;
                }
            }
            assert!(
                Instant::now() < deadline,
                "coordinator fixture did not start"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        let report = stop_marked_processes(
            &old_marker,
            Duration::from_millis(20),
            Instant::now() + Duration::from_secs(1),
        );
        assert!(
            report.groups >= 1,
            "marked parent was not selected: {report:?}"
        );
        match parent.wait() {
            Ok(status) => assert!(!status.success()),
            // The parallel test binary can reap a fixture that its marker
            // sweep selected. Either way, the exact PID must disappear.
            Err(error) if error.raw_os_error() == Some(libc::ECHILD) => {}
            Err(error) => panic!("reap marked parent: {error}"),
        }
        let parent_deadline =
            Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(2));
        while unsafe { libc::kill(parent_pid, 0) } == 0 && Instant::now() < parent_deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let parent_result = unsafe { libc::kill(parent_pid, 0) };
        let parent_error = std::io::Error::last_os_error();
        assert_eq!(
            parent_result, -1,
            "marked parent pid {parent_pid} remained after marker sweep"
        );
        assert_eq!(
            parent_error.raw_os_error(),
            Some(libc::ESRCH),
            "marked parent pid {parent_pid} was not gone"
        );
        let mut system = sysinfo::System::new();
        system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::Some(&[sysinfo::Pid::from_u32(coordinator)]),
            true,
            sysinfo::ProcessRefreshKind::new(),
        );
        assert!(
            system
                .process(sysinfo::Pid::from_u32(coordinator))
                .is_some_and(|process| process.status() != sysinfo::ProcessStatus::Zombie),
            "old host marker sweep killed or zombified reparented coordinator"
        );
        // This is our exact fixture pid, never a broad process lookup.
        unsafe {
            libc::kill(coordinator as i32, libc::SIGKILL);
        }
    }

    #[test]
    #[ignore = "requires-pty"]
    fn host_exit_paths_reap_pane_processes() {
        const CHILD_ENV: &str = "PLEXI_TEST_QUIT_PATH";
        /// Marks that a C exit handler ran (stint 1537: no route may run one).
        extern "C" fn mark_atexit_ran() {
            if let Ok(dir) = std::env::var("PLEXI_TEST_QUIT_EVIDENCE") {
                let _ = std::fs::write(std::path::Path::new(&dir).join("atexit-ran"), "ran");
            }
        }
        if let Ok(mode) = std::env::var(CHILD_ENV) {
            unsafe {
                libc::atexit(mark_atexit_ran);
            }
            let evidence =
                std::path::PathBuf::from(std::env::var("PLEXI_TEST_QUIT_EVIDENCE").unwrap());
            let mut h = crate::testing::HostHarness::new();
            for index in 0..2 {
                let settings = egui_term::BackendSettings {
                    shell: "/bin/sh".into(),
                    args: vec!["-c".into(), r#"
trap '' HUP TERM
printf '%s\n' "$$" > "$PLEXI_TEST_QUIT_EVIDENCE/root-$PLEXI_TEST_INDEX"
nohup /bin/sleep 600 >/dev/null 2>&1 &
printf '%s\n' "$!" > "$PLEXI_TEST_QUIT_EVIDENCE/nohup-$PLEXI_TEST_INDEX"
/usr/bin/python3 -c 'import os,subprocess; p=subprocess.Popen(["/bin/sleep","600"], start_new_session=True); open(os.environ["PLEXI_TEST_QUIT_EVIDENCE"]+"/escaped-"+os.environ["PLEXI_TEST_INDEX"],"w").write(str(p.pid)); p.wait()' &
wait
"#.into()],
                    env: [("PLEXI_TEST_QUIT_EVIDENCE".into(), evidence.display().to_string()),
                          ("PLEXI_TEST_INDEX".into(), index.to_string())].into(),
                    ..Default::default()
                };
                let id = h.app.host.alloc_pane_id();
                let terminal = crate::host::pane::TerminalPane::new(
                    id,
                    h.app.ctx.clone(),
                    h.app.pty_event_tx.clone(),
                    settings,
                    14.0,
                )
                .expect("spawn test terminal");
                h.app.windows[0]
                    .panes
                    .insert(id, crate::host::pane::Pane::Terminal(Box::new(terminal)));
            }
            let deadline = std::time::Instant::now()
                + crate::testing::load_aware_timeout(Duration::from_secs(10));
            while !(evidence.join("escaped-0").exists() && evidence.join("escaped-1").exists()) {
                assert!(
                    std::time::Instant::now() < deadline,
                    "PTY children did not become ready"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            let profile = h.profile_dir().to_path_buf();
            if mode == "host-stop" {
                // `exit_host` answers this token with its real exit outcome
                // (stint 1430); with no socket stream it lands in this file.
                h.inject_ipc(crate::protocol::HostRequest::Shutdown {
                    response_file: Some(evidence.join("stop-reply.json").display().to_string()),
                });
                h.run_hidden_frames(3);
                panic!("host stop returned instead of exiting");
            } else if mode == "update" {
                h.app
                    .request_quit(crate::app::quit::ExitReason::UpdateRestart);
                for _ in 0..3 {
                    h.app
                        .service_quit_with_restart(&eframe::Frame::_new_kittest(), || {
                            assert!(crate::workspace::WorkspaceFile::load_from(&profile).is_ok());
                            assert!(!evidence.join("restart-scheduled").exists());
                            std::fs::write(
                                evidence.join("restart-scheduled"),
                                "scheduled after save",
                            )
                            .unwrap();
                            Ok(())
                        });
                }
                panic!("update returned instead of exiting");
            } else {
                assert!(h.app.prepare_user_quit());
                h.app
                    .request_quit(crate::app::quit::ExitReason::WindowClosed);
                h.run_hidden_frames(3);
                panic!("window close returned instead of exiting");
            }
        }
        for mode in ["window", "host-stop", "update"] {
            let evidence = tempfile::tempdir().unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--ignored",
                    "--exact",
                    "host::process_cleanup::tests::host_exit_paths_reap_pane_processes",
                    "--nocapture",
                ])
                .env(CHILD_ENV, mode)
                .env("PLEXI_TEST_QUIT_EVIDENCE", evidence.path())
                .env("PLEXI_QUIT_GRACE_MS", "30")
                .output()
                .expect("run isolated host shutdown");
            if mode == "update" {
                assert!(evidence.path().join("restart-scheduled").exists());
            }
            assert!(
                !evidence.path().join("atexit-ran").exists(),
                "mode {mode}: a C exit handler ran, so exit_host did not end in _exit"
            );
            if mode == "host-stop" {
                // The reply `exit_host` wrote matches how the process exited.
                let reply: serde_json::Value = serde_json::from_slice(
                    &std::fs::read(evidence.path().join("stop-reply.json"))
                        .expect("exit_host wrote the host stop reply"),
                )
                .expect("the reply is JSON");
                assert_eq!(reply["ok"], true, "{reply}");
                assert_eq!(
                    reply["status"],
                    output.status.code().expect("exit code"),
                    "{reply}"
                );
                assert_eq!(reply["workspace"], "saved", "{reply}");
                assert_eq!(reply["remaining_processes"], 0, "{reply}");
                assert_eq!(reply["socket_released"], true, "{reply}");
            }
            eprintln!(
                "quit fixture mode={mode}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let mut alive = Vec::new();
            for kind in ["root", "nohup", "escaped"] {
                for index in 0..2 {
                    let pid: i32 =
                        std::fs::read_to_string(evidence.path().join(format!("{kind}-{index}")))
                            .expect("child pid evidence")
                            .trim()
                            .parse()
                            .unwrap();
                    let deadline = std::time::Instant::now()
                        + crate::testing::load_aware_timeout(Duration::from_secs(2));
                    while unsafe { libc::kill(pid, 0) } == 0 && std::time::Instant::now() < deadline
                    {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    if unsafe { libc::kill(pid, 0) } == 0 {
                        alive.push((kind, pid));
                        // Failure cleanup only; never leave our fixture running.
                        unsafe {
                            libc::kill(pid, libc::SIGKILL);
                        }
                    }
                }
            }
            assert!(
                output.status.success(),
                "{mode}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(alive.is_empty(), "{mode} left processes alive: {alive:?}");
        }
    }
}
