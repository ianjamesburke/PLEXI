//! The host process's single exit door.
//!
//! eframe 0.34 calls `App::on_exit` before destroying its window, and only
//! ends the *process* on a later
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

/// Schedules the relaunch into the installed build for an update restart.
/// Production uses [`schedule_installed_restart`]; headless hosts use
/// `test_restart_scheduler::record`, which never reads an installed receipt.
pub(crate) type RestartScheduler = fn() -> Result<(), String>;

/// Look up this machine's installed receipt and schedule the relaunch.
pub(crate) fn schedule_installed_restart() -> Result<(), String> {
    let receipt = crate::distribution::installed()?
        .ok_or_else(|| "no managed installation available for restart".to_string())?;
    plexi_distribution::transaction::schedule_restart(&receipt, std::process::id())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
pub(crate) mod test_restart_scheduler {
    //! Headless restart scheduler: counts requests on this thread and touches
    //! no installation.
    use std::cell::Cell;

    thread_local! {
        static REQUESTS: Cell<usize> = const { Cell::new(0) };
    }

    pub(crate) fn record() -> Result<(), String> {
        log::info!("update restart: test scheduler recorded a request");
        REQUESTS.with(|requests| requests.set(requests.get() + 1));
        Ok(())
    }

    /// Requests recorded on this thread since the last call.
    pub(crate) fn take() -> usize {
        REQUESTS.with(|requests| requests.replace(0))
    }
}

#[derive(Default)]
pub(crate) struct QuitCoordinator {
    pub(crate) reason: Option<ExitReason>,
    saved: bool,
    retry_at: Option<Instant>,
    hide_requested: bool,
    restart_scheduled: bool,
    /// Consecutive failed final saves since the last success.
    save_failures: u32,
    /// Set once failures reach [`MAX_QUIT_SAVE_ATTEMPTS`]: the cause shown in
    /// the "Quit without saving" dialog. While set, no retry runs.
    save_failure: Option<String>,
    /// The user chose to quit without saving; the file on disk stays untouched.
    save_waived: bool,
}

/// Failed final saves tolerated (one per second) before asking the user.
pub(crate) const MAX_QUIT_SAVE_ATTEMPTS: u32 = 5;

impl QuitCoordinator {
    pub(crate) fn is_active(&self) -> bool {
        self.reason.is_some()
    }

    pub(crate) fn blocks_input(&self) -> bool {
        self.is_active() && self.retry_at.is_none() && self.save_failure.is_none()
    }

    /// The validation message of the save that exhausted the retry budget.
    pub(crate) fn save_failure(&self) -> Option<&str> {
        self.save_failure.as_deref()
    }
}

/// Process-local monotonic clock. Input reception is the plugin callback boundary,
/// not physical input or compositor presentation. External observers measure
/// those boundaries and actual process death using the logged PID.
pub(crate) fn trace(phase: &str) {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    let origin = ORIGIN.get_or_init(Instant::now);
    log::info!(
        "quit_trace: pid={} elapsed_us={} phase={phase}",
        std::process::id(),
        origin.elapsed().as_micros()
    );
    #[cfg(test)]
    eprintln!(
        "quit_trace: pid={} elapsed_us={} phase={phase}",
        std::process::id(),
        origin.elapsed().as_micros()
    );
}

impl super::PlexiApp {
    /// Acceptance freezes further UI/IPC mutation. The next logic pass takes
    /// the final snapshot, after the accepting UI pass has finished.
    pub(crate) fn request_quit(&mut self, reason: ExitReason) {
        if self.quit_coordinator.reason.is_none() {
            trace("quit_accepted");
            log::info!("quit: coordinator accepted reason={}", reason.label());
            self.quit_coordinator.reason = Some(reason);
            self.quitting = true;
        }
        self.ctx
            .send_viewport_cmd(egui::ViewportCommand::CancelClose);
        self.ui_wake.wake("quit_coordinator");
    }

    pub(crate) fn persist_quit_state(&mut self) -> bool {
        if self.quit_coordinator.saved && !self.workspace_dirty {
            trace("save_reused");
            return true;
        }
        if self.quit_coordinator.save_waived {
            trace("save_waived");
            log::warn!("quit: continuing without saving the workspace at the user's request");
            return true;
        }
        trace("save_begin");
        let result = self.save_workspace_result();
        let saved = result.is_ok();
        trace(if saved { "save_ok" } else { "save_failed" });
        self.workspace_dirty = !saved;
        self.quit_coordinator.saved = saved;
        match result {
            Ok(()) => self.quit_coordinator.save_failures = 0,
            Err(error) => {
                self.quit_coordinator.save_failures += 1;
                if self.quit_coordinator.save_failures >= MAX_QUIT_SAVE_ATTEMPTS {
                    self.quit_coordinator.save_failure = Some(error.to_string());
                }
            }
        }
        saved
    }

    /// Dialog outcome for a quit whose save kept failing. `quit_anyway` skips
    /// the save (the file on disk is left as it is); otherwise the quit is
    /// cancelled and the window stays.
    pub(crate) fn resolve_quit_save_failure(&mut self, quit_anyway: bool) {
        self.pending_close = false;
        if quit_anyway {
            log::warn!(
                "quit: user chose Quit without saving after {} failed saves: {}",
                self.quit_coordinator.save_failures,
                self.quit_coordinator.save_failure().unwrap_or("unknown")
            );
            self.quit_coordinator.save_waived = true;
            self.quit_coordinator.save_failure = None;
            self.quit_coordinator.retry_at = None;
            self.ui_wake.wake("quit_without_saving");
        } else {
            log::info!("quit: user cancelled after save failures; keeping the window");
            self.quit_coordinator = QuitCoordinator::default();
            self.quitting = false;
            self.ctx
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
    }

    /// Returns true while the coordinator owns servicing. Hidden windows do
    /// not run App::ui, so retry, restart and cleanup must all live in logic.
    pub(crate) fn service_quit(&mut self, frame: &eframe::Frame) -> bool {
        self.service_quit_with_restart(frame, self.restart_scheduler)
    }

    pub(crate) fn service_quit_with_restart(
        &mut self,
        frame: &eframe::Frame,
        schedule_restart: impl FnOnce() -> Result<(), String>,
    ) -> bool {
        let Some(reason) = self.quit_coordinator.reason else {
            return false;
        };
        self.ctx
            .send_viewport_cmd(egui::ViewportCommand::CancelClose);
        if self.quit_coordinator.save_failure.is_some() {
            return false; // waiting on the user's answer to the dialog
        }
        if self
            .quit_coordinator
            .retry_at
            .is_some_and(|at| Instant::now() < at)
        {
            return false;
        }
        // A failed save leaves the UI usable. Recheck unsaved-document vetoes
        // before retrying a desktop quit, since the user may have edited again.
        if self.quit_coordinator.retry_at.is_some()
            && reason == ExitReason::WindowClosed
            && !self.prepare_user_quit()
        {
            self.quit_coordinator = QuitCoordinator::default();
            self.quitting = false;
            return false;
        }
        if !self.persist_quit_state() {
            if let Some(cause) = self.quit_coordinator.save_failure() {
                log::error!(
                    "quit: workspace save failed {MAX_QUIT_SAVE_ATTEMPTS} times, \
                     giving up retries: {cause}"
                );
                self.quit_coordinator.retry_at = None;
                self.pending_close = true;
                self.pending_quit = false;
                self.ui_wake.wake("quit_save_failed");
                return false;
            }
            let delay = Duration::from_secs(1);
            self.quit_coordinator.retry_at = Some(Instant::now() + delay);
            self.schedule_workspace_save_wake(delay);
            log::error!("quit: save failed; keeping window and retrying before cleanup");
            return false;
        }
        self.quit_coordinator.retry_at = None;
        if matches!(reason, ExitReason::UpdateRestart) && !self.quit_coordinator.restart_scheduled {
            let result = schedule_restart();
            if let Err(error) = result {
                log::error!("update restart failed: {error}");
                trace("restart_failed");
                self.quit_coordinator = QuitCoordinator::default();
                self.quitting = false;
                return false;
            }
            self.quit_coordinator.restart_scheduled = true;
            trace("restart_scheduled");
        }
        if !self.quit_coordinator.hide_requested {
            trace("window_hide_requested");
            // AppKit confirmation is distinct from egui's occlusion flag:
            // occluded means covered, and cannot prove the window disappeared.
            match hide_native_window(frame) {
                Ok(true) => {
                    trace("window_hidden_native");
                    // Native hide is acknowledged now. Deferred backends only
                    // enter this watchdog mode on the following logic pass.
                    crate::platform::logging::mark_shutdown_cleanup(
                        &self.ui_phase,
                        self.quit_coordinator.is_active(),
                        true,
                    );
                }
                Ok(false) => trace("window_hide_deferred"),
                Err(error) => {
                    log::error!("quit: cannot hide native window: {error}; retrying");
                    self.quit_coordinator.retry_at = Some(Instant::now() + Duration::from_secs(1));
                    self.schedule_workspace_save_wake(Duration::from_secs(1));
                    self.quit_coordinator.saved = false;
                    return false;
                }
            }
            self.ctx
                .send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.quit_coordinator.hide_requested = true;
            // Return control to the window system before any child grace wait.
            self.ui_wake.wake("quit_window_hidden");
            return true;
        }
        trace("post_hide_logic");
        self.begin_quit_cleanup();
        self.flush_ui_scale_persist_now();
        self.bank_final_focus_segment();
        exit_host(reason);
    }

    pub(super) fn begin_quit_cleanup(&self) {
        crate::platform::logging::mark_shutdown_cleanup(
            &self.ui_phase,
            self.quit_coordinator.is_active(),
            self.quit_coordinator.hide_requested,
        );
    }

    pub(crate) fn working_agent_count(&self) -> usize {
        self.windows
            .iter()
            .flat_map(|window| window.panes.values())
            .filter(|pane| {
                pane.agent()
                    .is_some_and(|agent| agent.state == crate::protocol::AgentState::Working)
            })
            .count()
    }

    pub(crate) fn resolve_quit_confirmation(&mut self, confirmed: bool) {
        trace(if confirmed {
            "confirmation_accepted"
        } else {
            "confirmation_cancelled"
        });
        self.pending_close = false;
        self.pending_quit = false;
        self.quit_press_count = 0;
        self.quit_last_press = None;
        self.quitting = confirmed;
        if confirmed && self.prepare_user_quit() {
            self.request_quit(ExitReason::WindowClosed);
            log::info!("quit: working-agent confirmation accepted");
        } else {
            self.quit_coordinator = QuitCoordinator::default();
            self.quitting = false;
            self.ctx
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
            log::info!("quit: working-agent confirmation cancelled or vetoed");
        }
    }

    /// Normal desktop quit shares pane-close protection. Forced IPC stop and
    /// process termination cannot present an interactive unsaved-work prompt.
    pub(crate) fn prepare_user_quit(&mut self) -> bool {
        let veto = self.windows.iter_mut().find_map(|window| {
            window.panes.iter_mut().find_map(|(id, pane)| {
                pane.as_view_mut()
                    .and_then(|app| (!app.runtime.request_close()).then_some(*id))
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
            trace("confirmation_required");
            self.pending_close = true;
            self.quit_press_count = 0;
            self.quit_last_press = None;
            log::info!("quit: confirmation required working_agents={working}");
            return false;
        }
        true
    }
}

/// `true` means AppKit reports the window removed from its visible list.
/// This is not a compositor timestamp. Other platforms use the viewport
/// command and yield to the backend before cleanup without claiming an ack.
fn hide_native_window(frame: &eframe::Frame) -> Result<bool, &'static str> {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSView;
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        #[cfg(test)]
        if frame.window_handle().is_err() {
            return Ok(false); // Headless harness, never installed visibility evidence.
        }
        let _main = objc2::MainThreadMarker::new().ok_or("not on AppKit main thread")?;
        let handle = frame
            .window_handle()
            .map_err(|_| "window handle unavailable")?;
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return Err("not an AppKit window");
        };
        // SAFETY: borrowed eframe NSView remains alive; AppKit main thread above.
        let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
        let window = view.window().ok_or("NSView has no window")?;
        window.orderOut(None);
        if window.isVisible() {
            Err("AppKit window remains visible")
        } else {
            Ok(true)
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = frame;
        Ok(false)
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    trace("cleanup_begin");
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
    if k.remaining == 0 {
        crate::broker::integrity::mark_clean_shutdown(&crate::config::config_dir());
        trace("integrity_written");
    } else {
        log::error!(
            "quit: refusing clean-shutdown stamp; {} owned processes remain",
            k.remaining
        );
    }
    trace("socket_release_begin");
    let socket = release_notify_socket();
    trace("socket_release_end");
    log::info!(
        "quit_phase: process exit — reason={reason_label} notify_socket={socket:?} total_ms={}",
        start.elapsed().as_millis()
    );
    // The log file is the only record of a quit; flush before the fd dies.
    trace("exit_call");
    log::logger().flush();
    // Deliberately *before* eframe's own `painter.destroy()`, which runs after
    // `on_exit` returns and which this call therefore skips: the wgpu device
    // and its worker threads are reclaimed by process teardown, exactly as on
    // the `plexi host stop` path that has always exited from mid-frame.
    std::process::exit(i32::from(
        k.remaining != 0 || matches!(socket, SocketRelease::Failed(_)),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quit_cleanup_replaces_the_stale_workspace_save_heartbeat_phase() {
        let mut h = crate::testing::HostHarness::new();
        h.add_test_pane();
        h.app.request_quit(ExitReason::HostStop);
        assert!(h.app.service_quit(&eframe::Frame::_new_kittest()));
        assert!(h.app.quit_coordinator.hide_requested);
        // Model the next logic pass after the backend has applied hide,
        // stopping before exit_host so the watchdog phase is observable.
        h.app.begin_quit_cleanup();
        assert_eq!(
            crate::platform::logging::current_ui_phase_name(&h.app.ui_phase),
            "shutdown_cleanup"
        );
    }

    #[test]
    fn quit_save_is_not_duplicated_by_a_second_quit_path() {
        let mut h = crate::testing::HostHarness::new();
        h.add_test_pane();
        assert!(h.app.persist_quit_state());
        let path = crate::workspace::workspace_path_in(h.profile_dir());
        let first = std::fs::metadata(&path).unwrap().modified().unwrap();
        // Distinct filesystem timestamps even on coarse filesystems.
        std::thread::sleep(Duration::from_millis(20));
        assert!(h.app.persist_quit_state());
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            first,
            "an unchanged quit must not write another workspace generation"
        );
    }

    #[test]
    fn quit_save_persists_a_later_dirty_mutation() {
        let mut h = crate::testing::HostHarness::new();
        h.add_test_pane();
        assert!(h.app.persist_quit_state());
        h.app.sidebar_visible = !h.app.sidebar_visible;
        h.app.mark_workspace_dirty();
        assert!(h.app.persist_quit_state());
        let saved = crate::workspace::WorkspaceFile::load_from(h.profile_dir()).unwrap();
        assert_eq!(saved.sidebar_visible, h.app.sidebar_visible);
        assert!(!h.app.workspace_dirty);
    }

    #[test]
    fn quit_save_failure_stays_dirty_and_retries() {
        let mut h = crate::testing::HostHarness::new();
        h.add_test_pane();
        let path = crate::workspace::workspace_path_in(h.profile_dir());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"invalid workspace").unwrap();
        assert!(!h.app.persist_quit_state());
        assert!(
            h.app.workspace_dirty,
            "a failed final save must remain pending even if initially clean"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"invalid workspace");
        std::fs::remove_file(&path).unwrap();
        assert!(h.app.persist_quit_state());
        assert!(crate::workspace::WorkspaceFile::load_from(h.profile_dir()).is_some());
        assert!(!h.app.workspace_dirty);
    }

    #[test]
    fn quit_confirmation_does_not_close_after_failed_save() {
        let mut h = crate::testing::HostHarness::new();
        h.add_test_pane();
        let path = crate::workspace::workspace_path_in(h.profile_dir());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"invalid workspace").unwrap();
        let output = h.app.ctx.clone().run_ui(egui::RawInput::default(), |_| {
            h.app.resolve_quit_confirmation(true);
        });
        assert!(
            !output.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .iter()
                .any(|command| matches!(command, egui::ViewportCommand::Close)),
            "failed durability must not allow eframe to destroy the window"
        );
    }

    #[test]
    fn quit_reasons_preserve_window_host_stop_and_update_grace() {
        assert_eq!(ExitReason::WindowClosed.grace(), Duration::from_millis(500));
        assert_eq!(ExitReason::HostStop.grace(), Duration::from_secs(2));
        assert_eq!(ExitReason::UpdateRestart.grace(), Duration::from_secs(2));
    }

    #[test]
    fn quit_coordinator_retries_before_hiding_or_scheduling_restart() {
        for reason in [
            ExitReason::WindowClosed,
            ExitReason::HostStop,
            ExitReason::UpdateRestart,
        ] {
            let mut h = crate::testing::HostHarness::new();
            h.add_test_pane();
            let path = crate::workspace::workspace_path_in(h.profile_dir());
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"invalid workspace").unwrap();
            h.app.request_quit(reason);
            assert!(!h
                .app
                .service_quit_with_restart(&eframe::Frame::_new_kittest(), || {
                    panic!("restart must wait for durability")
                }));
            assert!(h.app.workspace_dirty);
            assert!(!h.app.quit_coordinator.hide_requested);
            assert_eq!(h.app.quit_coordinator.reason, Some(reason));
            std::fs::remove_file(&path).unwrap();
            h.app.quit_coordinator.retry_at = None;
            let scheduled = std::cell::Cell::new(0);
            assert!(h
                .app
                .service_quit_with_restart(&eframe::Frame::_new_kittest(), || {
                    assert!(path.is_file());
                    scheduled.set(scheduled.get() + 1);
                    Ok(())
                }));
            assert!(h.app.quit_coordinator.hide_requested);
            assert!(!h.app.workspace_dirty);
            assert_eq!(
                scheduled.get(),
                usize::from(reason == ExitReason::UpdateRestart)
            );
        }
    }

    /// Drives `request_quit` against an unwritable-as-valid workspace file and
    /// returns the harness after the retry budget is exhausted.
    fn quit_with_unsaveable_workspace() -> (crate::testing::HostHarness, std::path::PathBuf) {
        let mut h = crate::testing::HostHarness::new();
        h.add_test_pane();
        let path = crate::workspace::workspace_path_in(h.profile_dir());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"invalid workspace").unwrap();
        h.app.request_quit(ExitReason::HostStop);
        for attempt in 1..=MAX_QUIT_SAVE_ATTEMPTS {
            h.app.quit_coordinator.retry_at = None;
            assert!(!h
                .app
                .service_quit_with_restart(&eframe::Frame::_new_kittest(), || {
                    panic!("restart must wait for durability")
                }));
            assert_eq!(h.app.quit_coordinator.save_failures, attempt);
            assert_eq!(
                h.app.quit_coordinator.save_failure().is_some(),
                attempt == MAX_QUIT_SAVE_ATTEMPTS,
                "the error is surfaced only once the budget is spent (attempt {attempt})"
            );
        }
        (h, path)
    }

    #[test]
    fn quit_save_retry_is_bounded_and_surfaces_the_cause_once() {
        let (mut h, path) = quit_with_unsaveable_workspace();
        let cause = h.app.quit_coordinator.save_failure().unwrap().to_owned();
        assert!(
            cause.contains("invalid workspace"),
            "cause names the failure: {cause}"
        );
        assert!(h.app.pending_close, "dialog is requested");
        assert!(
            !h.app.quit_coordinator.blocks_input(),
            "the dialog must be usable"
        );
        // No further save attempt while the dialog waits.
        for _ in 0..3 {
            assert!(!h
                .app
                .service_quit_with_restart(&eframe::Frame::_new_kittest(), || Ok(())));
        }
        assert_eq!(h.app.quit_coordinator.save_failures, MAX_QUIT_SAVE_ATTEMPTS);
        assert_eq!(std::fs::read(&path).unwrap(), b"invalid workspace");
    }

    #[test]
    fn quit_without_saving_proceeds_and_preserves_the_file_on_disk() {
        let (mut h, path) = quit_with_unsaveable_workspace();
        h.app.resolve_quit_save_failure(true);
        assert!(!h.app.pending_close);
        assert!(h
            .app
            .service_quit_with_restart(&eframe::Frame::_new_kittest(), || Ok(())));
        assert!(
            h.app.quit_coordinator.hide_requested,
            "quit proceeds to hide and exit"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"invalid workspace");
    }

    #[test]
    fn cancelling_after_save_failures_keeps_the_window() {
        let (mut h, path) = quit_with_unsaveable_workspace();
        h.app.resolve_quit_save_failure(false);
        assert!(!h.app.quit_coordinator.is_active());
        assert!(!h.app.quitting);
        assert!(!h.app.pending_close);
        assert!(!h
            .app
            .service_quit_with_restart(&eframe::Frame::_new_kittest(), || Ok(())));
        assert!(!h.app.quit_coordinator.hide_requested);
        assert_eq!(std::fs::read(&path).unwrap(), b"invalid workspace");
    }

    #[test]
    fn save_failure_dialog_is_keyboard_driven() {
        let (mut h, _path) = quit_with_unsaveable_workspace();
        h.run_frames(2);
        assert!(h
            .app
            .focus_stack
            .contains(&crate::app::focus::FocusKind::ConfirmClose));
        h.press_key(egui::Key::Escape, egui::Modifiers::NONE);
        assert!(
            !h.app.quit_coordinator.is_active(),
            "Esc cancels and keeps the window"
        );
        let (mut h, _path) = quit_with_unsaveable_workspace();
        h.run_frames(2);
        h.press_key(egui::Key::Q, egui::Modifiers::NONE);
        assert!(h.app.quit_coordinator.save_waived, "Q quits without saving");
        assert!(h.app.quit_coordinator.save_failure().is_none());
    }

    /// A real frame with an update restart pending goes through the injected
    /// scheduler: the harness records exactly one request and never reads an
    /// installed receipt or schedules a real restart.
    #[test]
    fn update_restart_frame_records_one_request_through_the_injected_scheduler() {
        test_restart_scheduler::take();
        let mut h = crate::testing::HostHarness::new();
        h.add_test_pane();
        h.app.update_quit_pending = true;
        h.run_frames(1);
        assert_eq!(test_restart_scheduler::take(), 1);
        assert_eq!(
            h.app.quit_coordinator.reason,
            Some(ExitReason::UpdateRestart)
        );
        assert!(h.app.quit_coordinator.restart_scheduled);
        assert!(h.app.quit_coordinator.hide_requested);
        // A second pass after the hide would call `exit_host`; stop here.
    }

    #[test]
    fn failed_update_schedule_keeps_window_and_allows_a_fresh_quit() {
        let mut h = crate::testing::HostHarness::new();
        h.add_test_pane();
        h.app.request_quit(ExitReason::UpdateRestart);
        assert!(!h
            .app
            .service_quit_with_restart(&eframe::Frame::_new_kittest(), || Err(
                "fixture error".into()
            )));
        assert!(!h.app.quit_coordinator.is_active());
        assert!(!h.app.quit_coordinator.hide_requested);
        assert!(!h.app.quitting);
        assert!(crate::workspace::WorkspaceFile::load_from(h.profile_dir()).is_some());
        h.app.request_quit(ExitReason::WindowClosed);
        assert!(h.app.service_quit(&eframe::Frame::_new_kittest()));
        assert!(h.app.quit_coordinator.hide_requested);
    }

    #[test]
    fn duplicate_desktop_close_is_cancelled_and_coordinated_once() {
        let mut h = crate::testing::HostHarness::new();
        h.add_test_pane();
        h.app.config.confirm_quit = Some(false);
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Key {
            key: egui::Key::Q,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        });
        input
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .events
            .push(egui::ViewportEvent::Close);
        h.frame(input);
        assert_eq!(
            h.app.quit_coordinator.reason,
            Some(ExitReason::WindowClosed)
        );
        let output = h.last_full_output.as_ref().unwrap();
        let commands = &output.viewport_output[&egui::ViewportId::ROOT].commands;
        assert!(commands
            .iter()
            .any(|command| matches!(command, egui::ViewportCommand::CancelClose)));
        assert!(!commands
            .iter()
            .any(|command| matches!(command, egui::ViewportCommand::Close)));
        h.hidden_frame(); // saves and hides; cleanup needs a subsequent pass
        assert!(h.app.quit_coordinator.saved);
        assert!(h.app.quit_coordinator.hide_requested);
    }

    #[test]
    fn host_stop_does_not_debounce_save_before_its_final_save() {
        let mut h = crate::testing::HostHarness::new();
        h.add_test_pane();
        h.app.workspace_dirty = true;
        h.app.last_workspace_save = Instant::now() - Duration::from_secs(2);
        h.inject_ipc(crate::protocol::HostRequest::Shutdown);
        h.hidden_frame(); // coordinator saves and hides, but does not exit yet
        let path = crate::workspace::workspace_path_in(h.profile_dir());
        assert!(path.exists());
        assert!(
            !path.with_extension("json.prev").exists(),
            "the debounce must not create an extra generation before final save"
        );
        assert!(h.app.quit_coordinator.hide_requested);
    }

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
