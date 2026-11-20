//! The Unix pty of one pane, owned here instead of by alacritty's
//! `tty::Pty` (stint 1650). alacritty's `Pty` keeps the child and the master
//! private, reaps the shell in `next_child_event`, and its `Drop` SIGHUPs
//! that pid and waits with the master still open (alacritty_terminal
//! 0.25.1 `src/tty/unix.rs`, `Pty::drop` and `next_child_event`). That
//! deadlocked a closing pane and could signal a reused pid. Here the master
//! closes first, and the shell belongs to the lifecycle worker, which on
//! the pane lifecycle path alone observes, signals and reaps it
//! (`lifecycle`).
//!
//! Spawning mirrors alacritty's `tty::unix::from_fd` line for line: IUTF8,
//! the child's env, `setsid`, controlling tty, closed fds and signal resets.
//! Everything fallible runs before the shell exists: fd setup in
//! [`prepare`], then the reader's start and poll registration, which
//! [`Launch::spawn`] waits for. Only then is the shell spawned and attached.
use std::fs::File;
use std::io::{self, Read};
use std::mem::ManuallyDrop;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use alacritty_terminal::event::{OnResize, WindowSize};
use alacritty_terminal::event_loop::EventLoopSender;
use alacritty_terminal::tty::{
    ChildEvent, EventedPty, EventedReadWrite, ToWinsize,
};
use polling::{Event, PollMode, Poller};

use super::lifecycle::{self, LaunchKey, MastersNotice};
use super::settings::BackendSettings;
use super::tap::TapReader;
use super::{failpoint, PtyExitStatus};

// The event loop's poll keys, `pub(crate)` in alacritty (`src/tty/unix.rs`
// `PTY_READ_WRITE_TOKEN`, `PTY_CHILD_EVENT_TOKEN`); the dependency is pinned.
pub(super) const PTY_READ_WRITE_TOKEN: usize = 0;
pub(super) const PTY_CHILD_EVENT_TOKEN: usize = 1;
/// Longest wait for a started reader to report its poll registration.
const REGISTRATION_WAIT: Duration = Duration::from_secs(5);

/// The master fd for foreground queries, `None` once it is closed. The
/// close takes the write lock, so a query holding the read lock always
/// borrows a live descriptor for its whole ioctl.
pub(super) type Foreground = Arc<RwLock<Option<RawFd>>>;

/// The launch, once admitted, and whether the masters are closed:
/// whichever comes second tells the worker the masters closed.
#[derive(Default)]
struct Slot {
    notice: Option<MastersNotice>,
    masters_closed: bool,
}

pub(super) struct PanePty {
    pane: u64,
    master: ManuallyDrop<File>,
    /// The tap reads through a dup of the master so it can own its reader.
    reader: ManuallyDrop<TapReader<File>>,
    foreground: Foreground,
    /// The worker writes here after publishing the shell's exit status.
    wake: UnixStream,
    slot: Arc<Mutex<Slot>>,
    registered: Option<mpsc::Sender<io::Result<()>>>,
    exit_status: Arc<Mutex<Option<PtyExitStatus>>>,
}

/// What spawning the shell needs, kept until the reader is running.
pub(super) struct Launch {
    pane: u64,
    shell: String,
    command: Command,
    slot: Arc<Mutex<Slot>>,
    registered: Receiver<io::Result<()>>,
    /// The worker's end of the reader's wake socket.
    wake: UnixStream,
    exit_status: Arc<Mutex<Option<PtyExitStatus>>>,
    pub(super) foreground: Foreground,
}

/// Set up the pty and everything fallible around it; the shell does not
/// exist yet. The returned pty goes to the event loop, the launch spawns
/// the shell once that loop's reader is running.
pub(super) fn prepare(
    pane: u64,
    settings: BackendSettings,
    size: WindowSize,
    reply: Box<dyn FnMut(Vec<u8>) + Send>,
    exit_status: Arc<Mutex<Option<PtyExitStatus>>>,
) -> io::Result<(PanePty, Launch)> {
    // The shell's owner exists before the shell does.
    lifecycle::worker()?;
    let pty = rustix_openpty::openpty(None, Some(&size.to_winsize()))?;
    let (master, slave): (OwnedFd, OwnedFd) = (pty.controller, pty.user);
    let (master_fd, slave_fd) = (master.as_raw_fd(), slave.as_raw_fd());
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        use rustix_openpty::rustix::termios;
        if let Ok(mut attrs) = termios::tcgetattr(&master) {
            attrs.input_modes.set(termios::InputModes::IUTF8, true);
            let _ = termios::tcsetattr(
                &master,
                termios::OptionalActions::Now,
                &attrs,
            );
        }
    }
    failpoint("nonblocking")?;
    let flags = unsafe { libc::fcntl(master_fd, libc::F_GETFL, 0) };
    if flags < 0
        || unsafe {
            libc::fcntl(master_fd, libc::F_SETFL, flags | libc::O_NONBLOCK)
        } < 0
    {
        return Err(io::Error::last_os_error());
    }
    let master = File::from(master);
    let tap = master.try_clone()?;
    let (worker_wake, wake) = UnixStream::pair()?;
    wake.set_nonblocking(true)?;
    worker_wake.set_nonblocking(true)?;
    failpoint("wake")?;
    let mut command = Command::new(&settings.shell);
    command.args(&settings.args);
    command.stdin(slave.try_clone()?);
    command.stderr(slave.try_clone()?);
    command.stdout(slave);
    let pane_id = pane.to_string();
    command.env("ALACRITTY_WINDOW_ID", &pane_id);
    command.env("USER", env_or_passwd("USER", |pw| pw.0)?);
    command.env("HOME", env_or_passwd("HOME", |pw| pw.1)?);
    command.env("WINDOWID", &pane_id);
    command.envs(&settings.env);
    command.env_remove("XDG_ACTIVATION_TOKEN");
    command.env_remove("DESKTOP_STARTUP_ID");
    let working_directory = settings.working_directory;
    unsafe {
        command.pre_exec(move || {
            if libc::setsid() == -1 {
                return Err(io::Error::other("Failed to set session id"));
            }
            if let Some(directory) = working_directory.as_ref() {
                let _ = std::env::set_current_dir(directory);
            }
            if libc::ioctl(slave_fd, libc::TIOCSCTTY as _, 0) < 0 {
                return Err(io::Error::last_os_error());
            }
            libc::close(slave_fd);
            libc::close(master_fd);
            for signal in [
                libc::SIGCHLD,
                libc::SIGHUP,
                libc::SIGINT,
                libc::SIGQUIT,
                libc::SIGTERM,
                libc::SIGALRM,
            ] {
                libc::signal(signal, libc::SIG_DFL);
            }
            Ok(())
        });
    }
    let foreground = Arc::new(RwLock::new(Some(master_fd)));
    let slot = Arc::new(Mutex::new(Slot::default()));
    let (registered_tx, registered) = mpsc::channel();
    #[cfg(test)]
    super::close_tests::note_master(&master);
    let pty = PanePty {
        pane,
        master: ManuallyDrop::new(master),
        reader: ManuallyDrop::new(TapReader::new(
            tap,
            settings.byte_tap,
            reply,
        )),
        foreground: foreground.clone(),
        wake,
        slot: slot.clone(),
        registered: Some(registered_tx),
        exit_status: exit_status.clone(),
    };
    let launch = Launch {
        pane,
        shell: settings.shell,
        command,
        slot,
        registered,
        wake: worker_wake,
        exit_status,
        foreground,
    };
    Ok((pty, launch))
}

impl Launch {
    /// Wait for the running reader's poll registration, then spawn the
    /// shell and hand it to the lifecycle worker, which owns it from here.
    /// `reader` lets the worker end the reader at host stop. On an error no
    /// shell exists, and the caller must shut the reader down.
    pub(super) fn spawn(
        mut self,
        reader: EventLoopSender,
    ) -> io::Result<(LaunchKey, u32)> {
        match self.registered.recv_timeout(REGISTRATION_WAIT) {
            Ok(Ok(())) => {},
            Ok(Err(error)) => return Err(error),
            Err(error) => {
                return Err(io::Error::other(format!(
                    "pane {} reader did not report its registration: {error}",
                    self.pane
                )))
            },
        }
        let admission = lifecycle::admit()?;
        #[cfg(any(test, feature = "testing"))]
        lifecycle::testing::admission_pause();
        let child = self.command.spawn().map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("Failed to spawn command '{}': {error}", self.shell),
            )
        })?;
        // The parent's slave copies close with the command.
        drop(self.command);
        let pid = child.id();
        #[cfg(test)]
        super::close_tests::note_spawn(pid);
        let launch = admission.own(
            self.pane,
            child,
            self.exit_status,
            self.wake,
            reader,
        );
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        let notice = launch.notice();
        if slot.masters_closed {
            notice.send();
        }
        slot.notice = Some(notice);
        drop(slot);
        Ok((launch, pid))
    }
}

/// `USER` / `HOME` from the environment, else from the passwd entry, as
/// alacritty's `ShellUser::from_env`.
fn env_or_passwd(
    key: &str,
    field: fn((String, String)) -> String,
) -> io::Result<String> {
    if let Ok(value) = std::env::var(key) {
        return Ok(value);
    }
    let mut buf = [0 as libc::c_char; 1024];
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut found = std::ptr::null_mut();
    let status = unsafe {
        libc::getpwuid_r(
            libc::getuid(),
            &mut entry,
            buf.as_mut_ptr(),
            buf.len(),
            &mut found,
        )
    };
    if status != 0 || found.is_null() {
        return Err(io::Error::other(format!(
            "{key} unset and no passwd entry"
        )));
    }
    let text = |ptr| {
        unsafe { std::ffi::CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned()
    };
    Ok(field((text(entry.pw_name), text(entry.pw_dir))))
}

/// Closes both masters, which hangs up the tty, then tells the worker, which
/// signals nothing before that. Runs on the PTY reader thread, or the
/// constructor's on a failed start; nothing here waits.
impl Drop for PanePty {
    fn drop(&mut self) {
        #[cfg(any(test, feature = "testing"))]
        let _held = lifecycle::testing::READER_HOLD
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        {
            let mut foreground =
                self.foreground.write().unwrap_or_else(|e| e.into_inner());
            unsafe {
                ManuallyDrop::drop(&mut self.reader);
                ManuallyDrop::drop(&mut self.master);
            }
            *foreground = None;
        }
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        slot.masters_closed = true;
        if let Some(notice) = &slot.notice {
            notice.send();
        }
    }
}

impl EventedReadWrite for PanePty {
    type Reader = TapReader<File>;
    type Writer = File;

    /// Reports the outcome to [`Launch::spawn`]. A failure undoes a partial
    /// registration; alacritty's loop then ends and drops this pty.
    unsafe fn register(
        &mut self,
        poll: &Arc<Poller>,
        mut interest: Event,
        mode: PollMode,
    ) -> io::Result<()> {
        interest.key = PTY_READ_WRITE_TOKEN;
        let result =
            unsafe { poll.add_with_mode(&*self.master, interest, mode) }
                .and_then(|()| {
                    #[cfg(test)]
                    let injected =
                        super::close_tests::fail_second_add(self.pane);
                    #[cfg(not(test))]
                    let injected = false;
                    let second = if injected {
                        Err(io::Error::other("injected second-add failure"))
                    } else {
                        unsafe {
                            poll.add_with_mode(
                                &self.wake,
                                Event::readable(PTY_CHILD_EVENT_TOKEN),
                                PollMode::Level,
                            )
                        }
                    };
                    // The one rollback, for a real or an injected failure.
                    second.inspect_err(|_| self.undo_first_add(poll))
                });
        if let Some(registered) = self.registered.take() {
            let _ = registered.send(match &result {
                Ok(()) => Ok(()),
                Err(error) => Err(io::Error::new(
                    error.kind(),
                    format!("pane {} poll registration: {error}", self.pane),
                )),
            });
        }
        result
    }

    fn reregister(
        &mut self,
        poll: &Arc<Poller>,
        mut interest: Event,
        mode: PollMode,
    ) -> io::Result<()> {
        interest.key = PTY_READ_WRITE_TOKEN;
        poll.modify_with_mode(&*self.master, interest, mode)?;
        poll.modify_with_mode(
            &self.wake,
            Event::readable(PTY_CHILD_EVENT_TOKEN),
            PollMode::Level,
        )
    }

    fn deregister(&mut self, poll: &Arc<Poller>) -> io::Result<()> {
        poll.delete(&*self.master)?;
        poll.delete(&self.wake)
    }

    fn reader(&mut self) -> &mut TapReader<File> {
        &mut self.reader
    }

    fn writer(&mut self) -> &mut File {
        &mut self.master
    }
}

impl EventedPty for PanePty {
    /// The worker publishes the shell's exit status, then wakes this
    /// reader. The reader never waits on the shell itself.
    fn next_child_event(&mut self) -> Option<ChildEvent> {
        let mut bytes = [0u8; 16];
        loop {
            match self.wake.read(&mut bytes) {
                Ok(n) if n > 0 => continue,
                Ok(_) => break,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    break
                },
                Err(error) => {
                    log::error!(
                        "egui_term: pane {} wake socket failed: {error}",
                        self.pane
                    );
                    return None;
                },
            }
        }
        let status =
            (*self.exit_status.lock().unwrap_or_else(|e| e.into_inner()))?;
        Some(ChildEvent::Exited(match status {
            PtyExitStatus::Code(code) => Some(code),
            _ => None,
        }))
    }
}

impl PanePty {
    /// The rollback for a registration that failed after its first add.
    fn undo_first_add(&self, poll: &Arc<Poller>) {
        if let Err(error) = poll.delete(&*self.master) {
            log::warn!(
                "egui_term: pane {} undoing its master registration failed: {error}",
                self.pane
            );
        }
        #[cfg(test)]
        super::close_tests::note_undo(self.pane, poll, &self.master);
    }
}

impl OnResize for PanePty {
    fn on_resize(&mut self, size: WindowSize) {
        let size = size.to_winsize();
        let fd = self.master.as_raw_fd();
        if unsafe { libc::ioctl(fd, libc::TIOCSWINSZ, &size as *const _) } < 0 {
            log::error!(
                "egui_term: TIOCSWINSZ on pane {} failed: {}",
                self.pane,
                io::Error::last_os_error()
            );
        }
    }
}
