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
}
impl PaneProcessGroup {
    pub(crate) fn record(pid: u32) -> Self {
        Self::record_for(pid, session_marker())
    }

    fn record_for(pid: u32, marker: &str) -> Self {
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
                    ROOTS.lock().unwrap_or_else(|e| e.into_inner()).insert(
                        pid,
                        RootProcess {
                            pgid,
                            started: process.start_time(),
                            marker: marker.into(),
                        },
                    );
                } else {
                    log::warn!("pane process identity unavailable pid={pid}");
                }
                log::info!("pane process group registered pid={pid} pgid={pgid}");
            } else {
                log::warn!("pane process group not isolated pid={pid} pgid={pgid}");
            }
        }
        #[cfg(not(unix))]
        let _ = (pid, marker);
        Self {
            #[cfg(unix)]
            pid,
        }
    }
}
#[cfg(unix)]
impl Drop for PaneProcessGroup {
    fn drop(&mut self) {
        ROOTS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.pid);
    }
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
}

/// `default_grace` is the caller's budget between HUP/TERM and SIGKILL;
/// `PLEXI_QUIT_GRACE_MS` overrides it when set.
pub(crate) fn stop_pane_processes(default_grace: Duration) -> KillReport {
    stop_pane_processes_for_marker(session_marker(), default_grace)
}

/// Stop pane groups belonging to `marker`, bounded across process and tmux
/// cleanup. This is also the socket-failure fallback used by `host stop`.
pub(crate) fn stop_pane_processes_for_marker(marker: &str, default_grace: Duration) -> KillReport {
    stop_pane_processes_with_tmux(marker, default_grace, "tmux")
}

fn stop_pane_processes_with_tmux(marker: &str, default_grace: Duration, tmux: &str) -> KillReport {
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
    let deadline = start + (grace + Duration::from_millis(250)).min(Duration::from_millis(2500));
    let mut report = stop_marked_processes(marker, grace, deadline);
    // Pane HUP/TERM must never wait on a stuck tmux server. Tmux consumes only
    // the remaining shared budget after pane groups have been signaled.
    let tmux_start = Instant::now();
    stop_marked_tmux_sessions(tmux, marker, deadline);
    report.tmux = tmux_start.elapsed();
    KillReport { grace, ..report }
}

// Shutdown helpers must not let an unresponsive tmux server stall host exit.
fn quit_helper(
    program: &str,
    args: &[&str],
    deadline: Instant,
) -> std::io::Result<std::process::Output> {
    if Instant::now() >= deadline {
        return Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "quit helper deadline elapsed",
        ));
    }
    let mut child = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    loop {
        if child.try_wait()?.is_some() {
            return child.wait_with_output();
        }
        if Instant::now() >= deadline {
            child.kill()?;
            child.wait()?;
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "quit helper timed out",
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// An existing tmux server does not necessarily propagate a client's env to
/// sessions. Only an explicit matching session environment proves ownership;
/// never kill an unmarked shared server or a user-selected custom socket.
fn stop_marked_tmux_sessions(tmux: &str, marker: &str, deadline: Instant) {
    let sessions = match quit_helper(tmux, &["list-sessions", "-F", "#{session_id}"], deadline) {
        Ok(output) if output.status.success() => output,
        Ok(_) => return, // No default server is a normal state.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
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
            Ok(output) => output,
            Err(e) => {
                log::warn!("quit: tmux environment query failed session={session}: {e}");
                continue;
            }
        };
        if env.status.success() && String::from_utf8_lossy(&env.stdout).trim() == expected {
            match quit_helper(tmux, &["kill-session", "-t", session], deadline) {
                Ok(output) if output.status.success() => {
                    log::info!("quit: stopped marked tmux session={session}")
                }
                Ok(output) => log::warn!(
                    "quit: tmux kill-session failed session={session} status={}",
                    output.status
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
        ProcessRefreshKind::new().with_environ(UpdateKind::Always),
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
    // HUP first, like a terminal going away: interactive shells ignore TERM
    // but exit (saving history) on HUP. TERM follows for daemons that only
    // honor TERM.
    for sig in [libc::SIGHUP, libc::SIGTERM] {
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
    let deadline = (signaled + grace).min(total_deadline);
    let tracked: Vec<_> = identities
        .keys()
        .map(|pid| sysinfo::Pid::from_u32(*pid))
        .collect();
    let survivors = loop {
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&tracked),
            true,
            ProcessRefreshKind::new(),
        );
        let live: Vec<u32> = identities
            .iter()
            .filter_map(|(pid, start)| {
                let mut status = 0;
                // A pane root is normally our child. Reap it when it exits:
                // macOS can otherwise retain an unreaped zombie in sysinfo's
                // process table for the entire quit grace period.
                if unsafe { libc::waitpid(*pid as i32, &mut status, libc::WNOHANG) } == *pid as i32
                {
                    return None;
                }
                system
                    .process(sysinfo::Pid::from_u32(*pid))
                    .filter(|p| {
                        p.start_time() == *start && p.status() != sysinfo::ProcessStatus::Zombie
                    })
                    .map(|_| *pid)
            })
            .collect();
        if live.is_empty() || Instant::now() >= deadline {
            break live;
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    let waited = Instant::now();
    let mut escalated = BTreeSet::new();
    for pid in survivors {
        let group = unsafe { libc::getpgid(pid as i32) };
        if groups.contains(&group) {
            escalated.insert(group);
        }
        signal(pid as i32, libc::SIGKILL);
    }
    // Group delivery also catches children forked during the grace interval.
    for group in &escalated {
        signal(-*group, libc::SIGKILL);
    }
    KillReport {
        groups: groups.len(),
        escalated: escalated.len(),
        scan: scanned - start,
        signal: signaled - scanned,
        grace_waited: waited - signaled,
        kill: waited.elapsed(),
        ..KillReport::default()
    }
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

    #[test]
    fn exited_or_unowned_signal_targets_are_expected() {
        assert!(signal_target_unavailable(Some(libc::ESRCH)));
        assert!(signal_target_unavailable(Some(libc::EPERM)));
        assert!(!signal_target_unavailable(Some(libc::EINVAL)));
        assert!(!signal_target_unavailable(None));
    }

    #[test]
    fn unresponsive_quit_helper_is_killed_at_deadline() {
        let start = Instant::now();
        let error =
            quit_helper("/bin/sleep", &["600"], start + Duration::from_millis(30)).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
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
        let ready_deadline = Instant::now() + Duration::from_secs(10);
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
        );
        assert!(
            start.elapsed() < Duration::from_millis(450),
            "cleanup exceeded its 280ms cap"
        );
        assert!(
            report.tmux < Duration::from_millis(300),
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

    #[test]
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
        let deadline = Instant::now() + Duration::from_secs(10);
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
            Instant::now() + Duration::from_secs(5),
        );
        let elapsed = start.elapsed();
        drop(registration);
        assert_eq!(report.groups, 1);
        assert_eq!(report.escalated, 0, "zsh -i needed SIGKILL");
        assert!(
            elapsed < Duration::from_secs(2),
            "teardown took {elapsed:?}"
        );
        match child.0.wait() {
            Ok(status) => assert_ne!(status.signal(), Some(libc::SIGKILL), "{status:?}"),
            Err(error) => assert_eq!(error.raw_os_error(), Some(libc::ECHILD)),
        }
    }

    #[test]
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
        let deadline = Instant::now() + Duration::from_secs(5);
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
        let parent_deadline = Instant::now() + Duration::from_secs(2);
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
    fn host_exit_paths_reap_pane_processes() {
        const CHILD_ENV: &str = "PLEXI_TEST_QUIT_PATH";
        if let Ok(mode) = std::env::var(CHILD_ENV) {
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
                .unwrap();
                h.app.windows[0]
                    .panes
                    .insert(id, crate::host::pane::Pane::Terminal(Box::new(terminal)));
            }
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while !(evidence.join("escaped-0").exists() && evidence.join("escaped-1").exists()) {
                assert!(
                    std::time::Instant::now() < deadline,
                    "PTY children did not become ready"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            if mode == "host-stop" {
                h.inject_ipc(crate::protocol::AppRequest::Shutdown);
                h.hidden_frame();
                panic!("host stop returned instead of exiting");
            } else {
                assert!(h.app.prepare_user_quit());
                // Eframe's real window-close callback. The harness does not
                // destroy an OS window, so invoke that final callback directly.
                eframe::App::on_exit(&mut h.app);
                panic!("window close returned instead of exiting");
            }
        }
        for mode in ["window", "host-stop"] {
            let evidence = tempfile::tempdir().unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "host::process_cleanup::tests::host_exit_paths_reap_pane_processes",
                    "--nocapture",
                ])
                .env(CHILD_ENV, mode)
                .env("PLEXI_TEST_QUIT_EVIDENCE", evidence.path())
                .env("PLEXI_QUIT_GRACE_MS", "30")
                .output()
                .expect("run isolated host shutdown");
            let mut alive = Vec::new();
            for kind in ["root", "nohup", "escaped"] {
                for index in 0..2 {
                    let pid: i32 =
                        std::fs::read_to_string(evidence.path().join(format!("{kind}-{index}")))
                            .expect("child pid evidence")
                            .trim()
                            .parse()
                            .unwrap();
                    let deadline = std::time::Instant::now() + Duration::from_secs(2);
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
