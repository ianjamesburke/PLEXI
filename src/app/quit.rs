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
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|m| (m.dev(), m.ino()))
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

/// Tear the host down and end the process. Never returns.
pub(crate) fn exit_host(reason: &str) -> ! {
    let socket = release_notify_socket();
    log::info!("quit_phase: process exit — reason={reason} notify_socket={socket:?}");
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
