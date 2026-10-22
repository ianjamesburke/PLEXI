//! The host process's single exit door.
//!
//! eframe 0.34 destroys the window on `EventResult::CloseRequested` — which is
//! where `App::on_exit` runs — but only ends the *process* on a later
//! `EventResult::Exit`, and the only thing that can produce one is another
//! winit window event arriving after the window is already gone. A window
//! closed from its chrome (`WM_DELETE_WINDOW`) need not deliver one: on Linux
//! the event loop then parks in `epoll_wait` forever with no window, still
//! holding `notify.sock`, the PTY children and the wgpu worker threads. The
//! result is a headless host that `plexi host status` reports as `ready:
//! false` against a live pid (observed 2026-09-20 on X11/llvmpipe, and the
//! same reason `plexi host stop` has always forced its own exit).
//!
//! Ending the process is therefore ours to do, not eframe's: every quit path
//! finishes in [`exit_host`], and it is the only place in the host that calls
//! `std::process::exit`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

impl super::PlexiApp {
    pub(crate) fn working_agent_count(&self) -> usize {
        self.windows.iter().flat_map(|window| window.panes.values())
            .filter(|pane| pane.agent().is_some_and(|agent| agent.state == crate::protocol::AgentState::Working))
            .count()
    }

    pub(crate) fn resolve_quit_confirmation(&mut self, confirmed: bool) {
        self.pending_close = false;
        self.pending_quit = false;
        self.quit_press_count = 0;
        self.quit_last_press = None;
        self.quitting = confirmed;
        if confirmed && self.prepare_user_quit() {
            self.save_workspace_now();
            self.workspace_dirty = false;
            self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            log::info!("quit: working-agent confirmation accepted");
        } else {
            self.quitting = false;
            self.ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            log::info!("quit: working-agent confirmation cancelled or vetoed");
        }
    }

    /// Normal desktop quit shares pane-close protection. Forced IPC stop and
    /// process termination cannot present an interactive unsaved-work prompt.
    pub(crate) fn prepare_user_quit(&mut self) -> bool {
        let veto = self.windows.iter_mut().find_map(|window| {
            window.panes.iter_mut().find_map(|(id, pane)| {
                pane.as_app_mut().and_then(|app| (!app.runtime.request_close()).then_some(*id))
            })
        });
        if let Some(pane) = veto {
            self.quitting = false;
            self.quit_press_count = 0;
            self.quit_last_press = None;
            self.pane_navigate(pane);
            log::info!("quit: unsaved work vetoed desktop quit pane={pane}");
            return false;
        }
        let working = self.working_agent_count();
        if !self.quitting && working > 0 {
            self.pending_quit = true;
            self.pending_close = true;
            self.quit_press_count = 0;
            self.quit_last_press = None;
            log::info!("quit: confirmation required working_agents={working}");
            return false;
        }
        true
    }
}

/// The notify socket this process bound, with the identity it had at bind
/// time. Written once by the socket listener, read only by teardown.
static BOUND_NOTIFY_SOCKET: OnceLock<BoundSocket> = OnceLock::new();

#[derive(Debug, Clone)]
struct BoundSocket {
    path: PathBuf,
    /// `(dev, ino)` when the socket could be stat'd at bind time.
    identity: Option<(u64, u64)>,
}

/// What teardown did with the notify socket. Named cases rather than a bool so
/// the log line says which one happened and the tests can tell them apart.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SocketRelease {
    /// This process never bound one (CLI invocations, tests).
    NotBound,
    Removed,
    /// Already gone — a second quit path, or someone cleaned up for us.
    Missing,
    /// A *different* socket sits at the path now: another host owns it, so
    /// removing the file would take that host's IPC down with us.
    Reused,
    Failed(String),
}

fn identity(path: &Path) -> Option<(u64, u64)> {
    // Unix notify sockets are filesystem inodes; (dev, ino) detects reuse.
    // Windows uses named pipes with no filesystem entry — identity is unused.
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(path).ok().map(|m| (m.dev(), m.ino()))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

/// Remember the socket [`exit_host`] must remove. Called once, right after the
/// listener binds.
pub(crate) fn record_notify_socket(path: &Path) {
    let bound = BoundSocket {
        path: path.to_path_buf(),
        identity: identity(path),
    };
    if BOUND_NOTIFY_SOCKET.set(bound).is_err() {
        log::warn!(
            "quit: notify socket already recorded — keeping the first binding ({:?})",
            BOUND_NOTIFY_SOCKET.get().map(|b| &b.path)
        );
    }
}

fn release(bound: &BoundSocket) -> SocketRelease {
    match (identity(&bound.path), bound.identity) {
        (None, _) => SocketRelease::Missing,
        (Some(now), Some(at_bind)) if now != at_bind => SocketRelease::Reused,
        _ => match std::fs::remove_file(&bound.path) {
            Ok(()) => SocketRelease::Removed,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => SocketRelease::Missing,
            Err(e) => SocketRelease::Failed(format!("{}: {e}", bound.path.display())),
        },
    }
}

/// Unbind pane IPC by removing the socket file this process bound, so no CLI
/// can connect to a host that is on its way out.
pub(crate) fn release_notify_socket() -> SocketRelease {
    match BOUND_NOTIFY_SOCKET.get() {
        None => SocketRelease::NotBound,
        Some(bound) => release(bound),
    }
}

/// Why the host is exiting. Each path owns its pane-process grace.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ExitReason {
    WindowClosed,
    HostStop,
    UpdateRestart,
}

impl ExitReason {
    fn label(self) -> &'static str {
        match self {
            Self::WindowClosed => "window closed",
            Self::HostStop => "host stop",
            Self::UpdateRestart => "update restart",
        }
    }

    /// The user is watching a window close, so keep it short. `host stop` and
    /// update restarts keep 2s so agent processes can flush.
    fn grace(self) -> Duration {
        match self {
            Self::WindowClosed => Duration::from_millis(500),
            Self::HostStop | Self::UpdateRestart => Duration::from_secs(2),
        }
    }
}

/// Tear the host down and end the process. Never returns.
pub(crate) fn exit_host(reason: ExitReason) -> ! {
    let start = Instant::now();
    let reason_label = reason.label();
    let k = crate::host::process_cleanup::stop_pane_processes(reason.grace());
    log::info!(
        "quit: killed {} pane process groups ({} SIGKILL) reason={reason_label} grace_ms={} tmux_ms={} scan_ms={} hup_term_ms={} grace_waited_ms={} kill_ms={} total_ms={}",
        k.groups,
        k.escalated,
        k.grace.as_millis(),
        k.tmux.as_millis(),
        k.scan.as_millis(),
        k.signal.as_millis(),
        k.grace_waited.as_millis(),
        k.kill.as_millis(),
        start.elapsed().as_millis()
    );
    // A kill skips this, so the next start can tell a clean quit from a death.
    crate::broker::integrity::mark_clean_shutdown(&crate::config::config_dir());
    let socket = release_notify_socket();
    log::info!(
        "quit_phase: process exit — reason={reason_label} notify_socket={socket:?} total_ms={}",
        start.elapsed().as_millis()
    );
    // The log file is the only record of a quit; flush before the fd dies.
    log::logger().flush();
    // Deliberately *before* eframe's own `painter.destroy()`, which runs after
    // `on_exit` returns and which this call therefore skips: the wgpu device
    // and its worker threads are reclaimed by process teardown, exactly as on
    // the `plexi host stop` path that has always exited from mid-frame.
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bind_fake_socket(dir: &Path, name: &str) -> BoundSocket {
        let path = dir.join(name);
        std::fs::write(&path, b"").expect("create stand-in socket");
        BoundSocket {
            path: path.clone(),
            identity: identity(&path),
        }
    }

    #[test]
    fn release_removes_the_socket_this_process_bound() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bound = bind_fake_socket(dir.path(), "notify.sock");

        assert_eq!(release(&bound), SocketRelease::Removed);
        assert!(
            !bound.path.exists(),
            "quitting must leave no socket for a CLI to connect to"
        );
    }

    #[test]
    fn release_leaves_a_socket_another_host_has_rebound() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bound = bind_fake_socket(dir.path(), "notify.sock");
        // A successor host binds the same path while we are quitting. Renamed
        // into place rather than recreated: ext4 hands the just-freed inode
        // straight back, which would make the successor indistinguishable
        // from ours.
        let successor = dir.path().join("successor.sock");
        std::fs::write(&successor, b"").expect("successor binds");
        std::fs::rename(&successor, &bound.path).expect("successor takes the path");

        assert_eq!(release(&bound), SocketRelease::Reused);
        assert!(
            bound.path.exists(),
            "teardown must not unbind a socket a different host now owns"
        );
    }

    #[test]
    fn release_is_quiet_when_the_socket_is_already_gone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bound = bind_fake_socket(dir.path(), "notify.sock");
        std::fs::remove_file(&bound.path).expect("remove");

        assert_eq!(release(&bound), SocketRelease::Missing);
    }

    /// The only test that touches the process-wide binding, because the
    /// `OnceLock` it sets is visible to every other test in this binary.
    #[test]
    fn a_recorded_socket_is_the_one_teardown_releases() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("notify.sock");
        std::fs::write(&path, b"").expect("create stand-in socket");

        record_notify_socket(&path);

        assert_eq!(release_notify_socket(), SocketRelease::Removed);
        assert!(!path.exists());
        // Idempotent: a second quit path finds nothing left to do.
        assert_eq!(release_notify_socket(), SocketRelease::Missing);
    }
}
