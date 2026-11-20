//! Stint 1650: closing a pane. The contract is in `session`.
use std::cell::Cell;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::event::{OnResize, WindowSize};
use alacritty_terminal::event_loop::EventLoop;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Term;
use alacritty_terminal::tty::{ChildEvent, EventedPty, EventedReadWrite};

use super::pane_pty::{PTY_CHILD_EVENT_TOKEN, PTY_READ_WRITE_TOKEN};
use super::session::testing::{self as worker, Event};
use super::settings::BackendSettings;
use super::{
    EventProxy, PtyEvent, PtyExitStatus, TerminalBackend, TerminalSize,
};

thread_local! {
    pub(super) static FAIL_AT: Cell<Option<&'static str>> = const { Cell::new(None) };
    /// The last shell spawned on this thread and its master's device.
    static SPAWNED: Cell<Option<(u32, libc::dev_t)>> = const { Cell::new(None) };
}

pub(super) fn note_spawn(pid: u32, master: &std::fs::File) {
    use std::os::unix::fs::MetadataExt;
    let device = master.metadata().unwrap().rdev() as libc::dev_t;
    SPAWNED.with(|spawned| spawned.set(Some((pid, device))));
}

fn spawn(script: &str) -> (TerminalBackend, Receiver<(u64, PtyEvent)>) {
    let (tx, rx) = mpsc::channel();
    let settings = BackendSettings {
        shell: "/bin/sh".to_string(),
        args: vec!["-c".to_string(), script.to_string()],
        ..Default::default()
    };
    let backend =
        TerminalBackend::new(1650, egui::Context::default(), tx, settings)
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

fn open_fds_on(device: libc::dev_t) -> usize {
    (0..unsafe { libc::getdtablesize() })
        .filter(|fd| {
            let mut stat: libc::stat = unsafe { std::mem::zeroed() };
            (unsafe { libc::fstat(*fd, &mut stat) }) == 0
                && stat.st_mode & libc::S_IFMT == libc::S_IFCHR
                && stat.st_rdev == device
        })
        .count()
}

/// The worker signalled only the shell's own group, and reaped the shell
/// exactly once, after its last signal.
fn assert_owned_signals_then_one_reap(sid: libc::pid_t) {
    wait_until("the shell to be reaped", || {
        worker::events(sid).contains(&Event::Reaped)
    });
    let events = worker::events(sid);
    assert_eq!(events.last(), Some(&Event::Reaped), "{events:?}");
    assert_eq!(
        events.iter().filter(|e| **e == Event::Reaped).count(),
        1,
        "{events:?}"
    );
    for event in &events {
        if let Event::Signal(target, _) = event {
            assert_eq!(
                *target, -sid,
                "signal outside the shell's group: {events:?}"
            );
        }
    }
    assert!(
        events.contains(&Event::Signal(-sid, libc::SIGKILL)),
        "{events:?}"
    );
}

/// Runs the calling test in a child process, alone, and returns `false`
/// there it should stop; in the child it returns `true`. For tests that
/// fill the fd table, depend on fd numbering, or finish every close.
fn isolated(name: &str) -> bool {
    const CHILD: &str = "EGUI_TERM_1650_ISOLATED";
    if std::env::var_os(CHILD).is_some() {
        return true;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!("backend::close_tests::{name}"),
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD, "1")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "isolated run failed:\n{text}");
    assert!(
        text.contains("1 passed"),
        "isolated test did not run:\n{text}"
    );
    false
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
    let (pid, master) = SPAWNED.with(|s| s.get()).unwrap();
    rx.recv_timeout(Duration::from_secs(5))
        .expect("the child never produced output");
    assert_eq!(
        open_fds_on(master),
        2,
        "the pty and its tap hold the master"
    );

    drop(backend);
    wait_retired(&rx);
    assert_owned_signals_then_one_reap(pid as libc::pid_t);
    assert!(gone(pid as libc::pid_t));
    assert_eq!(open_fds_on(master), 0, "a master of the closed pty is open");

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
    let shell = backend.child_pid() as libc::pid_t;
    assert_eq!(wait_exit(&rx), PtyExitStatus::Code(3));
    let job = read_pid(&job_file);
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        !gone(shell),
        "shell {shell} was reaped while its pane is open"
    );
    assert!(!gone(job), "job {job} ended before its pane closed");
    assert!(worker::events(shell).is_empty(), "signalled before close");

    drop(backend);
    wait_until("the job left in the shell's group to end", || gone(job));
    assert_owned_signals_then_one_reap(shell);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        worker::events(shell).last(),
        Some(&Event::Reaped),
        "signalled after the reap"
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
    let shell = backend.child_pid() as libc::pid_t;
    let (background, detached) = (read_pid(&background), read_pid(&detached));
    let mut foreground = shell;
    wait_until("the foreground job", || {
        foreground = backend.foreground_pid().unwrap_or(shell);
        foreground != shell
    });
    assert_ne!(unsafe { libc::getpgid(background) }, shell);

    drop(backend);
    wait_retired(&rx);
    assert_owned_signals_then_one_reap(shell);
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

/// A shell not yet waitable when its kill is due stays on the worker's
/// list, without blocking it, until it can be reaped.
#[test]
fn a_shell_not_yet_waitable_stays_owned_until_reaped() {
    let (backend, rx) = spawn("trap '' HUP; exec sleep 3600");
    let shell = backend.child_pid() as libc::pid_t;
    worker::set_pending(shell, 40);
    drop(backend);
    wait_retired(&rx);
    assert_owned_signals_then_one_reap(shell);
    let events = worker::events(shell);
    let pending = events.iter().filter(|e| **e == Event::ReapPending).count();
    assert_eq!(pending, 40, "{events:?}");
    assert!(gone(shell));
}

/// A constructor that fails at any step returns while the worker is held,
/// and leaves no master, child or reader behind once the worker runs.
#[test]
fn failed_starts_return_at_once_and_leave_nothing() {
    for step in ["nonblocking", "sigchld", "poller", "subscription"] {
        SPAWNED.with(|s| s.set(None));
        FAIL_AT.with(|at| at.set(Some(step)));
        let held = worker::HOLD.lock().unwrap_or_else(|e| e.into_inner());
        let started = Instant::now();
        let (tx, _rx) = mpsc::channel();
        let settings = BackendSettings {
            shell: "/bin/sh".to_string(),
            args: vec![
                "-c".to_string(),
                "trap '' HUP; exec sleep 3600".to_string(),
            ],
            ..Default::default()
        };
        let result =
            TerminalBackend::new(1650, egui::Context::default(), tx, settings);
        FAIL_AT.with(|at| at.set(None));
        assert!(result.is_err(), "{step}: the injected failure did not fail");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{step}: the constructor waited {:?}",
            started.elapsed()
        );
        drop(held);
        match SPAWNED.with(|s| s.get()) {
            Some((pid, master)) => {
                assert!(matches!(step, "poller" | "subscription"), "{step}");
                assert_eq!(open_fds_on(master), 0, "{step}: a master is open");
                assert_owned_signals_then_one_reap(pid as libc::pid_t);
            },
            None => {
                assert!(matches!(step, "nonblocking" | "sigchld"), "{step}")
            },
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
    let pid = backend.child_pid() as libc::pid_t;
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
    assert_owned_signals_then_one_reap(pid);
}

/// Host stop finishes closes in progress: a closing shell is reaped before
/// it returns, and one still not waitable at its deadline is frozen, never
/// signalled or reaped again, so the sweep after it only meets pinned pids.
#[test]
fn finishing_closes_reaps_or_freezes_each_shell() {
    if !isolated("finishing_closes_reaps_or_freezes_each_shell") {
        return;
    }
    let (backend, rx) = spawn("trap '' HUP; exec sleep 3600");
    let shell = backend.child_pid() as libc::pid_t;
    drop(backend);
    wait_retired(&rx);
    assert_eq!(
        super::finish_pane_closes(Instant::now() + Duration::from_secs(2)),
        []
    );
    assert_owned_signals_then_one_reap(shell);

    let (backend, rx) = spawn("trap '' HUP; exec sleep 3600");
    let stuck = backend.child_pid() as libc::pid_t;
    worker::set_pending(stuck, u32::MAX);
    drop(backend);
    wait_retired(&rx);
    wait_until("the stuck shell to reach the worker", || {
        !worker::events(stuck).is_empty()
    });
    let left =
        super::finish_pane_closes(Instant::now() + Duration::from_millis(300));
    assert_eq!(left, [stuck as u32]);
    let seen = worker::events(stuck).len();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        worker::events(stuck).len(),
        seen,
        "the worker touched a frozen shell"
    );
    assert!(!worker::events(stuck).contains(&Event::Reaped));
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
