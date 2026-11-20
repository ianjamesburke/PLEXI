//! Stint 1650: closing a pane ends its whole process tree and releases the pty.

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

/// The shell ignores SIGHUP, a background job (its own process group under
/// job control) ignores it too, and a foreground job floods the tty. That is
/// the measured Beta leak: alacritty's `Pty::drop` hung up the shell pid only
/// and waited with the master open, so the exiting shell blocked draining its
/// output and every group survived the close.
#[test]
#[ignore = "requires-pty"]
fn closing_a_pane_ends_its_whole_process_tree() {
    let _shell = crate::host::shell::set_test_shell_override("/bin/sh");
    let mut h = crate::testing::HostHarness::new();
    let terminal = h.add_focused_terminal();
    let dir = tempfile::tempdir().unwrap();
    let (grandchild, tty, ready) = (
        dir.path().join("grandchild"),
        dir.path().join("tty"),
        dir.path().join("ready"),
    );
    h.terminal_backend(terminal)
        .process_command(egui_term::BackendCommand::Write(
            format!(
                "trap '' HUP; set -m; sleep 3600 & echo $! > '{}'; tty > '{}'; : > '{}'; \
                 yes plexi-close-test\n",
                grandchild.display(),
                tty.display(),
                ready.display()
            )
            .into_bytes(),
        ));
    let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(10));
    wait_for("the pane's jobs to start", deadline, || {
        h.run_frames(1);
        ready.exists()
    });
    let shell = h.terminal_backend(terminal).child_pid() as i32;
    let grandchild = read_pid(&grandchild);
    let mut foreground = shell;
    wait_for("the foreground job", deadline, || {
        h.run_frames(1);
        foreground = h
            .terminal_backend(terminal)
            .foreground_pid()
            .unwrap_or(shell);
        foreground != shell
    });
    let background = unsafe { libc::getpgid(grandchild) };
    assert!(
        background != shell && background != foreground,
        "the background job must be its own group: shell={shell} background={background} foreground={foreground}"
    );
    #[cfg(target_os = "macos")]
    let master = master_of(Path::new(std::fs::read_to_string(&tty).unwrap().trim()));
    #[cfg(target_os = "macos")]
    assert_eq!(
        open_fds_on(master),
        2,
        "the pty reader and its tap hold the master"
    );

    h.app.close_pane_by_id(terminal);
    h.run_frames(1);

    let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(10));
    for (what, pid) in [
        ("shell", shell),
        ("background job", grandchild),
        ("foreground job", foreground),
    ] {
        wait_for(&format!("the {what} {pid} to end"), deadline, || gone(pid));
    }
    for group in [shell, background, foreground] {
        wait_for(&format!("process group {group} to empty"), deadline, || {
            gone(-group)
        });
    }
    // The shell is gone, not a zombie: it was reaped, and its only reaper is
    // the pane's PTY reader thread, so that thread is past its last wait.
    #[cfg(target_os = "macos")]
    wait_for("the pty master to close", deadline, || {
        open_fds_on(master) == 0
    });
}
