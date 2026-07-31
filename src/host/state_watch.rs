//! App state file watcher (stint 0644) — watches each Python pane's state
//! files for *external* writes (CLI, agents, editors) and emits a debounced
//! `StateChangedNotice` per `(pane, scope)`.
//!
//! # Design
//!
//! Modelled on `crate::host::hot_reload`, with two deliberate differences:
//!
//! - **The watch target is the state file's PARENT DIRECTORY, non-recursive**,
//!   never the file itself. Everything writing state uses atomic temp+rename,
//!   which replaces the inode — a single-file watch dies after the first
//!   replacement. Events are filtered by file name instead.
//! - **The watched dirs are shared** (`~/.plexi/app_states/` holds every
//!   app's global state), so filename filtering is mandatory: without it,
//!   every app's save would fire every other app's watcher.
//!
//! Same 250ms debounce-thread pattern as hot_reload; one debouncer thread per
//! watched pane coalesces a burst into one notice per scope. Self-write
//! echoes are suppressed downstream by `LivePythonPane::apply_external_state`
//! via the cached `(mtime, len)` identity — the watcher itself cannot tell
//! who wrote.
//!
//! # Invariants
//!
//! - `unwatch(pane_id)` is idempotent.
//! - Notices arrive exclusively through the channel handed out by
//!   `StateWatcher::new`. No other side effects.

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::app::ui_mailbox::{MailboxReceiver, UiMailbox, UiWake};
use crate::host::state_scope::StateScope;
use crate::spatial::tiling::PaneId;

/// Debounce window — bursts of writes to the same state file within this much
/// of each other coalesce into a single notice.
pub const DEBOUNCE_MS: u64 = 250;

/// Sent on each debounced state-file change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateChangedNotice {
    pub pane_id: PaneId,
    pub scope: StateScope,
}

/// Owns the lifetime of one pane's watchers + debouncer thread.
struct WatcherHandle {
    /// Cancellation flag — set to true to signal the debouncer thread to exit.
    cancel: Arc<Mutex<bool>>,
    /// Debouncer thread join handle. Joined on drop best-effort.
    thread: Option<JoinHandle<()>>,
    /// One notify watcher per watched parent directory. Held for drop.
    _watchers: Vec<RecommendedWatcher>,
    /// The exact `(scope, path)` set registered, so callers can detect when a
    /// pane's resolved paths drift (e.g. `plexi context set-root`) and
    /// re-register.
    entries: Vec<(StateScope, PathBuf)>,
}

impl Drop for WatcherHandle {
    fn drop(&mut self) {
        if let Ok(mut c) = self.cancel.lock() {
            *c = true;
        }
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
        // _watchers drop here, releasing FS resources.
    }
}

/// Manages per-pane state-file watchers. The host owns one of these, registers
/// each file-backed app pane at launch, re-syncs registrations when a pane's
/// resolved state paths change, and unwatches on pane close.
pub struct StateWatcher {
    watchers: HashMap<PaneId, WatcherHandle>,
    /// Mailbox shared with every debouncer thread; each send wakes the UI
    /// thread so an external edit is never stranded on an idle host.
    sender: UiMailbox<StateChangedNotice>,
}

impl StateWatcher {
    /// Construct the watcher set + matching receiver. The host stores the
    /// receiver and drains it each `logic` pass.
    pub fn new(wake: Arc<dyn UiWake>) -> (Self, MailboxReceiver<StateChangedNotice>) {
        let (tx, rx) = UiMailbox::channel(wake, "state_watch");
        (
            Self {
                watchers: HashMap::new(),
                sender: tx,
            },
            rx,
        )
    }

    /// The `(scope, path)` set currently registered for `pane_id`. Empty when
    /// the pane is not watched. Used by the drain pass to detect a context
    /// root change without extra bookkeeping.
    pub fn watched_entries(&self, pane_id: PaneId) -> &[(StateScope, PathBuf)] {
        self.watchers
            .get(&pane_id)
            .map(|handle| handle.entries.as_slice())
            .unwrap_or(&[])
    }

    /// Begin watching `entries` (each a declared scope's resolved state file)
    /// for `pane_id`. Replaces any existing registration for the pane.
    pub fn watch(&mut self, pane_id: PaneId, entries: &[(StateScope, PathBuf)]) {
        // Replace any existing registration (idempotent).
        self.watchers.remove(&pane_id);
        if entries.is_empty() {
            return;
        }

        let cancel = Arc::new(Mutex::new(false));
        let cancel_thread = Arc::clone(&cancel);
        let sender = self.sender.clone();

        // Internal channel: every notify watcher → the pane's debouncer.
        // Carries the matched scope so the debouncer never re-derives paths.
        let (raw_tx, raw_rx) = mpsc::channel::<StateScope>();

        let mut watchers = Vec::new();
        for (scope, path) in entries {
            let Some(parent) = path.parent().map(Path::to_path_buf) else {
                log::error!(
                    "state_watch: state path {} has no parent — cannot watch (pane {pane_id})",
                    path.display()
                );
                continue;
            };
            let Some(file_name) = path.file_name().map(|n| n.to_os_string()) else {
                log::error!(
                    "state_watch: state path {} has no file name — cannot watch (pane {pane_id})",
                    path.display()
                );
                continue;
            };
            // The parent may not exist before the first persist; create it so
            // the watch can arm now and catch that first external write.
            if let Err(error) = std::fs::create_dir_all(&parent) {
                log::error!(
                    "state_watch: create state dir {} for pane {pane_id}: {error}",
                    parent.display()
                );
                continue;
            }
            let scope = *scope;
            let tx = raw_tx.clone();
            let mut watcher =
                match notify::recommended_watcher(move |res: notify::Result<Event>| {
                    match res {
                        Ok(ev) => {
                            // Content changes only; macOS FSEvents reports
                            // `Any` for many save flows — accept those too.
                            if !matches!(
                                ev.kind,
                                EventKind::Modify(_)
                                    | EventKind::Create(_)
                                    | EventKind::Remove(_)
                                    | EventKind::Any
                            ) {
                                return;
                            }
                            // Shared dir: only this app's file counts. An
                            // atomic rename reports the destination path, so
                            // matching the final file name is sufficient.
                            if ev
                                .paths
                                .iter()
                                .any(|p| p.file_name() == Some(file_name.as_os_str()))
                            {
                                let _ = tx.send(scope);
                            }
                        }
                        Err(e) => log::warn!("state_watch: watcher error: {e}"),
                    }
                }) {
                    Ok(w) => w,
                    Err(e) => {
                        log::error!(
                            "state_watch: failed to create watcher for pane {pane_id} at {}: {e}",
                            parent.display()
                        );
                        continue;
                    }
                };
            // NonRecursive parent-dir watch: survives atomic rename (which
            // replaces the file's inode) and never descends into siblings.
            if let Err(e) = watcher.watch(&parent, RecursiveMode::NonRecursive) {
                log::error!(
                    "state_watch: failed to begin watching {} for pane {pane_id}: {e}",
                    parent.display()
                );
                continue;
            }
            log::info!(
                "state_watch: watching {} (scope={}) for pane {pane_id}",
                path.display(),
                scope.as_str()
            );
            watchers.push(watcher);
        }
        drop(raw_tx);
        if watchers.is_empty() {
            return;
        }

        let thread = thread::spawn(move || {
            debounce_loop(pane_id, raw_rx, sender, cancel_thread);
        });

        self.watchers.insert(
            pane_id,
            WatcherHandle {
                cancel,
                thread: Some(thread),
                _watchers: watchers,
                entries: entries.to_vec(),
            },
        );
    }

    /// Stop watching `pane_id`. Idempotent — closing a pane that was never
    /// watched is a no-op.
    pub fn unwatch(&mut self, pane_id: PaneId) {
        if self.watchers.remove(&pane_id).is_some() {
            log::info!("state_watch: stopped watching pane {pane_id}");
        }
    }
}

/// Debounce loop: collect per-scope events into a window, emit one
/// `StateChangedNotice` per changed scope when the window closes (no new
/// events within `DEBOUNCE_MS`). Polls every 50ms so cancellation is observed
/// promptly even when the watchers are silent.
fn debounce_loop(
    pane_id: PaneId,
    rx: Receiver<StateScope>,
    sender: UiMailbox<StateChangedNotice>,
    cancel: Arc<Mutex<bool>>,
) {
    let mut pending: HashMap<StateScope, Instant> = HashMap::new();
    let debounce = Duration::from_millis(DEBOUNCE_MS);
    let poll = Duration::from_millis(50);

    loop {
        if cancel.lock().map(|g| *g).unwrap_or(true) {
            return;
        }

        match rx.try_recv() {
            Ok(scope) => {
                pending.insert(scope, Instant::now());
            }
            Err(TryRecvError::Disconnected) => {
                // All watchers dropped — exit cleanly.
                return;
            }
            Err(TryRecvError::Empty) => {
                let due: Vec<StateScope> = pending
                    .iter()
                    .filter(|(_, t)| t.elapsed() >= debounce)
                    .map(|(scope, _)| *scope)
                    .collect();
                for scope in due {
                    pending.remove(&scope);
                    log::info!(
                        "state_watch: debounced state change for pane {pane_id} scope={}",
                        scope.as_str()
                    );
                    if sender.send(StateChangedNotice { pane_id, scope }).is_err() {
                        // Host receiver dropped — nothing more to do.
                        return;
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
    use tempfile::tempdir;

    fn poll_for_notice(
        rx: &MailboxReceiver<StateChangedNotice>,
        base_timeout: Duration,
    ) -> Option<StateChangedNotice> {
        let deadline = Instant::now() + crate::testing::load_aware_timeout(base_timeout);
        while Instant::now() < deadline {
            if let Ok(notice) = rx.try_recv() {
                return Some(notice);
            }
            thread::sleep(Duration::from_millis(20));
        }
        None
    }

    fn arm_delay() {
        // Give FSEvents a moment to arm before mutating.
        thread::sleep(crate::testing::load_aware_timeout(Duration::from_millis(
            150,
        )));
    }

    #[test]
    fn fires_on_external_write() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("todo.json");
        fs::write(&file, "{}\n").unwrap();

        let wake = Arc::new(RecordingWake::new());
        let (mut watcher, rx) = StateWatcher::new(wake.clone());
        watcher.watch(42, &[(StateScope::Global, file.clone())]);
        arm_delay();

        fs::write(&file, "{\"k\":1}\n").unwrap();

        let got = poll_for_notice(&rx, Duration::from_secs(3));
        assert_eq!(
            got,
            Some(StateChangedNotice {
                pane_id: 42,
                scope: StateScope::Global
            }),
            "external write should yield a debounced notice"
        );
        assert!(
            wake.sources().contains(&"state_watch"),
            "state change must wake the UI thread"
        );
    }

    #[test]
    fn survives_atomic_rename_replacement() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("todo.json");
        fs::write(&file, "{}\n").unwrap();

        let (mut watcher, rx) = StateWatcher::new(Arc::new(RecordingWake::new()));
        watcher.watch(7, &[(StateScope::Context, file.clone())]);
        arm_delay();

        // First replacement (new inode).
        crate::host::state_scope::atomic_write(&file, b"{\"v\":1}\n").unwrap();
        let first = poll_for_notice(&rx, Duration::from_secs(3));
        assert!(first.is_some(), "first atomic replacement must fire");
        // Drain any stragglers from the first burst before the second write.
        thread::sleep(crate::testing::load_aware_timeout(Duration::from_millis(
            400,
        )));
        while rx.try_recv().is_ok() {}

        // Second replacement — a single-file watch would be dead by now
        // because the first rename replaced the watched inode.
        crate::host::state_scope::atomic_write(&file, b"{\"v\":2}\n").unwrap();
        let second = poll_for_notice(&rx, Duration::from_secs(3));
        assert_eq!(
            second,
            Some(StateChangedNotice {
                pane_id: 7,
                scope: StateScope::Context
            }),
            "watch must survive inode replacement (parent-dir watch, not file watch)"
        );
    }

    #[test]
    fn debounces_burst_to_single_event() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("burst.json");
        fs::write(&file, "{}\n").unwrap();

        let (mut watcher, rx) = StateWatcher::new(Arc::new(RecordingWake::new()));
        watcher.watch(9, &[(StateScope::Global, file.clone())]);
        arm_delay();

        for i in 0..6 {
            fs::write(&file, format!("{{\"v\":{i}}}\n")).unwrap();
            thread::sleep(Duration::from_millis(10));
        }

        let first = poll_for_notice(&rx, Duration::from_secs(3));
        assert!(first.is_some(), "expected at least one notice");

        // With a 250ms debounce the burst should coalesce — at most one
        // extra is OK (FSEvents occasionally splits a burst).
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
            "burst of 6 writes should debounce to 1–2 notices, got {extras} extras"
        );
    }

    #[test]
    fn ignores_sibling_app_files() {
        let dir = tempdir().unwrap();
        let mine = dir.path().join("mine.json");
        let sibling = dir.path().join("other-app.json");
        fs::write(&mine, "{}\n").unwrap();
        fs::write(&sibling, "{}\n").unwrap();

        let (mut watcher, rx) = StateWatcher::new(Arc::new(RecordingWake::new()));
        watcher.watch(11, &[(StateScope::Global, mine.clone())]);
        arm_delay();
        // FSEvents can replay the pre-watch creation of mine.json once the
        // watch arms — drain that echo so only the sibling write is under test.
        thread::sleep(crate::testing::load_aware_timeout(Duration::from_millis(
            600,
        )));
        while rx.try_recv().is_ok() {}

        // Only the sibling changes — the shared dir fires FS events, but the
        // filename filter must drop them.
        fs::write(&sibling, "{\"other\":1}\n").unwrap();
        let got = poll_for_notice(&rx, Duration::from_millis(800));
        assert!(
            got.is_none(),
            "a sibling app's state file must not fire this pane's watcher, got {got:?}"
        );
    }

    #[test]
    fn unwatch_stops_delivery_and_is_idempotent() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("stop.json");
        fs::write(&file, "{}\n").unwrap();

        let (mut watcher, rx) = StateWatcher::new(Arc::new(RecordingWake::new()));
        watcher.watch(13, &[(StateScope::Global, file.clone())]);
        arm_delay();

        watcher.unwatch(13);
        // Drain any in-flight event from before the unwatch.
        thread::sleep(crate::testing::load_aware_timeout(Duration::from_millis(
            400,
        )));
        while rx.try_recv().is_ok() {}

        fs::write(&file, "{\"v\":1}\n").unwrap();
        let got = poll_for_notice(&rx, Duration::from_millis(800));
        assert!(
            got.is_none(),
            "no notice should be delivered after unwatch, got {got:?}"
        );

        // Idempotent: unwatching again (and an unknown pane) is a no-op.
        watcher.unwatch(13);
        watcher.unwatch(999);
        assert!(watcher.watched_entries(13).is_empty());
    }

    #[test]
    fn watched_entries_reports_registration() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("app.json");
        fs::write(&file, "{}\n").unwrap();

        let (mut watcher, _rx) = StateWatcher::new(Arc::new(RecordingWake::new()));
        assert!(watcher.watched_entries(21).is_empty());
        watcher.watch(21, &[(StateScope::Global, file.clone())]);
        assert_eq!(
            watcher.watched_entries(21),
            &[(StateScope::Global, file.clone())]
        );
        watcher.unwatch(21);
        assert!(watcher.watched_entries(21).is_empty());
    }
}
