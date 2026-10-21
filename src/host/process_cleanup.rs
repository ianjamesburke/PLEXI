//! Pane-owned process teardown. PTY launch uses setsid (alacritty), so the
//! recorded root pid is also its process-group id. The host marker follows
//! children across nohup, setsid and reparenting; it is never inherited by a
//! host launched through `plexi host start`.
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

#[derive(Default, Debug)]
pub(crate) struct KillReport {
    pub groups: usize,
    pub escalated: usize,
}

pub(crate) fn stop_pane_processes() -> KillReport {
    let grace = match std::env::var("PLEXI_QUIT_GRACE_MS") {
        Ok(value) => match value.parse::<u64>() {
            Ok(ms) => Duration::from_millis(ms.min(30_000)),
            Err(error) => {
                log::warn!("quit: invalid PLEXI_QUIT_GRACE_MS: {error}");
                Duration::from_secs(2)
            }
        },
        Err(_) => Duration::from_secs(2),
    };
    stop_marked_tmux_sessions(session_marker());
    stop_marked_processes(session_marker(), grace)
}

// Shutdown helpers must not let an unresponsive tmux server stall host exit.
fn quit_helper(args: &[&str], deadline: Instant) -> std::io::Result<std::process::Output> {
    if Instant::now() >= deadline {
        return Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "quit helper deadline elapsed",
        ));
    }
    let mut child = std::process::Command::new(args[0])
        .args(&args[1..])
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
fn stop_marked_tmux_sessions(marker: &str) {
    let deadline = Instant::now() + Duration::from_millis(500);
    let sessions = match quit_helper(&["tmux", "list-sessions", "-F", "#{session_id}"], deadline) {
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
            &["tmux", "show-environment", "-t", session, SESSION_ENV],
            deadline,
        ) {
            Ok(output) => output,
            Err(e) => {
                log::warn!("quit: tmux environment query failed session={session}: {e}");
                continue;
            }
        };
        if env.status.success() && String::from_utf8_lossy(&env.stdout).trim() == expected {
            match quit_helper(&["tmux", "kill-session", "-t", session], deadline) {
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
fn stop_marked_processes(marker: &str, grace: Duration) -> KillReport {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
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
        if unsafe { libc::kill(target, signal) } != 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                log::warn!("quit: signal={signal} target={target} failed: {error}");
            }
        }
    };
    for group in &groups {
        signal(-*group, libc::SIGTERM);
    }
    for pid in &owned {
        if !process_groups
            .get(pid)
            .is_some_and(|group| groups.contains(group))
        {
            signal(*pid as i32, libc::SIGTERM);
        }
    }
    let deadline = std::time::Instant::now() + grace;
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
                system
                    .process(sysinfo::Pid::from_u32(*pid))
                    .filter(|p| {
                        p.start_time() == *start && p.status() != sysinfo::ProcessStatus::Zombie
                    })
                    .map(|_| *pid)
            })
            .collect();
        if live.is_empty() || std::time::Instant::now() >= deadline {
            break live;
        }
        std::thread::sleep(Duration::from_millis(25));
    };
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
    }
}

#[cfg(not(unix))]
fn stop_marked_processes(_marker: &str, _grace: Duration) -> KillReport {
    log::warn!("quit: explicit pane process-group teardown requires Unix");
    KillReport::default()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::process::CommandExt;

    #[test]
    fn unresponsive_quit_helper_is_killed_at_deadline() {
        let start = Instant::now();
        let error =
            quit_helper(&["/bin/sleep", "600"], start + Duration::from_millis(30)).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    struct FixtureChild(std::process::Child);
    impl Drop for FixtureChild {
        fn drop(&mut self) {
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
        let report = stop_marked_processes(&marker, Duration::from_millis(30));
        assert_eq!(report.groups, 2);
        assert_eq!(report.escalated, 2);
        for mut child in children {
            assert!(!child.0.wait().expect("reap").success());
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
