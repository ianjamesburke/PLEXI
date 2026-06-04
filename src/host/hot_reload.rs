//! Hot reload (#83) — watches an app's directory for source changes and
//! emits a debounced `ReloadRequest` per pane.
//!
//! # Design
//!
//! - One `notify::RecommendedWatcher` per watched pane (FSEvents on macOS).
//! - Recursive watch on the app's directory — the dir is small in practice.
//! - In-house 250ms debounce window (saves trigger 5+ events per save on
//!   macOS); no extra crate. Per-pane debouncer thread coalesces a burst
//!   into a single `ReloadRequest`.
//! - Per-pane handles owned by `HotReloadWatcher::watchers`. Dropping a
//!   handle stops the underlying `Watcher` and signals the debouncer to
//!   exit.
//!
//! # Invariants
//!
//! - `unwatch(pane_id)` is idempotent — closing a pane that wasn't watched
//!   is a no-op, not a panic.
//! - The host receives `ReloadRequest { pane_id }` exclusively through the
//!   channel passed to `HotReloadWatcher::new`. No other side-effects.
//! - Watching is opt-in (manifest `[app] watch = true`) and gated to
//!   workspace-local installs by the caller (`AppRegistry::launch_process`).

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::spatial::tiling::PaneId;

/// Debounce window — bursts of save events within this much of each other
/// coalesce into a single reload request.
pub const DEBOUNCE_MS: u64 = 250;

/// Sent on each debounced filesystem change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReloadRequest {
    pub pane_id: PaneId,
}

/// Owns the lifetime of one pane's watcher + debouncer thread.
///
/// Drop semantics:
/// - The `RecommendedWatcher` is dropped, releasing FS resources.
/// - The debouncer thread polls the cancellation flag; we set it then join.
struct WatcherHandle {
    /// Cancellation flag — set to true to signal the debouncer thread to exit.
    cancel: Arc<Mutex<bool>>,
    /// Debouncer thread join handle. Joined on drop best-effort.
    thread: Option<JoinHandle<()>>,
    /// The notify watcher. Held to keep watching alive; dropped to stop.
    /// Public reads are not needed — the field exists solely for its drop.
    _watcher: RecommendedWatcher,
}

impl Drop for WatcherHandle {
    fn drop(&mut self) {
        if let Ok(mut c) = self.cancel.lock() {
            *c = true;
        }
        if let Some(handle) = self.thread.take() {
            // Best-effort join; the debouncer wakes within DEBOUNCE_MS and
            // will exit on the next loop iteration.
            let _ = handle.join();
        }
        // _watcher drops here, releasing FSEvents resources.
    }
}

/// Manages per-pane file-system watchers. The host owns one of these and
/// asks for a watcher each time a watching-eligible app launches; the
/// host calls `unwatch(pane_id)` when the pane closes (or the app reloads
/// — though reload reuses the same watcher since the dir is unchanged).
pub struct HotReloadWatcher {
    /// One handle per actively-watched pane.
    watchers: HashMap<PaneId, WatcherHandle>,
    /// Channel sender shared with every watcher's debouncer thread.
    sender: Sender<ReloadRequest>,
}

impl HotReloadWatcher {
    /// Construct a new watcher set + the matching receiver. The host stores
    /// the receiver and drains it each frame; one `ReloadRequest` per
    /// debounce window.
    pub fn new() -> (Self, Receiver<ReloadRequest>) {
        let (tx, rx) = mpsc::channel();
        (
            Self {
                watchers: HashMap::new(),
                sender: tx,
            },
            rx,
        )
    }

    /// Begin watching `app_dir` for `pane_id`. Replaces any existing watcher
    /// for the same pane (caller responsibility — typically a no-op since
    /// reload reuses the watcher).
    pub fn watch(&mut self, pane_id: PaneId, app_dir: &Path) {
        // Replace any existing watcher (idempotent).
        self.watchers.remove(&pane_id);

        let cancel = Arc::new(Mutex::new(false));
        let cancel_thread = Arc::clone(&cancel);
        let sender = self.sender.clone();

        // Internal channel: notify watcher → debouncer thread.
        let (raw_tx, raw_rx) = mpsc::channel::<Event>();

        let mut watcher = match notify::recommended_watcher(move |res: notify::Result<Event>| {
            match res {
                Ok(ev) => {
                    // Drop noisy access events; we only care about content
                    // changes (Modify, Create, Remove). On macOS, the
                    // FSEvents backend reports `Any` for many save flows;
                    // accept those too.
                    if matches!(
                        ev.kind,
                        EventKind::Modify(_)
                            | EventKind::Create(_)
                            | EventKind::Remove(_)
                            | EventKind::Any
                    ) {
                        let _ = raw_tx.send(ev);
                    }
                }
                Err(e) => log::warn!("hot_reload: watcher error: {e}"),
            }
        }) {
            Ok(w) => w,
            Err(e) => {
                log::error!(
                    "hot_reload: failed to create watcher for pane {pane_id} at {app_dir:?}: {e}"
                );
                return;
            }
        };

        if let Err(e) = watcher.watch(app_dir, RecursiveMode::Recursive) {
            log::error!(
                "hot_reload: failed to begin watching {app_dir:?} for pane {pane_id}: {e}"
            );
            return;
        }

        let app_dir_log = app_dir.to_path_buf();
        let thread = thread::spawn(move || {
            debounce_loop(pane_id, raw_rx, sender, cancel_thread, app_dir_log);
        });

        self.watchers.insert(
            pane_id,
            WatcherHandle {
                cancel,
                thread: Some(thread),
                _watcher: watcher,
            },
        );
        log::info!("hot_reload: watching {app_dir:?} for pane {pane_id}");
    }

    /// Stop watching `pane_id`. Idempotent — closing a pane that was never
    /// watched is a no-op.
    pub fn unwatch(&mut self, pane_id: PaneId) {
        if self.watchers.remove(&pane_id).is_some() {
            log::info!("hot_reload: stopped watching pane {pane_id}");
        }
    }

    /// Returns the pane IDs of all actively-watched panes.
    pub fn watched_pane_ids(&self) -> Vec<PaneId> {
        self.watchers.keys().copied().collect()
    }

    /// Test-only — number of active watchers.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.watchers.len()
    }
}

/// Debounce loop: collect events into a window, emit one `ReloadRequest`
/// when the window closes (no new events within `DEBOUNCE_MS`).
///
/// The loop polls every 50ms so cancellation can be observed promptly even
/// when the watcher is silent.
fn debounce_loop(
    pane_id: PaneId,
    rx: Receiver<Event>,
    sender: Sender<ReloadRequest>,
    cancel: Arc<Mutex<bool>>,
    app_dir: PathBuf,
) {
    let mut last_event: Option<Instant> = None;
    let debounce = Duration::from_millis(DEBOUNCE_MS);
    let poll = Duration::from_millis(50);

    loop {
        // Cancellation check.
        if cancel.lock().map(|g| *g).unwrap_or(true) {
            return;
        }

        match rx.try_recv() {
            Ok(_ev) => {
                last_event = Some(Instant::now());
            }
            Err(TryRecvError::Disconnected) => {
                // Watcher dropped — exit cleanly.
                return;
            }
            Err(TryRecvError::Empty) => {
                if let Some(t) = last_event {
                    if t.elapsed() >= debounce {
                        log::info!(
                            "hot_reload: debounced reload for pane {pane_id} ({app_dir:?})"
                        );
                        if sender.send(ReloadRequest { pane_id }).is_err() {
                            // Host receiver dropped — nothing more to do.
                            return;
                        }
                        last_event = None;
                    }
                }
                thread::sleep(poll);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;
    use tempfile::tempdir;

    fn poll_for_reload(rx: &Receiver<ReloadRequest>, timeout: Duration) -> Option<ReloadRequest> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Ok(req) = rx.try_recv() {
                return Some(req);
            }
            thread::sleep(Duration::from_millis(20));
        }
        None
    }

    #[test]
    fn watcher_fires_event_on_file_change() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("source.py");
        fs::write(&file, "print('v1')\n").unwrap();

        let (mut watcher, rx) = HotReloadWatcher::new();
        watcher.watch(42, dir.path());

        // Give FSEvents a moment to arm before mutating.
        thread::sleep(Duration::from_millis(150));
        fs::write(&file, "print('v2')\n").unwrap();

        let got = poll_for_reload(&rx, Duration::from_secs(3));
        assert_eq!(
            got,
            Some(ReloadRequest { pane_id: 42 }),
            "save should yield a debounced ReloadRequest within 3s"
        );
    }

    #[test]
    fn watcher_debounces_burst_to_single_event() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("burst.py");
        fs::write(&file, "v0\n").unwrap();

        let (mut watcher, rx) = HotReloadWatcher::new();
        watcher.watch(7, dir.path());
        thread::sleep(Duration::from_millis(150));

        // Fire a tight burst — every write within ~10ms.
        for i in 0..6 {
            fs::write(&file, format!("v{i}\n")).unwrap();
            thread::sleep(Duration::from_millis(10));
        }

        // First event arrives after debounce + jitter.
        let first = poll_for_reload(&rx, Duration::from_secs(3));
        assert!(first.is_some(), "expected at least one reload");

        // Drain anything else that arrives in a 600ms tail. With a 250ms
        // debounce the burst should coalesce — at most one extra is OK
        // (FSEvents occasionally splits a burst across two windows).
        let mut extras = 0;
        let deadline = Instant::now() + Duration::from_millis(600);
        while Instant::now() < deadline {
            if rx.try_recv().is_ok() {
                extras += 1;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert!(
            extras <= 1,
            "burst of 6 writes should debounce to 1–2 reloads, got {} extras",
            extras
        );
    }

    #[test]
    fn unwatch_stops_event_delivery() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("stop.py");
        fs::write(&file, "v0\n").unwrap();

        let (mut watcher, rx) = HotReloadWatcher::new();
        watcher.watch(11, dir.path());
        thread::sleep(Duration::from_millis(150));

        watcher.unwatch(11);
        // Drain any in-flight event from before the unwatch.
        thread::sleep(Duration::from_millis(400));
        while rx.try_recv().is_ok() {}

        // Subsequent edits should never produce a ReloadRequest.
        fs::write(&file, "v1\n").unwrap();
        let got = poll_for_reload(&rx, Duration::from_millis(800));
        assert!(
            got.is_none(),
            "no event should be delivered after unwatch, got {got:?}"
        );
    }

    #[test]
    fn unwatch_is_idempotent_for_unknown_pane() {
        let (mut watcher, _rx) = HotReloadWatcher::new();
        watcher.unwatch(999); // never watched
        assert_eq!(watcher.len(), 0);
    }

    #[test]
    fn watched_pane_ids_returns_all_watched() {
        let dir = tempdir().unwrap();
        let (mut watcher, _rx) = HotReloadWatcher::new();
        watcher.watch(10, dir.path());
        watcher.watch(20, dir.path());

        let mut ids = watcher.watched_pane_ids();
        ids.sort();
        assert_eq!(ids, vec![10, 20]);

        watcher.unwatch(10);
        let mut ids = watcher.watched_pane_ids();
        ids.sort();
        assert_eq!(ids, vec![20]);
    }
}
