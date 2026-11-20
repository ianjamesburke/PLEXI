//! The Unix pty of one pane, owned here instead of by alacritty's
//! `tty::Pty` (stint 1650). alacritty's `Pty` keeps the child and the master
//! private, reaps the shell in `next_child_event`, and its `Drop` SIGHUPs
//! that pid and waits with the master still open (alacritty_terminal
//! 0.25.1 `src/tty/unix.rs`, `Pty::drop` and `next_child_event`). That
//! deadlocked a closing pane and could signal a reused pid. Here the master
//! closes first, and the shell stays an unreaped zombie, pinning its pid,
//! session and group numbers, until its [`Session`] ends and reaps it.
//!
//! Spawning mirrors alacritty's `tty::unix::from_fd` line for line: IUTF8,
//! the child's env, `setsid`, controlling tty, closed fds and signal resets.
use std::fs::File;
use std::io::{self, Read};
use std::mem::ManuallyDrop;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::process::Command;
use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{OnResize, WindowSize};
use alacritty_terminal::tty::{
    ChildEvent, EventedPty, EventedReadWrite, ToWinsize,
};
use polling::{Event, PollMode, Poller};
use signal_hook::low_level::{pipe as signal_pipe, unregister};
use signal_hook::SigId;

use super::session::Session;
use super::settings::BackendSettings;
use super::tap::TapReader;
use super::PtyExitStatus;

// The event loop's poll keys, `pub(crate)` in alacritty (`src/tty/unix.rs`
// `PTY_READ_WRITE_TOKEN`, `PTY_CHILD_EVENT_TOKEN`); the dependency is pinned.
const PTY_READ_WRITE_TOKEN: usize = 0;
const PTY_CHILD_EVENT_TOKEN: usize = 1;

pub(super) struct PanePty {
    master: ManuallyDrop<File>,
    /// The tap reads through a dup of the master so it can own its reader.
    reader: ManuallyDrop<TapReader<File>>,
    signals: UnixStream,
    sig_id: SigId,
    session: Arc<Session>,
    exit_status: Arc<Mutex<Option<PtyExitStatus>>>,
}

pub(super) fn spawn(
    pane: u64,
    settings: BackendSettings,
    size: WindowSize,
    reply: Box<dyn FnMut(Vec<u8>) + Send>,
    exit_status: Arc<Mutex<Option<PtyExitStatus>>>,
) -> io::Result<(PanePty, Arc<Session>)> {
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
    let (sender, signals) = UnixStream::pair()?;
    let sig_id = signal_pipe::register(libc::SIGCHLD, sender)?;
    signals.set_nonblocking(true)?;
    let child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            unregister(sig_id);
            return Err(io::Error::new(
                error.kind(),
                format!(
                    "Failed to spawn command '{}': {error}",
                    settings.shell
                ),
            ));
        },
    };
    // The parent's slave copies close with the command.
    drop(command);
    let session = Arc::new(Session::new(pane, child.id() as libc::pid_t));
    let master = File::from(master);
    let flags = unsafe { libc::fcntl(master_fd, libc::F_GETFL, 0) };
    if flags < 0
        || unsafe {
            libc::fcntl(master_fd, libc::F_SETFL, flags | libc::O_NONBLOCK)
        } < 0
    {
        unregister(sig_id);
        return Err(io::Error::last_os_error());
    }
    let tap = match master.try_clone() {
        Ok(tap) => tap,
        Err(error) => {
            unregister(sig_id);
            return Err(error);
        },
    };
    let pty = PanePty {
        master: ManuallyDrop::new(master),
        reader: ManuallyDrop::new(TapReader::new(
            tap,
            settings.byte_tap,
            reply,
        )),
        signals,
        sig_id,
        session: session.clone(),
        exit_status,
    };
    Ok((pty, session))
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

impl PanePty {
    /// For `tcgetpgrp` foreground queries by the backend.
    pub(super) fn master_fd(&self) -> std::os::fd::RawFd {
        self.master.as_raw_fd()
    }
}

/// Closes the master before anything waits on the shell, then lets go of
/// the session: on a close this is its last handle and it ends here, on the
/// PTY reader thread. After a natural exit the backend still holds it.
impl Drop for PanePty {
    fn drop(&mut self) {
        unsafe {
            ManuallyDrop::drop(&mut self.reader);
            ManuallyDrop::drop(&mut self.master);
        }
        unregister(self.sig_id);
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
