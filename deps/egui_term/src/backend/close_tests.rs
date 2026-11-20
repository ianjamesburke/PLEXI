//! Stint 1650: closing a pane. The contract is in `lifecycle`.
use std::cell::Cell;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use alacritty_terminal::event::{OnResize, WindowSize};
use alacritty_terminal::event_loop::EventLoop;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Term;
use alacritty_terminal::tty::{ChildEvent, EventedPty, EventedReadWrite};

use super::lifecycle::testing::{self as worker, Event, Site};
use super::lifecycle::{shutdown_panes, ShutdownOutcome};
use super::pane_pty::{PTY_CHILD_EVENT_TOKEN, PTY_READ_WRITE_TOKEN};
use super::settings::BackendSettings;
use super::{
    EventProxy, PtyEvent, PtyExitStatus, TerminalBackend, TerminalSize,
};

thread_local! {
    pub(super) static FAIL_AT: Cell<Option<&'static str>> = const { Cell::new(None) };
    /// The last pty prepared on this thread, by its master's identity.
    static MASTER: Cell<Option<u64>> = const { Cell::new(None) };
    /// The last shell spawned on this thread.
    static SPAWNED: Cell<Option<u32>> = const { Cell::new(None) };
}

/// How far past its deadline host stop may return.
const EPSILON: Duration = Duration::from_millis(50);

/// The pane whose reader's second poll registration fails.
static FAIL_SECOND_ADD: AtomicU64 = AtomicU64::new(u64::MAX);
/// Rollbacks of a first registration: (pane, master still open then,
/// master no longer registered with the poller).
static UNDONE: Mutex<Vec<(u64, bool, bool)>> = Mutex::new(Vec::new());

pub(super) fn note_master(master: &std::fs::File) {
    MASTER.with(|m| m.set(master_id(master.as_raw_fd())));
}

pub(super) fn note_spawn(pid: u32) {
    SPAWNED.with(|spawned| spawned.set(Some(pid)));
}

pub(super) fn fail_second_add(pane: u64) -> bool {
    FAIL_SECOND_ADD.load(Ordering::SeqCst) == pane
}

pub(super) fn note_undo(
    pane: u64,
    poll: &polling::Poller,
    master: &std::fs::File,
) {
    let open = unsafe { libc::fcntl(master.as_raw_fd(), libc::F_GETFD) } != -1;
    // Deleting again fails only if the rollback really deleted it.
    let deleted = poll.delete(master).is_err();
    UNDONE.lock().unwrap().push((pane, open, deleted));
}

/// What identifies one pty's master among this process's fds: macOS gives
/// each master its own device number; Linux masters opened through ptmx
/// share one, so there it is the pty index.
fn master_id(fd: RawFd) -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let mut index: libc::c_uint = 0;
        (unsafe { libc::ioctl(fd, libc::TIOCGPTN, &mut index) } == 0)
            .then_some(u64::from(index))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        ((unsafe { libc::fstat(fd, &mut stat) }) == 0
            && stat.st_mode & libc::S_IFMT == libc::S_IFCHR)
            .then_some(stat.st_rdev as u64)
    }
}

/// Open fds of this process on the master identified by `master`.
fn open_masters(master: u64) -> usize {
    (0..unsafe { libc::getdtablesize() })
        .filter(|fd| master_id(*fd) == Some(master))
        .count()
}

fn settings(shell: &str, script: &str) -> BackendSettings {
    BackendSettings {
        shell: shell.to_string(),
        args: vec!["-c".to_string(), script.to_string()],
        ..Default::default()
    }
}

fn spawn(script: &str) -> (TerminalBackend, Receiver<(u64, PtyEvent)>) {
    let (tx, rx) = mpsc::channel();
    let backend = TerminalBackend::new(
        1650,
        egui::Context::default(),
        tx,
        settings("/bin/sh", script),
    )
    .expect("spawn backend");
    (backend, rx)
}

fn temp(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    std::env::temp_dir().join(format!(
        "egui-term-1650-{}-{}-{name}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn read_pid(path: &Path) -> libc::pid_t {
    wait_until("a pid file", || {
        std::fs::read_to_string(path).is_ok_and(|s| s.ends_with('\n'))
    });
    let pid = std::fs::read_to_string(path)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let _ = std::fs::remove_file(path);
    pid
}

fn gone(pid: libc::pid_t) -> bool {
    // ESRCH also means reaped: a zombie still answers `kill(pid, 0)`.
    (unsafe { libc::kill(pid, 0) }) != 0
        && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The event loop's proxy drops only after the pty, so the receiver
/// disconnecting proves the reader and the subscription both retired.
fn wait_retired(rx: &Receiver<(u64, PtyEvent)>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            Ok(_) => continue,
            Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {
                panic!("PTY reader of the closed pane still running after 10 s")
            },
        }
    }
}

fn wait_exit(rx: &Receiver<(u64, PtyEvent)>) -> PtyExitStatus {
    loop {
        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok((_, PtyEvent::Exit(status))) => return status,
            Ok(_) => continue,
            Err(error) => panic!("no exit event: {error}"),
        }
    }
}

/// The owner signalled only the shell's original group, sent KILL, and
/// reaped the shell exactly once, after its last signal.
fn assert_owned_signals_then_one_reap(key: u64, pid: libc::pid_t) {
    wait_until("the shell to be reaped", || {
        worker::events(key).contains(&Event::Reaped)
    });
    let events = worker::events(key);
    assert_eq!(events.last(), Some(&Event::Reaped), "{events:?}");
    assert_eq!(
        events.iter().filter(|e| **e == Event::Reaped).count(),
        1,
        "{events:?}"
    );
    for event in &events {
        if let Event::Signal(target, _) = event {
            assert_eq!(
                *target, -pid,
                "signal outside the shell's group: {events:?}"
            );
        }
    }
    assert!(
        events.contains(&Event::Signal(-pid, libc::SIGKILL)),
        "{events:?}"
    );
}

/// Runs the calling test in a child process, alone, and returns `false`
/// there it should stop; in the child it returns `true`. For tests that
/// fill the fd table, depend on fd numbering, arm the next launch, or stop
/// the host.
fn isolated(name: &str) -> bool {
    const CHILD: &str = "EGUI_TERM_1650_ISOLATED";
    if std::env::var_os(CHILD).is_some() {
        return true;
    }
    let dir = std::env::temp_dir();
    let log =
        dir.join(format!("egui-term-1650-{}-{name}.log", std::process::id()));
    // One open file for both streams, so they share an offset.
    let out = std::fs::File::create(&log).unwrap();
    let mut child =
        std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                &format!("backend::close_tests::{name}"),
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD, "1")
            .stdout(out.try_clone().unwrap())
            .stderr(out)
            .spawn()
            .unwrap();
    // A watchdog: a hang (say a stop blocked behind a spawn) fails.
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("isolated run hung for 60 s: {name}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let text = std::fs::read_to_string(&log).unwrap_or_default();
    let _ = std::fs::remove_file(&log);
    assert!(status.success(), "isolated run failed:\n{text}");
    assert!(
        text.contains("1 passed"),
        "isolated test did not run:\n{text}"
    );
    false
}

/// End a fixture shell the owner no longer owns.
fn end_fixture(pid: libc::pid_t) {
    unsafe { libc::kill(pid, libc::SIGKILL) };
    let mut status = 0;
    unsafe { libc::waitpid(pid, &mut status, 0) };
}

/// A HUP-ignoring program flooding the tty: the measured Beta leak, where
/// the exiting shell waited for its output to drain while the pty waited
/// for the shell with the master open. Closing releases both masters,
/// retires the reader and subscription, reaps the shell, and leaves the
/// next pane working.
#[test]
fn closing_a_flooding_hangup_ignoring_pane_retires_everything() {
    let (neighbour, neighbour_rx) = spawn("exec cat");
    let (backend, rx) = spawn("trap '' HUP; exec yes");
    let (pid, key) = (backend.child_pid() as libc::pid_t, backend.launch_key());
    let master = MASTER.with(|m| m.get()).unwrap();
    rx.recv_timeout(Duration::from_secs(5))
        .expect("the child never produced output");
    assert_eq!(
        open_masters(master),
        2,
        "the pty and its tap hold the master"
    );

    drop(backend);
    wait_retired(&rx);
    assert_owned_signals_then_one_reap(key, pid);
    assert!(gone(pid));
    assert_eq!(
        open_masters(master),
        0,
        "a master of the closed pty is open"
    );

    let mut neighbour = neighbour;
    neighbour.process_command(super::BackendCommand::Write(
        b"still-here\n".to_vec(),
    ));
    wait_until("the next pane to echo", || {
        while neighbour_rx.try_recv().is_ok() {}
        neighbour.capture_lines(5).join("\n").contains("still-here")
    });
}

/// After the shell exits on its own: its status is reported, it stays
/// pinned (unreaped) while the pane is open, a job left in its group ends
/// on close, and it is reaped once, with nothing signalled after.
#[test]
fn a_natural_exit_keeps_its_status_and_is_reaped_once_on_close() {
    let job_file = temp("job");
    let (backend, rx) = spawn(&format!(
        "trap '' HUP; sleep 3600 & echo $! > '{}'; exit 3",
        job_file.display()
    ));
    let (shell, key) =
        (backend.child_pid() as libc::pid_t, backend.launch_key());
    assert_eq!(wait_exit(&rx), PtyExitStatus::Code(3));
    let job = read_pid(&job_file);
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        !gone(shell),
        "shell {shell} was reaped while its pane is open"
    );
    assert!(!gone(job), "job {job} ended before its pane closed");
    assert!(
        !worker::events(key)
            .iter()
            .any(|e| matches!(e, Event::Signal(..))),
        "signalled before close"
    );

    drop(backend);
    wait_until("the job left in the shell's group to end", || gone(job));
    assert_owned_signals_then_one_reap(key, shell);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        worker::events(key).last(),
        Some(&Event::Reaped),
        "acted after the reap"
    );
}

/// Only the shell's group is signalled. With job control, the foreground
/// job ends on the terminal hangup itself; a redirected, HUP-ignoring job
/// in its own group and a process that left the session survive, by
/// contract, as does another pane.
#[test]
fn close_signals_only_the_shells_own_group() {
    let (background, detached) = (temp("background"), temp("detached"));
    let (neighbour, _neighbour_rx) = spawn("exec sleep 3600");
    let (backend, rx) = spawn(&format!(
        "set -m; trap '' HUP; sleep 3600 >/dev/null 2>&1 & echo $! > '{}'; \
         perl -MPOSIX -e 'fork and exit; setsid or die; open F, \">\", $ARGV[0]; \
         print F \"$$\\n\"; close F; sleep 3600' '{}'; yes",
        background.display(),
        detached.display()
    ));
    let (shell, key) =
        (backend.child_pid() as libc::pid_t, backend.launch_key());
    let (background, detached) = (read_pid(&background), read_pid(&detached));
    let mut foreground = shell;
    wait_until("the foreground job", || {
        foreground = backend.foreground_pid().unwrap_or(shell);
        foreground != shell
    });
    assert_ne!(unsafe { libc::getpgid(background) }, shell);

    drop(backend);
    wait_retired(&rx);
    assert_owned_signals_then_one_reap(key, shell);
    wait_until("the foreground job to end on hangup", || gone(foreground));
    let survivors = [background, detached].map(|pid| !gone(pid));
    for pid in [background, detached] {
        unsafe { libc::kill(pid, libc::SIGKILL) };
    }
    assert_eq!(survivors, [true, true], "background, detached");
    assert!(
        !gone(neighbour.child_pid() as libc::pid_t),
        "another pane ended"
    );
}

/// A shell not yet waitable after its kill stays owned, retried without
/// blocking the worker, until it can be reaped.
#[test]
fn a_shell_not_yet_waitable_stays_owned_until_reaped() {
    let (backend, rx) = spawn("trap '' HUP; exec sleep 3600");
    let (shell, key) =
        (backend.child_pid() as libc::pid_t, backend.launch_key());
    worker::set_pending(key, 10);
    drop(backend);
    wait_retired(&rx);
    assert_owned_signals_then_one_reap(key, shell);
    let events = worker::events(key);
    let pending = events.iter().filter(|e| **e == Event::ReapPending).count();
    assert_eq!(pending, 10, "{events:?}");
    assert!(gone(shell));
}

/// A constructor that fails at any step returns promptly with the worker
/// held, and leaves no master, shell or reader. That includes a failure to
/// start the reader's thread, which alacritty would have panicked on, and a
/// second poll registration failing, whose first registration is undone
/// exactly once, while the master is still open.
#[test]
fn failed_starts_return_at_once_and_leave_nothing() {
    for step in [
        "nonblocking",
        "wake",
        "poller",
        "subscription",
        "launcher",
        "registration",
        "shell",
    ] {
        let pane = 1650_000 + step.len() as u64 * 7 + step.as_bytes()[0] as u64;
        MASTER.with(|m| m.set(None));
        SPAWNED.with(|s| s.set(None));
        match step {
            "registration" => FAIL_SECOND_ADD.store(pane, Ordering::SeqCst),
            "shell" => {},
            step => FAIL_AT.with(|at| at.set(Some(step))),
        }
        let held = worker::HOLD.lock().unwrap_or_else(|e| e.into_inner());
        let started = Instant::now();
        let (tx, rx) = mpsc::channel();
        let shell = if step == "shell" {
            "/nonexistent/1650"
        } else {
            "/bin/sh"
        };
        let result = TerminalBackend::new(
            pane,
            egui::Context::default(),
            tx,
            settings(shell, "trap '' HUP; exec sleep 3600"),
        );
        FAIL_AT.with(|at| at.set(None));
        FAIL_SECOND_ADD.store(u64::MAX, Ordering::SeqCst);
        assert!(result.is_err(), "{step}: the failure did not fail");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{step}: the constructor waited {:?}",
            started.elapsed()
        );
        drop(held);
        assert_eq!(
            SPAWNED.with(|s| s.get()),
            None,
            "{step}: a shell was spawned"
        );
        if let Some(master) = MASTER.with(|m| m.get()) {
            wait_until(&format!("{step}: the masters to close"), || {
                open_masters(master) == 0
            });
        }
        wait_retired(&rx);
        let undone: Vec<_> = UNDONE
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _, _)| *p == pane)
            .copied()
            .collect();
        if step == "registration" {
            assert_eq!(
                undone,
                [(pane, true, true)],
                "the first registration must be deleted once, before the master closes"
            );
        } else {
            assert!(undone.is_empty(), "{step}: {undone:?}");
        }
    }
}

/// A pane whose pty closed answers no foreground query, even once its old
/// master number belongs to another pane's pty.
#[test]
fn a_closed_pane_cannot_query_its_neighbour() {
    if !isolated("a_closed_pane_cannot_query_its_neighbour") {
        return;
    }
    let (exited, rx) = spawn("exit 0");
    let old = exited.foreground.read().unwrap().expect("open master");
    wait_exit(&rx);
    // The number is free once the pty closed both its masters.
    wait_until("the exited pane's pty to close", || unsafe {
        libc::fcntl(old, libc::F_GETFD) == -1
    });
    let (neighbour, _neighbour_rx) = spawn("exec sleep 3600");
    assert_eq!(
        *neighbour.foreground.read().unwrap(),
        Some(old),
        "precondition: the neighbour's master reuses the old number"
    );
    assert!(neighbour.foreground_pid().is_some());
    assert_eq!(exited.foreground_pid(), None);
}

/// Closing needs no new fd: with the fd table full, a close still ends a
/// HUP-ignoring, flooding child and retires its reader.
#[test]
fn closing_with_the_fd_table_full_still_ends_the_child() {
    if !isolated("closing_with_the_fd_table_full_still_ends_the_child") {
        return;
    }
    let (backend, rx) = spawn("trap '' HUP; exec yes");
    let (pid, key) = (backend.child_pid() as libc::pid_t, backend.launch_key());
    rx.recv_timeout(Duration::from_secs(5))
        .expect("the child never produced output");
    let mut limit: libc::rlimit = unsafe { std::mem::zeroed() };
    assert_eq!(
        unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) },
        0
    );
    limit.rlim_cur = limit.rlim_cur.min(1024);
    assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) }, 0);
    let mut held = Vec::new();
    loop {
        match std::fs::File::open("/dev/null") {
            Ok(file) => held.push(file),
            Err(error) => {
                assert_eq!(error.raw_os_error(), Some(libc::EMFILE));
                break;
            },
        }
    }
    drop(backend);
    wait_retired(&rx);
    drop(held);
    assert_owned_signals_then_one_reap(key, pid);
}

/// Host stop closes panes that are still open through their owner, seals
/// admission, and reports complete once every shell is reaped.
#[test]
fn host_stop_closes_open_panes_through_their_owner() {
    if !isolated("host_stop_closes_open_panes_through_their_owner") {
        return;
    }
    let (backend, _rx) = spawn("trap '' HUP; exec sleep 3600");
    let (shell, key) =
        (backend.child_pid() as libc::pid_t, backend.launch_key());
    let outcome = shutdown_panes(Instant::now() + Duration::from_secs(2));
    assert_eq!(
        outcome,
        ShutdownOutcome::Complete {
            owned: vec![shell as u32]
        }
    );
    assert_owned_signals_then_one_reap(key, shell);
    assert!(gone(shell));
    let (tx, _rx2) = mpsc::channel();
    let refused = TerminalBackend::new(
        1651,
        egui::Context::default(),
        tx,
        settings("/bin/sh", "exec sleep 3600"),
    );
    assert!(refused.is_err(), "a pane was admitted after host stop");
    assert_eq!(
        SPAWNED.with(|s| s.get()),
        Some(shell as u32),
        "a shell was spawned"
    );
    drop(backend);
}

/// Work the owner cannot finish by host stop's deadline is reported, never
/// acknowledged as clean, and still owned: it is reaped once the reader
/// lets its masters go.
#[test]
fn host_stop_reports_unresolved_work_and_keeps_owning_it() {
    if !isolated("host_stop_reports_unresolved_work_and_keeps_owning_it") {
        return;
    }
    let (backend, rx) = spawn("trap '' HUP; exec sleep 3600");
    let (shell, key) =
        (backend.child_pid() as libc::pid_t, backend.launch_key());
    let reader = worker::READER_HOLD
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    drop(backend);
    let outcome = shutdown_panes(Instant::now() + Duration::from_millis(300));
    assert!(
        matches!(
            &outcome,
            ShutdownOutcome::Incomplete { unresolved, .. } if unresolved.len() == 1
        ),
        "{outcome:?}"
    );
    assert!(outcome.unresolved() > 0);
    assert!(
        !worker::events(key)
            .iter()
            .any(|e| matches!(e, Event::Signal(..))),
        "signalled with the masters open"
    );
    drop(reader);
    wait_retired(&rx);
    assert_owned_signals_then_one_reap(key, shell);
}

/// R4.2: a host stop that meets the owner right after a consuming wait gets
/// no clean answer, and nothing is signalled for that launch after its
/// reap: the owner was consumed by the wait.
#[test]
fn host_stop_during_a_reap_is_unconfirmed_and_nothing_follows_it() {
    if !isolated(
        "host_stop_during_a_reap_is_unconfirmed_and_nothing_follows_it",
    ) {
        return;
    }
    let (backend, rx) = spawn("trap '' HUP; exec sleep 3600");
    let key = backend.launch_key();
    let after_reap =
        worker::AFTER_REAP.lock().unwrap_or_else(|e| e.into_inner());
    drop(backend);
    wait_retired(&rx);
    wait_until("the consuming wait", || {
        worker::events(key).contains(&Event::Reaped)
    });
    let started = Instant::now();
    let outcome = shutdown_panes(started + Duration::from_millis(200));
    assert!(
        started.elapsed() < Duration::from_millis(200) + EPSILON,
        "host stop outlived its deadline: {:?}",
        started.elapsed()
    );
    assert_eq!(outcome, ShutdownOutcome::Unconfirmed);
    assert!(outcome.unresolved() > 0, "a timeout was taken as clean");
    drop(after_reap);
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(
        worker::events(key).last(),
        Some(&Event::Reaped),
        "acted for a reaped launch: {:?}",
        worker::events(key)
    );
}

/// R4.3: a wait error on the observation path retires the launch for
/// good: a retained pane closed later is never probed, signalled or waited
/// on again, so a reused pid cannot be reached through it.
#[test]
fn a_pin_lost_on_observation_is_never_used_again() {
    if !isolated("a_pin_lost_on_observation_is_never_used_again") {
        return;
    }
    worker::fail_next_launch_probe_once(Site::Observe);
    let (backend, rx) = spawn("exec sleep 3600");
    let (shell, key) =
        (backend.child_pid() as libc::pid_t, backend.launch_key());
    assert_eq!(wait_exit(&rx), PtyExitStatus::Unknown);
    let seen = worker::events(key);
    assert_eq!(seen.last(), Some(&Event::Lost), "{seen:?}");
    drop(backend);
    std::thread::sleep(Duration::from_millis(900));
    assert_eq!(worker::events(key), seen, "used a lost launch");
    // The loss stays on record: host stop is never complete with it.
    let outcome = shutdown_panes(Instant::now() + Duration::from_millis(300));
    assert!(
        matches!(&outcome, ShutdownOutcome::Incomplete { unresolved, .. }
            if unresolved.iter().any(|u| u.contains("pin lost"))),
        "{outcome:?}"
    );
    assert_eq!(worker::events(key), seen, "host stop used a lost launch");
    end_fixture(shell);
}

/// A launch admitted but not yet spawned when host stop starts is never
/// missed: the stop does not wait behind the spawn, returns by its
/// deadline, and is not complete; the launch is then closed by its owner.
#[test]
fn host_stop_during_an_admission_returns_on_time_and_is_never_complete() {
    if !isolated(
        "host_stop_during_an_admission_returns_on_time_and_is_never_complete",
    ) {
        return;
    }
    let held = worker::ADMIT_HOLD.lock().unwrap_or_else(|e| e.into_inner());
    let launching = std::thread::spawn(|| {
        let (tx, rx) = mpsc::channel();
        let backend = TerminalBackend::new(
            1652,
            egui::Context::default(),
            tx,
            settings("/bin/sh", "trap '' HUP; exec sleep 3600"),
        );
        (backend, rx)
    });
    wait_until("the launch to be admitted", || {
        worker::ADMITTING.load(Ordering::SeqCst) == 1
    });
    let started = Instant::now();
    let outcome = shutdown_panes(started + Duration::from_millis(300));
    assert!(
        started.elapsed() < Duration::from_millis(300) + EPSILON,
        "host stop waited {:?} behind a spawn",
        started.elapsed()
    );
    assert!(
        matches!(&outcome, ShutdownOutcome::Incomplete { admitting: 1, .. }),
        "{outcome:?}"
    );
    assert_eq!(outcome.unresolved(), 1);
    drop(held);
    let (backend, _rx) = launching.join().unwrap();
    let backend = backend.expect("a launch admitted before the stop runs");
    let (shell, key) =
        (backend.child_pid() as libc::pid_t, backend.launch_key());
    // Sealed: the owner closes it without waiting for the pane to drop.
    assert_owned_signals_then_one_reap(key, shell);
    drop(backend);
}

/// Host stop before any pane exists seals admission in the same decision:
/// it is complete, and no pane is admitted after it.
#[test]
fn host_stop_before_any_pane_refuses_later_ones() {
    if !isolated("host_stop_before_any_pane_refuses_later_ones") {
        return;
    }
    assert_eq!(
        shutdown_panes(Instant::now() + Duration::from_millis(300)),
        ShutdownOutcome::Complete { owned: Vec::new() }
    );
    let (tx, _rx) = mpsc::channel();
    let refused = TerminalBackend::new(
        1653,
        egui::Context::default(),
        tx,
        settings("/bin/sh", "exec sleep 3600"),
    );
    assert!(refused.is_err(), "a pane was admitted after host stop");
    assert_eq!(SPAWNED.with(|s| s.get()), None, "a shell was spawned");
}

/// R4.3: a wait error right before escalation retires the launch: no KILL,
/// no consuming wait, nothing after.
#[test]
fn a_pin_lost_before_escalation_is_never_used_again() {
    if !isolated("a_pin_lost_before_escalation_is_never_used_again") {
        return;
    }
    worker::fail_next_launch_probe_once(Site::Escalate);
    let (backend, rx) = spawn("trap '' HUP; exec sleep 3600");
    let (shell, key) =
        (backend.child_pid() as libc::pid_t, backend.launch_key());
    drop(backend);
    wait_retired(&rx);
    wait_until("the lost pin", || {
        worker::events(key).contains(&Event::Lost)
    });
    std::thread::sleep(Duration::from_millis(600));
    let events = worker::events(key);
    assert_eq!(
        &events[events.len() - 2..],
        [Event::Probe(Site::Escalate), Event::Lost],
        "{events:?}"
    );
    assert!(
        !events.contains(&Event::Signal(-shell, libc::SIGKILL)),
        "{events:?}"
    );
    end_fixture(shell);
}

/// A pty whose final bytes become readable only once its exit is seen.
struct ExitFirstPty {
    master: UnixStream,
    child_side: UnixStream,
    signals: UnixStream,
}

impl EventedReadWrite for ExitFirstPty {
    type Reader = UnixStream;
    type Writer = UnixStream;

    unsafe fn register(
        &mut self,
        poll: &Arc<polling::Poller>,
        mut interest: polling::Event,
        mode: polling::PollMode,
    ) -> std::io::Result<()> {
        interest.key = PTY_READ_WRITE_TOKEN;
        unsafe {
            poll.add_with_mode(&self.master, interest, mode)?;
            poll.add_with_mode(
                &self.signals,
                polling::Event::readable(PTY_CHILD_EVENT_TOKEN),
                polling::PollMode::Level,
            )
        }
    }

    fn reregister(
        &mut self,
        poll: &Arc<polling::Poller>,
        mut interest: polling::Event,
        mode: polling::PollMode,
    ) -> std::io::Result<()> {
        interest.key = PTY_READ_WRITE_TOKEN;
        poll.modify_with_mode(&self.master, interest, mode)?;
        poll.modify_with_mode(
            &self.signals,
            polling::Event::readable(PTY_CHILD_EVENT_TOKEN),
            polling::PollMode::Level,
        )
    }

    fn deregister(
        &mut self,
        poll: &Arc<polling::Poller>,
    ) -> std::io::Result<()> {
        poll.delete(&self.master)?;
        poll.delete(&self.signals)
    }

    fn reader(&mut self) -> &mut UnixStream {
        &mut self.master
    }

    fn writer(&mut self) -> &mut UnixStream {
        &mut self.master
    }
}

impl EventedPty for ExitFirstPty {
    fn next_child_event(&mut self) -> Option<ChildEvent> {
        let mut byte = [0u8; 1];
        self.signals.read(&mut byte).ok()?;
        self.child_side.write_all(b"final-marker").unwrap();
        Some(ChildEvent::Exited(Some(0)))
    }
}

impl OnResize for ExitFirstPty {
    fn on_resize(&mut self, _: WindowSize) {}
}

/// Unix drains on exit: final bytes that become readable with the exit
/// still reach the screen.
#[test]
fn final_bytes_readable_only_after_the_exit_reach_the_screen() {
    let (master, child_side) = UnixStream::pair().unwrap();
    let (signals, mut exit) = UnixStream::pair().unwrap();
    master.set_nonblocking(true).unwrap();
    signals.set_nonblocking(true).unwrap();
    exit.write_all(&[1]).unwrap();
    let (tx, _rx) = mpsc::channel();
    let size = TerminalSize::default();
    let term = Arc::new(FairMutex::new(Term::new(
        super::terminal_config(),
        &size,
        EventProxy(tx.clone()),
    )));
    let pty = ExitFirstPty {
        master,
        child_side,
        signals,
    };
    EventLoop::new(
        term.clone(),
        EventProxy(tx),
        pty,
        super::DRAIN_ON_EXIT,
        false,
    )
    .unwrap()
    .spawn()
    .join()
    .unwrap();
    let lines =
        TerminalBackend::capture_last_content_lines(term.lock().grid(), 5);
    assert!(lines.join("\n").contains("final-marker"), "{lines:?}");
}
