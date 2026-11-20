//! Stint 1650: closing a pane hangs up its terminal (contract in egui_term's
//! `backend::session`).

use std::path::Path;
use std::time::{Duration, Instant};

fn gone(pid: i32) -> bool {
    // ESRCH also means reaped: a zombie still answers `kill(pid, 0)`.
    (unsafe { libc::kill(pid, 0) }) != 0
        && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
}

fn wait_for(what: &str, deadline: Instant, mut done: impl FnMut() -> bool) {
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn read_pid(path: &Path) -> i32 {
    std::fs::read_to_string(path)
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

/// Device number of the master of the pty whose slave is `slave`: macOS
/// numbers a master and its slave with the same minor.
#[cfg(target_os = "macos")]
fn master_of(slave: &Path) -> libc::dev_t {
    use std::os::unix::fs::MetadataExt;
    let slave = std::fs::metadata(slave).unwrap().rdev() as libc::dev_t;
    let probe = unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY) };
    assert!(
        probe >= 0,
        "posix_openpt: {}",
        std::io::Error::last_os_error()
    );
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    assert_eq!(unsafe { libc::fstat(probe, &mut stat) }, 0);
    unsafe { libc::close(probe) };
    libc::makedev(libc::major(stat.st_rdev), libc::minor(slave))
}

/// Open fds of this process on character device `device`.
#[cfg(target_os = "macos")]
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

fn type_into(h: &mut crate::testing::HostHarness, pane: u64, line: String) {
    h.terminal_backend(pane)
        .process_command(egui_term::BackendCommand::Write(line.into_bytes()));
}

/// The audit topology: a job-control shell ignoring SIGHUP, a foreground
/// program that, like Claude Code, cleans up on SIGHUP by writing to the
/// tty while flooding it, a redirected HUP-ignoring background job in its
/// own group, a process that left the session, and another pane. Before,
/// `Pty::drop` hung up the shell pid only and waited with the master open:
/// the exiting shell blocked draining output and the foreground program
/// blocked writing it. Now the master closes first: the foreground program
/// and the shell end, the masters are released, and by contract the
/// background job, the detached process and the other pane survive.
#[test]
#[ignore = "requires-pty"]
fn closing_a_pane_hangs_up_its_terminal() {
    let _shell = crate::host::shell::set_test_shell_override("/bin/sh");
    let mut h = crate::testing::HostHarness::new();
    let terminal = h.add_focused_terminal();
    let neighbour = h.add_focused_terminal();
    let neighbour_shell = h.terminal_backend(neighbour).child_pid() as i32;
    let dir = tempfile::tempdir().unwrap();
    let path = |name: &str| dir.path().join(name);
    type_into(
        &mut h,
        terminal,
        format!(
            "trap '' HUP; set -m; sleep 3600 >/dev/null 2>&1 & echo $! > '{}'; \
             perl -MPOSIX -e 'fork and exit; setsid or die; open F, \">\", $ARGV[0]; \
             print F $$; close F; sleep 3600' '{}'; tty > '{}'; : > '{}'; \
             perl -e '$SIG{{HUP}} = sub {{ print \"\\e[?1049l\" x 20000; exit 0 }}; \
             $| = 1; print \"plexi-close-test\\n\" while 1'\n",
            path("background").display(),
            path("detached").display(),
            path("tty").display(),
            path("ready").display()
        ),
    );
    let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(10));
    wait_for("the pane's jobs to start", deadline, || {
        h.run_frames(1);
        path("ready").exists()
    });
    let shell = h.terminal_backend(terminal).child_pid() as i32;
    let background = read_pid(&path("background"));
    wait_for("the detached process", deadline, || {
        path("detached").exists()
    });
    let detached = read_pid(&path("detached"));
    let mut foreground = shell;
    wait_for("the foreground job", deadline, || {
        h.run_frames(1);
        foreground = h
            .terminal_backend(terminal)
            .foreground_pid()
            .unwrap_or(shell);
        foreground != shell
    });
    assert_ne!(unsafe { libc::getpgid(background) }, shell);
    #[cfg(target_os = "macos")]
    let master = master_of(Path::new(
        std::fs::read_to_string(path("tty")).unwrap().trim(),
    ));
    #[cfg(target_os = "macos")]
    assert_eq!(
        open_fds_on(master),
        2,
        "the pty and its tap hold the master"
    );

    h.app.close_pane_by_id(terminal);
    h.run_frames(1);

    let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(10));
    wait_for("the foreground program to end", deadline, || {
        gone(foreground)
    });
    wait_for("the shell to be reaped", deadline, || gone(shell));
    #[cfg(target_os = "macos")]
    wait_for("the pty master to close", deadline, || {
        open_fds_on(master) == 0
    });
    let survivors = [background, detached].map(|pid| !gone(pid));
    for pid in [background, detached] {
        unsafe { libc::kill(pid, libc::SIGKILL) };
    }
    assert_eq!(survivors, [true, true], "background, detached");
    assert!(!gone(neighbour_shell), "closing one pane ended another");
}

/// A close immediately followed by host stop has one owner: the closing
/// shell's `PaneProcessGroup` record stays until its reap, host stop
/// finishes the close (escalates and reaps) before its sweep, and the
/// shell is signalled only in its own group and reaped once.
#[test]
#[ignore = "requires-pty"]
fn closing_then_stopping_the_host_has_one_owner() {
    let _shell = crate::host::shell::set_test_shell_override("/bin/sh");
    let mut h = crate::testing::HostHarness::new();
    let terminal = h.add_focused_terminal();
    let dir = tempfile::tempdir().unwrap();
    let ready = dir.path().join("ready");
    type_into(
        &mut h,
        terminal,
        format!("trap '' HUP; : > '{}'; exec sleep 3600\n", ready.display()),
    );
    let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(10));
    wait_for("the shell to exec", deadline, || {
        h.run_frames(1);
        ready.exists()
    });
    let shell = h.terminal_backend(terminal).child_pid();
    assert!(crate::host::process_cleanup::pane_root_recorded(shell));

    let held = egui_term::pane_close_testing::HOLD
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    h.app.close_pane_by_id(terminal);
    h.run_frames(1);
    let stop = std::thread::spawn(|| {
        crate::host::process_cleanup::finish_closing_panes(Instant::now() + Duration::from_secs(5))
    });
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        crate::host::process_cleanup::pane_root_recorded(shell),
        "the closing shell's record was dropped before its reap"
    );
    assert!(
        !gone(shell as i32),
        "the shell was reaped while the worker was held"
    );
    drop(held);
    stop.join().unwrap();

    assert!(
        gone(shell as i32),
        "host stop returned before the shell was reaped"
    );
    assert!(!crate::host::process_cleanup::pane_root_recorded(shell));
    let sid = shell as i32;
    let events = egui_term::pane_close_testing::events(sid);
    use egui_term::pane_close_testing::Event;
    assert_eq!(events.last(), Some(&Event::Reaped), "{events:?}");
    assert_eq!(events.iter().filter(|e| **e == Event::Reaped).count(), 1);
    assert!(
        events
            .iter()
            .all(|e| !matches!(e, Event::Signal(target, _) if *target != -sid)),
        "{events:?}"
    );
}
