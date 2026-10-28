//! Watches the global and workspace-local app directories for filesystem changes
//! and emits debounced reload signals so the host can rescan the `AppRegistry`
//! without a restart. (#1712)
//!
//! Mirrors `config_watcher.rs` but watches a list of directories (not a single
//! file) with a 500 ms debounce — scaffolding creates many files in rapid
//! succession, so a longer window avoids redundant rescans.

use crate::app::ui_mailbox::{MailboxReceiver, UiMailbox, UiWake};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

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
            if let Err(e) = h.join() {
                log::warn!("app_registry_watcher: debounce thread panicked: {e:?}");
            }
        }
    }
}

/// Start watching `dirs`, filtering to those that currently exist.
///
/// Returns `None` if no directories could be watched (e.g. the global apps dir
/// has never been created). Any `Create`, `Modify`, or `Remove` event in any
/// watched directory fires a signal on the returned receiver, waking the UI
/// thread via `wake` so the rescan is not stranded on an idle host.
pub fn start(
    dirs: Vec<PathBuf>,
    wake: Arc<dyn UiWake>,
) -> Option<(AppRegistryWatcher, MailboxReceiver<()>)> {
    let existing: Vec<PathBuf> = dirs.into_iter().filter(|d| d.is_dir()).collect();
    if existing.is_empty() {
        log::info!("app_registry_watcher: no existing dirs to watch; watcher not started");
        return None;
    }

    let (reload_tx, reload_rx) = UiMailbox::channel(wake, "app_registry_watcher");
    let (raw_tx, raw_rx) = mpsc::channel::<()>();

    let mut watcher = match notify::recommended_watcher(move |res: notify::Result<Event>| match res
    {
        Ok(ev) => {
            if matches!(
                ev.kind,
                EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
            ) {
                let _ = raw_tx.send(());
            }
        }
        Err(e) => log::warn!("app_registry_watcher: watcher error: {e}"),
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
        crate::app::debounce::run(
            raw_rx,
            cancel_thread,
            Duration::from_millis(DEBOUNCE_MS),
            |_| {
                log::info!("app_registry_watcher: apps dir changed, rescanning registry");
                reload_tx.send(()).is_ok()
            },
        );
    });

    Some((
        AppRegistryWatcher {
            _watcher: watcher,
            cancel,
            thread: Some(thread),
        },
        reload_rx,
    ))
}
