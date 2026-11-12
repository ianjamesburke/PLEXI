//! Host side of the background update check: start a check off the UI thread,
//! drain its events into `UpdateStatus`, and restart into a ready build.
//! State lives in `crate::host::update_status`; this file only wires it.

use super::{ui_mailbox, PlexiApp};
use crate::host::update_status::{CheckMode, UpdateEvent, UpdateStatus};

/// Starts an update check thread. Production uses
/// `cli::updater::spawn_update_check`; headless hosts use a launcher that
/// never touches the network.
pub(crate) type UpdateCheckLauncher =
    fn(std::path::PathBuf, ui_mailbox::UiMailbox<UpdateEvent>, CheckMode) -> std::io::Result<()>;

/// Begin a check in `status` and launch it. Returns the receiver for its
/// events, or `None` when a check is already running or the launch failed
/// (the failure is recorded in `status`).
pub(crate) fn launch_update_check(
    ui_wake: &std::sync::Arc<dyn ui_mailbox::UiWake>,
    launcher: UpdateCheckLauncher,
    cache_dir: std::path::PathBuf,
    mode: CheckMode,
    status: &mut UpdateStatus,
) -> Option<ui_mailbox::MailboxReceiver<UpdateEvent>> {
    if !status.begin_check() {
        log::info!(
            "update check: {} check skipped; a check is already running",
            mode.label()
        );
        return None;
    }
    let (mailbox, rx) =
        ui_mailbox::UiMailbox::channel(std::sync::Arc::clone(ui_wake), "update_check");
    match launcher(cache_dir, mailbox, mode) {
        Ok(()) => Some(rx),
        Err(error) => {
            log::error!(
                "update check: could not start the {} check thread: {error}",
                mode.label()
            );
            status.apply(UpdateEvent::Failed {
                reason: format!("could not start the check: {error}"),
            });
            None
        }
    }
}

impl PlexiApp {
    /// Start a check unless one is already running. A manual check skips the
    /// update cache.
    pub(crate) fn start_update_check(&mut self, mode: CheckMode) {
        if let Some(rx) = launch_update_check(
            &self.ui_wake,
            self.update_check_launcher,
            crate::config::config_dir(),
            mode,
            &mut self.update_status,
        ) {
            self.update_rx = Some(rx);
        }
    }

    /// Apply every pending event from the running check. Runs from
    /// `update_preamble` (`App::logic`), so a hidden window still settles.
    pub(crate) fn drain_update_events(&mut self) {
        let Some(rx) = &self.update_rx else {
            return;
        };
        loop {
            match rx.try_recv() {
                Ok(event) => {
                    log::info!("update check: status event {event:?}");
                    self.update_status.apply(event);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    if self.update_status.in_flight() {
                        log::warn!("update check: thread ended without a result");
                    }
                    self.update_status.ended_without_result();
                    self.update_rx = None;
                    return;
                }
            }
        }
    }

    /// Restart into the ready build through the same quit path as the
    /// changelog's Restart button. No-op when nothing is ready.
    pub(crate) fn restart_to_update(&mut self, source: &str) {
        match self.update_status.ready() {
            Some(version) => {
                log::info!("update: restart to v{version} requested from {source}");
                self.update_quit_pending = true;
            }
            None => log::warn!("update: restart requested from {source} with no build ready"),
        }
    }
}

#[cfg(test)]
pub(crate) mod test_launcher {
    //! Headless launcher: records each launched check's mailbox on this thread
    //! so a test can report events through the real drain path.
    use super::*;
    use std::cell::RefCell;

    thread_local! {
        static LAUNCHED: RefCell<Vec<(CheckMode, ui_mailbox::UiMailbox<UpdateEvent>)>> =
            const { RefCell::new(Vec::new()) };
    }

    pub(crate) fn record(
        _cache_dir: std::path::PathBuf,
        mailbox: ui_mailbox::UiMailbox<UpdateEvent>,
        mode: CheckMode,
    ) -> std::io::Result<()> {
        LAUNCHED.with(|launched| launched.borrow_mut().push((mode, mailbox)));
        Ok(())
    }

    /// Every check launched on this thread since the last call.
    pub(crate) fn take() -> Vec<(CheckMode, ui_mailbox::UiMailbox<UpdateEvent>)> {
        LAUNCHED.with(|launched| std::mem::take(&mut *launched.borrow_mut()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::update_status::UpdateLine;

    #[test]
    fn update_check_manual_check_reports_through_the_mailbox() {
        let mut h = crate::testing::HostHarness::new();
        test_launcher::take();
        h.app.start_update_check(CheckMode::Manual);
        assert_eq!(h.app.update_status.line(), UpdateLine::Checking);
        let mut launched = test_launcher::take();
        assert_eq!(launched.len(), 1);
        let (mode, mailbox) = launched.remove(0);
        assert_eq!(mode, CheckMode::Manual);

        // A second check while one runs launches nothing.
        h.app.start_update_check(CheckMode::Manual);
        assert!(test_launcher::take().is_empty());

        mailbox
            .send(UpdateEvent::Downloading {
                version: "0.5.1".into(),
            })
            .unwrap();
        h.app.drain_update_events();
        assert_eq!(
            h.app.update_status.line(),
            UpdateLine::Downloading {
                version: "0.5.1".into()
            }
        );
        mailbox
            .send(UpdateEvent::Ready {
                version: "0.5.1".into(),
            })
            .unwrap();
        drop(mailbox);
        h.app.drain_update_events();
        assert_eq!(h.app.update_status.ready(), Some("0.5.1"));
        assert!(
            h.app.update_rx.is_none(),
            "a finished check releases its receiver"
        );

        h.app.restart_to_update("test");
        assert!(h.app.update_quit_pending);
    }

    #[test]
    fn update_check_thread_exit_without_result_fails_the_check() {
        let mut h = crate::testing::HostHarness::new();
        test_launcher::take();
        h.app.start_update_check(CheckMode::Scheduled);
        drop(test_launcher::take());
        h.app.drain_update_events();
        assert!(matches!(
            h.app.update_status.line(),
            UpdateLine::Failed { .. }
        ));
        assert!(!h.app.update_status.in_flight());
    }

    #[test]
    fn update_check_restart_without_ready_build_does_nothing() {
        let mut h = crate::testing::HostHarness::new();
        h.app.restart_to_update("test");
        assert!(!h.app.update_quit_pending);
    }

    #[test]
    fn update_check_launch_failure_is_reported() {
        fn failing(
            _: std::path::PathBuf,
            _: ui_mailbox::UiMailbox<UpdateEvent>,
            _: CheckMode,
        ) -> std::io::Result<()> {
            Err(std::io::Error::other("no threads"))
        }
        let h = crate::testing::HostHarness::new();
        let mut status = UpdateStatus::default();
        let rx = launch_update_check(
            &h.app.ui_wake,
            failing,
            std::env::temp_dir(),
            CheckMode::Manual,
            &mut status,
        );
        assert!(rx.is_none());
        assert_eq!(
            status.line(),
            UpdateLine::Failed {
                reason: "could not start the check: no threads".into()
            }
        );
    }
}
