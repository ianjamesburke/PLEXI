//! Watches `config.toml` for filesystem changes and emits debounced reload
//! signals so the host can call `reload_config()` without a manual shortcut.

use crate::app::ui_mailbox::{MailboxReceiver, UiMailbox, UiWake};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
#[cfg(test)]
use std::time::Instant;

const DEBOUNCE_MS: u64 = 200;

pub struct ConfigWatcher {
    _watcher: RecommendedWatcher,
    cancel: Arc<Mutex<bool>>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for ConfigWatcher {
    fn drop(&mut self) {
        if let Ok(mut c) = self.cancel.lock() {
            *c = true;
        }
        if let Some(h) = self.thread.take() {
            let _ = h.join();
        }
    }
}

/// Start watching `config_path` for changes. Returns a receiver that yields
/// `()` on each debounced config change, plus the watcher handle (must be kept
/// alive). Each signal wakes the UI thread via `wake` — a fully idle host
/// produces zero frames, and the receiver is only drained during a frame.
/// Returns `None` if the watcher could not be created.
pub fn start(
    config_path: PathBuf,
    wake: Arc<dyn UiWake>,
) -> Option<(ConfigWatcher, MailboxReceiver<()>)> {
    let watch_dir = config_path.parent()?.to_path_buf();
    let target_name = config_path.file_name()?.to_os_string();

    let (reload_tx, reload_rx) = UiMailbox::channel(wake, "config_watcher");
    let (raw_tx, raw_rx) = mpsc::channel::<()>();

    let filter_name = target_name;
    let mut watcher = match notify::recommended_watcher(move |res: notify::Result<Event>| match res
    {
        Ok(ev) => {
            if matches!(ev.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                let dominated = ev.paths.iter().any(|p| p.file_name() == Some(&filter_name));
                if dominated {
                    let _ = raw_tx.send(());
                }
            }
        }
        Err(e) => log::warn!("config_watcher: watcher error: {e}"),
    }) {
        Ok(w) => w,
        Err(e) => {
            log::warn!("config_watcher: failed to create watcher: {e}");
            return None;
        }
    };

    if let Err(e) = watcher.watch(&watch_dir, RecursiveMode::NonRecursive) {
        log::warn!("config_watcher: failed to watch {watch_dir:?}: {e}");
        return None;
    }

    log::info!("config_watcher: watching {config_path:?}");

    let cancel = Arc::new(Mutex::new(false));
    let cancel_thread = Arc::clone(&cancel);

    let thread = thread::spawn(move || {
        crate::app::debounce::run(
            raw_rx,
            cancel_thread,
            Duration::from_millis(DEBOUNCE_MS),
            |_| {
                log::info!("config_watcher: config.toml changed, reloading");
                reload_tx.send(()).is_ok()
            },
        );
    });

    Some((
        ConfigWatcher {
            _watcher: watcher,
            cancel,
            thread: Some(thread),
        },
        reload_rx,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ui_mailbox::RecordingWake;
    use std::fs;
    use tempfile::tempdir;

    fn poll_for_signal(rx: &MailboxReceiver<()>, base_timeout: Duration) -> bool {
        let deadline = Instant::now() + crate::testing::load_aware_timeout(base_timeout);
        while Instant::now() < deadline {
            if rx.try_recv().is_ok() {
                return true;
            }
            thread::sleep(Duration::from_millis(20));
        }
        false
    }

    /// Absorb the `config.toml` create that `start` may deliver late, then
    /// wait out one debounce of silence.
    ///
    /// A fixed sleep plus a single drain loses to FSEvents latency on a loaded
    /// macOS runner. The create's debounced signal then lands in the next
    /// assertion and looks like an unrelated file woke the watcher.
    fn settle_initial_events(rx: &MailboxReceiver<()>) {
        let _ = poll_for_signal(rx, Duration::from_secs(2));
        let quiet = crate::testing::load_aware_timeout(Duration::from_millis(DEBOUNCE_MS + 200));
        let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(3));
        let mut quiet_since = Instant::now();
        while Instant::now() < deadline {
            if rx.try_recv().is_ok() {
                quiet_since = Instant::now();
            } else if quiet_since.elapsed() >= quiet {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn fires_on_config_change() {
        let dir = tempdir().unwrap();
        let config = dir.path().join("config.toml");
        fs::write(&config, "# v1\n").unwrap();

        let wake = Arc::new(RecordingWake::new());
        let (watcher, rx) = start(config.clone(), wake.clone()).expect("watcher should start");
        thread::sleep(crate::testing::load_aware_timeout(Duration::from_millis(
            150,
        )));

        fs::write(&config, "# v2\n").unwrap();
        assert!(
            poll_for_signal(&rx, Duration::from_secs(3)),
            "should fire after config change"
        );
        assert!(
            wake.sources().contains(&"config_watcher"),
            "debounced signal must wake the UI thread"
        );
        drop(watcher);
    }

    #[test]
    fn ignores_unrelated_files() {
        let dir = tempdir().unwrap();
        let config = dir.path().join("config.toml");
        fs::write(&config, "# v1\n").unwrap();

        let (watcher, rx) =
            start(config, Arc::new(RecordingWake::new())).expect("watcher should start");
        settle_initial_events(&rx);

        fs::write(dir.path().join("other.txt"), "noise\n").unwrap();
        assert!(
            !poll_for_signal(&rx, Duration::from_millis(800)),
            "should not fire for unrelated file"
        );
        drop(watcher);
    }

    #[test]
    fn debounces_burst() {
        let dir = tempdir().unwrap();
        let config = dir.path().join("config.toml");
        fs::write(&config, "# v0\n").unwrap();

        let (_watcher, rx) =
            start(config.clone(), Arc::new(RecordingWake::new())).expect("watcher should start");
        thread::sleep(crate::testing::load_aware_timeout(Duration::from_millis(
            150,
        )));

        for i in 0..5 {
            fs::write(&config, format!("# v{i}\n")).unwrap();
            thread::sleep(Duration::from_millis(10));
        }

        assert!(poll_for_signal(&rx, Duration::from_secs(3)));

        let mut extras = 0;
        let deadline =
            Instant::now() + crate::testing::load_aware_timeout(Duration::from_millis(500));
        while Instant::now() < deadline {
            if rx.try_recv().is_ok() {
                extras += 1;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert!(
            extras <= 1,
            "burst of 5 writes should debounce to 1-2 signals, got {} extras",
            extras
        );
    }
}
