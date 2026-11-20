//! The Unix pty of one pane, owned here instead of by alacritty's
//! `tty::Pty` (stint 1650). alacritty's `Pty` keeps the child and the master
//! private, reaps the shell in `next_child_event`, and its `Drop` SIGHUPs
//! that pid and waits with the master still open (alacritty_terminal
//! 0.25.1 `src/tty/unix.rs`, `Pty::drop` and `next_child_event`). That
//! deadlocked a closing pane and could signal a reused pid. Here the master
//! closes first, and the shell stays waitable, pinning its pid and group,
//! until the lifecycle worker reaps it (`session`).
//!
//! Spawning mirrors alacritty's `tty::unix::from_fd` line for line: IUTF8,
//! the child's env, `setsid`, controlling tty, closed fds and signal resets.
//! Every fallible fd step runs before the child exists.
use std::fs::File;
use std::io::{self, Read};
use std::mem::ManuallyDrop;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::process::Command;
use std::sync::{Arc, Mutex, RwLock};

use alacritty_terminal::event::{OnResize, WindowSize};
use alacritty_terminal::tty::{
    ChildEvent, EventedPty, EventedReadWrite, ToWinsize,
};
use polling::{Event, PollMode, Poller};
use signal_hook::low_level::{pipe as signal_pipe, unregister};
use signal_hook::SigId;

use super::session::{self, Session};
use super::settings::BackendSettings;
use super::tap::TapReader;
use super::{failpoint, PtyExitStatus};

// The event loop's poll keys, `pub(crate)` in alacritty (`src/tty/unix.rs`
// `PTY_READ_WRITE_TOKEN`, `PTY_CHILD_EVENT_TOKEN`); the dependency is pinned.
pub(super) const PTY_READ_WRITE_TOKEN: usize = 0;
pub(super) const PTY_CHILD_EVENT_TOKEN: usize = 1;

/// The master fd for foreground queries, `None` once it is closed. The
/// close takes the write lock, so a query holding the read lock always
/// borrows a live descriptor for its whole ioctl.
pub(super) type Foreground = Arc<RwLock<Option<RawFd>>>;

pub(super) struct PanePty {
    master: ManuallyDrop<File>,
    /// The tap reads through a dup of the master so it can own its reader.
    reader: ManuallyDrop<TapReader<File>>,
    foreground: Foreground,
    signals: UnixStream,
    _sigchld: SigChld,
    session: Arc<Session>,
    exit_status: Arc<Mutex<Option<PtyExitStatus>>>,
}

/// The SIGCHLD pipe registration, removed on drop.
struct SigChld(SigId);

impl Drop for SigChld {
    fn drop(&mut self) {
        unregister(self.0);
    }
}

pub(super) fn spawn(
    pane: u64,
    settings: BackendSettings,
    size: WindowSize,
    reply: Box<dyn FnMut(Vec<u8>) + Send>,
    exit_status: Arc<Mutex<Option<PtyExitStatus>>>,
) -> io::Result<(PanePty, Arc<Session>, Foreground)> {
    // The shell's owner exists before the shell does.
    let worker = session::worker()?;
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
    let (sender, signals) = UnixStream::pair()?;
    signals.set_nonblocking(true)?;
    let sigchld = SigChld(signal_pipe::register(libc::SIGCHLD, sender)?);
    failpoint("sigchld")?;
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
    let child = command.spawn().map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("Failed to spawn command '{}': {error}", settings.shell),
        )
    })?;
    // The parent's slave copies close with the command.
    drop(command);
    let session =
        Arc::new(Session::new(pane, child.id() as libc::pid_t, worker));
    #[cfg(test)]
    super::close_tests::note_spawn(session.pid(), &master);
    let foreground = Arc::new(RwLock::new(Some(master_fd)));
    let pty = PanePty {
        master: ManuallyDrop::new(master),
        reader: ManuallyDrop::new(TapReader::new(
            tap,
            settings.byte_tap,
            reply,
        )),
        foreground: foreground.clone(),
        signals,
        _sigchld: sigchld,
        session: session.clone(),
        exit_status,
    };
    Ok((pty, session, foreground))
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

/// Closes both masters, which hangs up the tty, before anything waits on
/// the shell. Then lets go of the session: if this is its last handle
/// (a close of a running pane), the shell goes to the lifecycle worker.
/// Runs on the PTY reader thread, or the constructor's on a failed start;
/// nothing here waits.
impl Drop for PanePty {
    fn drop(&mut self) {
        let mut foreground =
            self.foreground.write().unwrap_or_else(|e| e.into_inner());
        unsafe {
            ManuallyDrop::drop(&mut self.reader);
            ManuallyDrop::drop(&mut self.master);
        }
        *foreground = None;
    }
}

impl EventedReadWrite for PanePty {
    type Reader = TapReader<File>;
    type Writer = File;

    unsafe fn register(
        &mut self,
        poll: &Arc<Poller>,
        mut interest: Event,
        mode: PollMode,
    ) -> io::Result<()> {
        interest.key = PTY_READ_WRITE_TOKEN;
        unsafe {
            poll.add_with_mode(&*self.master, interest, mode)?;
            poll.add_with_mode(
                &self.signals,
                Event::readable(PTY_CHILD_EVENT_TOKEN),
                PollMode::Level,
            )
        }
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
            &self.signals,
            Event::readable(PTY_CHILD_EVENT_TOKEN),
            PollMode::Level,
        )
    }

    fn deregister(&mut self, poll: &Arc<Poller>) -> io::Result<()> {
        poll.delete(&*self.master)?;
        poll.delete(&self.signals)
    }

    fn reader(&mut self) -> &mut TapReader<File> {
        &mut self.reader
    }

    fn writer(&mut self) -> &mut File {
        &mut self.master
    }
}

impl EventedPty for PanePty {
    /// Like alacritty's, but observes the exit with `WNOWAIT` instead of
    /// reaping, so the pid stays pinned until the session ends.
    fn next_child_event(&mut self) -> Option<ChildEvent> {
        let mut byte = [0u8; 1];
        if let Err(error) = self.signals.read(&mut byte) {
            if error.kind() != io::ErrorKind::WouldBlock {
                log::error!(
                    "egui_term: reading the SIGCHLD pipe failed: {error}"
                );
            }
            return None;
        }
        let status = self.session.exit_status()?;
        *self.exit_status.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(status);
        Some(ChildEvent::Exited(match status {
            PtyExitStatus::Code(code) => Some(code),
            _ => None,
        }))
    }
}

impl OnResize for PanePty {
    fn on_resize(&mut self, size: WindowSize) {
        let size = size.to_winsize();
        let fd = self.master.as_raw_fd();
        if unsafe { libc::ioctl(fd, libc::TIOCSWINSZ, &size as *const _) } < 0 {
            log::error!(
                "egui_term: TIOCSWINSZ on pane {} failed: {}",
                self.session.pane(),
                io::Error::last_os_error()
            );
        }
    }
}
