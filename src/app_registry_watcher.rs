//! Watches the global and workspace-local app directories for filesystem changes
//! and emits debounced reload signals so the host can rescan the `AppRegistry`
//! without a restart. (#1712)
//!
//! Mirrors `config_watcher.rs` but watches a list of directories (not a single
//! file) with a 500 ms debounce — scaffolding creates many files in rapid
//! succession, so a longer window avoids redundant rescans.

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const DEBOUNCE_MS: u64 = 500;

pub struct AppRegistryWatcher {
    _watcher: RecommendedWatcher,
    cancel: Arc<Mutex<bool>>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for AppRegistryWatcher {
    fn drop(&mut self) {
        if let Ok(mut c) = self.cancel.lock() {
            *c = true;
        }
        if let Some(h) = self.thread.take() {
            let _ = h.join();
        }
    }
}

/// Start watching `dirs`, filtering to those that currently exist.
///
/// Returns `None` if no directories could be watched (e.g. the global apps dir
/// has never been created). Any `Create`, `Modify`, or `Remove` event in any
/// watched directory fires a signal on the returned `Receiver<()>`.
pub fn start(dirs: Vec<PathBuf>) -> Option<(AppRegistryWatcher, Receiver<()>)> {
    let existing: Vec<PathBuf> = dirs.into_iter().filter(|d| d.is_dir()).collect();
    if existing.is_empty() {
        log::info!("app_registry_watcher: no existing dirs to watch; watcher not started");
        return None;
    }

    let (reload_tx, reload_rx) = mpsc::channel::<()>();
    let (raw_tx, raw_rx) = mpsc::channel::<()>();

    let mut watcher = match notify::recommended_watcher(move |res: notify::Result<Event>| {
        match res {
            Ok(ev) => {
                if matches!(
                    ev.kind,
                    EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
                ) {
                    let _ = raw_tx.send(());
                }
            }
            Err(e) => log::warn!("app_registry_watcher: watcher error: {e}"),
        }
    }) {
        Ok(w) => w,
        Err(e) => {
            log::warn!("app_registry_watcher: failed to create watcher: {e}");
            return None;
        }
    };

    for dir in &existing {
        if let Err(e) = watcher.watch(dir, RecursiveMode::NonRecursive) {
            log::warn!("app_registry_watcher: failed to watch {dir:?}: {e}");
        } else {
            log::info!("app_registry_watcher: watching {dir:?}");
        }
    }

    let cancel = Arc::new(Mutex::new(false));
    let cancel_thread = Arc::clone(&cancel);
    let thread = thread::spawn(move || {
        debounce_loop(raw_rx, reload_tx, cancel_thread);
    });

    Some((AppRegistryWatcher { _watcher: watcher, cancel, thread: Some(thread) }, reload_rx))
}

fn debounce_loop(rx: Receiver<()>, sender: Sender<()>, cancel: Arc<Mutex<bool>>) {
    let mut last_event: Option<Instant> = None;
    let debounce = Duration::from_millis(DEBOUNCE_MS);
    let poll = Duration::from_millis(50);
    loop {
        if cancel.lock().map(|g| *g).unwrap_or(true) {
            return;
        }
        match rx.try_recv() {
            Ok(()) => {
                last_event = Some(Instant::now());
            }
            Err(TryRecvError::Disconnected) => return,
            Err(TryRecvError::Empty) => {
                if let Some(t) = last_event {
                    if t.elapsed() >= debounce {
                        log::info!("app_registry_watcher: apps dir changed, rescanning registry");
                        if sender.send(()).is_err() {
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
