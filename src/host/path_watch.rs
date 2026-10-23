//! Debounced external-change watch on one file or one directory, for apps that
//! show live disk state (text editor, file browser).
//!
//! Same shape as `crate::host::state_watch`: the watch target is always a
//! *directory* (non-recursive), because atomic temp+rename replaces a file's
//! inode and kills a single-file watch. A file watch is the parent directory
//! with a file-name filter. A debounce thread coalesces a burst into one
//! signal, sets a flag the owner polls from its `App::service_external` pass
//! (never from `ui`), and wakes the UI thread so an idle or occluded host
//! still services it. The thread exits on its own when the watcher drops.
//!
//! The watcher cannot tell who wrote. Owners suppress their own write echoes
//! by comparing content, not by trusting the event.

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::ffi::OsString;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::app::ui_mailbox::UiWake;

/// Quiet period that closes a burst of events into one signal.
pub(crate) const DEBOUNCE: Duration = Duration::from_millis(150);

pub(crate) struct PathWatch {
    changed: Arc<AtomicBool>,
    _watcher: RecommendedWatcher,
}

impl PathWatch {
    /// Watch `dir` (non-recursive). With `file_name` set, only events naming
    /// that file count. `source` labels the UI wake.
    pub(crate) fn new(
        dir: &Path,
        file_name: Option<OsString>,
        wake: Arc<dyn UiWake>,
        source: &'static str,
    ) -> Result<Self, String> {
        let changed = Arc::new(AtomicBool::new(false));
        let (raw_tx, raw_rx) = mpsc::channel::<()>();
        let mut watcher =
            notify::recommended_watcher(move |res: notify::Result<Event>| match res {
                Ok(ev) => {
                    if !matches!(
                        ev.kind,
                        EventKind::Modify(_)
                            | EventKind::Create(_)
                            | EventKind::Remove(_)
                            | EventKind::Any
                    ) {
                        return;
                    }
                    let relevant = match &file_name {
                        Some(name) => ev
                            .paths
                            .iter()
                            .any(|p| p.file_name() == Some(name.as_os_str())),
                        None => true,
                    };
                    if relevant {
                        let _ = raw_tx.send(());
                    }
                }
                Err(e) => log::warn!("path_watch: watcher error ({source}): {e}"),
            })
            .map_err(|e| format!("path_watch: create watcher for {}: {e}", dir.display()))?;
        watcher
            .watch(dir, RecursiveMode::NonRecursive)
            .map_err(|e| format!("path_watch: watch {}: {e}", dir.display()))?;

        let flag = Arc::clone(&changed);
        thread::spawn(move || {
            // Block for the first event, then absorb the burst.
            while raw_rx.recv().is_ok() {
                loop {
                    match raw_rx.recv_timeout(DEBOUNCE) {
                        Ok(()) => continue,
                        Err(RecvTimeoutError::Timeout) => break,
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
                flag.store(true, Ordering::SeqCst);
                wake.wake(source);
            }
        });
        log::info!("path_watch: watching {} ({source})", dir.display());
        Ok(Self {
            changed,
            _watcher: watcher,
        })
    }

    /// True once per debounced burst.
    pub(crate) fn take_changed(&self) -> bool {
        self.changed.swap(false, Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ui_mailbox::RecordingWake;
    use std::time::Instant;

    fn wait_for_change(watch: &PathWatch) -> bool {
        let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(3));
        while Instant::now() < deadline {
            if watch.take_changed() {
                return true;
            }
            thread::sleep(Duration::from_millis(20));
        }
        false
    }

    fn arm() {
        thread::sleep(crate::testing::load_aware_timeout(Duration::from_millis(
            200,
        )));
    }

    #[test]
    fn file_watch_fires_on_write_and_ignores_siblings() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "1").unwrap();
        let wake = Arc::new(RecordingWake::new());
        let watch = PathWatch::new(dir.path(), Some("a.txt".into()), wake.clone(), "test").unwrap();
        arm();
        // FSEvents can replay the seed write after the watch arms; settle it.
        thread::sleep(crate::testing::load_aware_timeout(Duration::from_millis(
            500,
        )));
        let _ = watch.take_changed();

        std::fs::write(dir.path().join("other.txt"), "x").unwrap();
        thread::sleep(crate::testing::load_aware_timeout(Duration::from_millis(
            500,
        )));
        assert!(!watch.take_changed(), "sibling writes are filtered out");

        std::fs::write(&file, "2").unwrap();
        assert!(wait_for_change(&watch));
        assert!(!wake.sources().is_empty(), "a change wakes the UI thread");
    }

    #[test]
    fn dir_watch_fires_on_create() {
        let dir = tempfile::tempdir().unwrap();
        let wake = Arc::new(RecordingWake::new());
        let watch = PathWatch::new(dir.path(), None, wake, "test").unwrap();
        arm();
        std::fs::write(dir.path().join("new.txt"), "x").unwrap();
        assert!(wait_for_change(&watch));
    }
}
