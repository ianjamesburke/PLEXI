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
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::app::ui_mailbox::{MailboxReceiver, UiMailbox, UiWake};
use crate::spatial::tiling::PaneId;

/// Debounce window — bursts of save events within this much of each other
/// coalesce into a single reload request.
pub const DEBOUNCE_MS: u64 = 250;

/// Directory names that never contain app source and must never trigger a
/// reload — tooling output from running the app's own checks (`plexi app
/// check` / `app test`) while the pane is open. Checked against every path
/// component, not just the leaf, so `<app>/.venv/lib/...` is excluded too.
const EXCLUDED_DIR_NAMES: &[&str] = &[".venv", "__pycache__", ".pytest_cache", ".mypy_cache"];

/// Returns true if `path` (an absolute event path somewhere under
/// `app_dir`) should trigger a reload — i.e. no component of its path
/// *relative to `app_dir`* names an excluded tooling directory or a
/// dotfile/dot-directory.
///
/// Only components below `app_dir` are examined: the app's own absolute
/// path (e.g. a dotfile in the user's home directory ancestry — every
/// installed app lives under `~/.plexi-<channel>/apps/<name>`) must never
/// affect the classification. Single source of truth for the exclusion
/// set — both the event filter and any future caller (docs, tests) go
/// through this function rather than re-deriving the list.
///
/// Fails open: if `path` cannot be stripped of the `app_dir` prefix (a
/// non-canonical event path, a differing symlink resolution), there is
/// nothing safe to classify, so the event is treated as relevant rather
/// than scanning the full path — which would misclassify every event for
/// any installed app, since `~/.plexi-<channel>/...` is itself a dot-dir
/// ancestor. Watching a bit too much beats hot reload dying silently.
fn is_reload_relevant(path: &Path, app_dir: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(app_dir) else {
        return true;
    };
    for component in relative.components() {
        let std::path::Component::Normal(name) = component else {
            continue;
        };
        let Some(name) = name.to_str() else {
            // Non-UTF-8 component — cannot classify it, so exclude
            // defensively rather than trigger on it unexamined.
            return false;
        };
        if EXCLUDED_DIR_NAMES.contains(&name) {
            return false;
        }
        if name.starts_with('.') {
            return false;
        }
    }
    true
}

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
    /// Mailbox shared with every watcher's debouncer thread; each send wakes
    /// the UI thread so a reload is never stranded on an idle host.
    sender: UiMailbox<ReloadRequest>,
}

impl HotReloadWatcher {
    /// Construct a new watcher set + the matching receiver. The host stores
    /// the receiver and drains it each frame; one `ReloadRequest` per
    /// debounce window.
    pub fn new(wake: Arc<dyn UiWake>) -> (Self, MailboxReceiver<ReloadRequest>) {
        let (tx, rx) = UiMailbox::channel(wake, "hot_reload");
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

        // Canonicalize for classification only (never for the actual
        // `watcher.watch` call below): on macOS `/var` is a symlink to
        // `/private/var`, so a tempdir-style path and the paths FSEvents
        // reports for it differ textually even though they name the same
        // directory. Without this, `strip_prefix` below silently falls
        // back to the full path and every component of the app's own
        // (often dot-prefixed, e.g. under a `.tmp*` tempdir) ancestry gets
        // misclassified as excluded.
        let watch_dir = app_dir
            .canonicalize()
            .unwrap_or_else(|_| app_dir.to_path_buf());
        let logged_ignored = Arc::new(Mutex::new(false));
        let mut watcher = match notify::recommended_watcher(move |res: notify::Result<Event>| {
            match res {
                Ok(ev) => {
                    // Drop noisy access events; we only care about content
                    // changes (Modify, Create, Remove). On macOS, the
                    // FSEvents backend reports `Any` for many save flows;
                    // accept those too.
                    if !matches!(
                        ev.kind,
                        EventKind::Modify(_)
                            | EventKind::Create(_)
                            | EventKind::Remove(_)
                            | EventKind::Any
                    ) {
                        return;
                    }
                    // Filter out tooling output (`.venv`, `__pycache__`,
                    // etc.) so running the app's own checks does not
                    // restart the pane — see `is_reload_relevant`. An
                    // event with no paths carries nothing to classify, so
                    // it is never filtered.
                    let relevant = ev.paths.is_empty()
                        || ev.paths.iter().any(|p| is_reload_relevant(p, &watch_dir));
                    if !relevant {
                        let mut logged = logged_ignored.lock().unwrap_or_else(|e| e.into_inner());
                        if !*logged {
                            *logged = true;
                            log::info!(
                                "hot_reload: ignoring tooling-output events under {watch_dir:?} for pane {pane_id} (.venv/__pycache__/dotfiles)"
                            );
                        }
                        return;
                    }
                    let _ = raw_tx.send(ev);
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
            log::error!("hot_reload: failed to begin watching {app_dir:?} for pane {pane_id}: {e}");
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
    #[cfg(test)]
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
    sender: UiMailbox<ReloadRequest>,
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
                        log::info!("hot_reload: debounced reload for pane {pane_id} ({app_dir:?})");
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
    use crate::app::ui_mailbox::RecordingWake;
    use std::fs;
    use std::time::Duration;
    use tempfile::tempdir;

    fn poll_for_reload(
        rx: &MailboxReceiver<ReloadRequest>,
        base_timeout: Duration,
    ) -> Option<ReloadRequest> {
        let deadline = Instant::now() + crate::testing::load_aware_timeout(base_timeout);
        while Instant::now() < deadline {
            if let Ok(req) = rx.try_recv() {
                return Some(req);
            }
            thread::sleep(Duration::from_millis(20));
        }
        None
    }

    #[test]
    fn is_reload_relevant_excludes_venv_and_caches() {
        let app_dir = Path::new("/apps/foo");
        assert!(!is_reload_relevant(
            &app_dir.join(".venv/lib/site-packages/x.py"),
            app_dir
        ));
        assert!(!is_reload_relevant(
            &app_dir.join("__pycache__/main.cpython-312.pyc"),
            app_dir
        ));
        assert!(!is_reload_relevant(
            &app_dir.join(".pytest_cache/v/cache/lastfailed"),
            app_dir
        ));
        assert!(!is_reload_relevant(
            &app_dir.join(".mypy_cache/3.12"),
            app_dir
        ));
        assert!(!is_reload_relevant(&app_dir.join(".DS_Store"), app_dir));
    }

    #[test]
    fn is_reload_relevant_allows_source_files() {
        let app_dir = Path::new("/apps/foo");
        assert!(is_reload_relevant(&app_dir.join("main.py"), app_dir));
        assert!(is_reload_relevant(&app_dir.join("lib/helpers.py"), app_dir));
    }

    #[test]
    fn is_reload_relevant_ignores_dotfiles_in_ancestor_path() {
        // A dot component above `app_dir` (e.g. the user's home directory
        // ancestry) must never affect classification — only the path
        // relative to `app_dir` is examined.
        let app_dir = Path::new("/Users/.hidden-home/apps/foo");
        assert!(is_reload_relevant(&app_dir.join("main.py"), app_dir));
    }

    #[test]
    fn is_reload_relevant_fails_open_when_path_is_not_under_app_dir() {
        // A prefix mismatch (non-canonical event path, differing symlink
        // resolution) leaves nothing safe to classify. Every installed app
        // lives under `~/.plexi-<channel>/apps/<name>` — itself a dot-dir
        // ancestor — so falling back to scanning the full path would
        // misclassify every event for an installed app as irrelevant and
        // kill hot reload silently. Fail open instead.
        let app_dir = Path::new("/Users/.plexi-alpha/apps/foo");
        let unrelated = Path::new("/Users/.plexi-alpha/apps/bar/main.py");
        assert!(is_reload_relevant(unrelated, app_dir));
    }

    #[test]
    fn watcher_ignores_venv_writes_but_fires_on_source_change() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".venv/lib")).unwrap();
        let source = dir.path().join("main.py");
        fs::write(&source, "print('v1')\n").unwrap();

        let (mut watcher, rx) = HotReloadWatcher::new(Arc::new(RecordingWake::new()));
        watcher.watch(99, dir.path());
        thread::sleep(crate::testing::load_aware_timeout(Duration::from_millis(
            150,
        )));
        // Drain any reload from FSEvents replaying the pre-watch creation
        // of `main.py` itself — irrelevant to what this test checks.
        let _ = poll_for_reload(&rx, Duration::from_millis(500));
        while rx.try_recv().is_ok() {}

        // Simulate tooling output from `plexi app check` / `app test`.
        fs::write(dir.path().join(".venv/lib/marker.txt"), "x").unwrap();
        let got = poll_for_reload(&rx, Duration::from_millis(800));
        assert!(
            got.is_none(),
            "a write under .venv must never produce a ReloadRequest, got {got:?}"
        );

        // A genuine source edit still reloads.
        fs::write(&source, "print('v2')\n").unwrap();
        let got = poll_for_reload(&rx, Duration::from_secs(3));
        assert_eq!(
            got,
            Some(ReloadRequest { pane_id: 99 }),
            "a write to main.py must still produce a ReloadRequest"
        );
    }

    #[test]
    fn watcher_ignores_pycache_writes() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("__pycache__")).unwrap();

        let (mut watcher, rx) = HotReloadWatcher::new(Arc::new(RecordingWake::new()));
        watcher.watch(100, dir.path());
        thread::sleep(crate::testing::load_aware_timeout(Duration::from_millis(
            150,
        )));
        // Drain any reload from FSEvents replaying the pre-watch creation
        // of the tempdir/`__pycache__` themselves.
        let _ = poll_for_reload(&rx, Duration::from_millis(500));
        while rx.try_recv().is_ok() {}

        fs::write(dir.path().join("__pycache__/main.cpython-312.pyc"), "x").unwrap();
        let got = poll_for_reload(&rx, Duration::from_millis(800));
        assert!(
            got.is_none(),
            "a write under __pycache__ must never produce a ReloadRequest, got {got:?}"
        );
    }

    #[test]
    fn watcher_fires_event_on_file_change() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("source.py");
        fs::write(&file, "print('v1')\n").unwrap();

        let wake = Arc::new(RecordingWake::new());
        let (mut watcher, rx) = HotReloadWatcher::new(wake.clone());
        watcher.watch(42, dir.path());

        // Give FSEvents a moment to arm before mutating.
        thread::sleep(crate::testing::load_aware_timeout(Duration::from_millis(
            150,
        )));
        fs::write(&file, "print('v2')\n").unwrap();

        let got = poll_for_reload(&rx, Duration::from_secs(3));
        assert_eq!(
            got,
            Some(ReloadRequest { pane_id: 42 }),
            "save should yield a debounced ReloadRequest within 3s"
        );
        assert!(
            wake.sources().contains(&"hot_reload"),
            "debounced reload must wake the UI thread"
        );
    }

    #[test]
    fn watcher_debounces_burst_to_single_event() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("burst.py");
        fs::write(&file, "v0\n").unwrap();

        let (mut watcher, rx) = HotReloadWatcher::new(Arc::new(RecordingWake::new()));
        watcher.watch(7, dir.path());
        thread::sleep(crate::testing::load_aware_timeout(Duration::from_millis(
            150,
        )));

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
        let deadline =
            Instant::now() + crate::testing::load_aware_timeout(Duration::from_millis(600));
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

        let (mut watcher, rx) = HotReloadWatcher::new(Arc::new(RecordingWake::new()));
        watcher.watch(11, dir.path());
        thread::sleep(crate::testing::load_aware_timeout(Duration::from_millis(
            150,
        )));

        watcher.unwatch(11);
        // Drain any in-flight event from before the unwatch.
        thread::sleep(crate::testing::load_aware_timeout(Duration::from_millis(
            400,
        )));
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
        let (mut watcher, _rx) = HotReloadWatcher::new(Arc::new(RecordingWake::new()));
        watcher.unwatch(999); // never watched
        assert_eq!(watcher.len(), 0);
    }

    #[test]
    fn watched_pane_ids_returns_all_watched() {
        let dir = tempdir().unwrap();
        let (mut watcher, _rx) = HotReloadWatcher::new(Arc::new(RecordingWake::new()));
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
