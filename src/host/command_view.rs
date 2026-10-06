//! Live command view over agents API records.
//!
//! The pane and `command-view --follow` read [`crate::agent::leads::projection`].
//! This module does not keep a board. `enqueue`, `pause`, and `block` are not
//! commands here, and `resolve` / `allow` do not grant.

use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

use serde_json::{json, Value};

pub const PUBLISHER: &str = "plexi.host.command";
pub const STREAM: &str = "command.view";

fn wake() -> &'static (Mutex<u64>, Condvar) {
    static WAKE: OnceLock<(Mutex<u64>, Condvar)> = OnceLock::new();
    WAKE.get_or_init(|| (Mutex::new(0), Condvar::new()))
}

/// Record one projection after a real mutation. List and projection reads do
/// not call this.
pub fn publish(workspace: &Path, summary: &str) {
    let revision = {
        let (lock, cond) = wake();
        let mut guard = lock.lock().unwrap_or_else(|err| err.into_inner());
        *guard = guard.saturating_add(1);
        let revision = *guard;
        cond.notify_all();
        revision
    };
    log::info!("command_view: revision={revision} {summary}");
    let payload = crate::agent::leads::projection(workspace);
    let timeline = crate::host::app_timeline::global();
    let Ok(mut timeline) = timeline.lock() else {
        log::warn!("command_view: timeline lock failed");
        return;
    };
    if let Err(error) = timeline.record_command_view(revision, summary, payload) {
        log::warn!("command_view: event record failed: {error}");
    }
}

/// Stream `command.view` until the client disconnects.
pub fn serve_follow(mut socket: crate::platform::ipc::IpcStream, workspace: String) {
    if workspace.is_empty() {
        log::warn!("command_view: follow missing workspace");
        return;
    }
    let workspace = std::path::PathBuf::from(workspace);
    log::info!(
        "command_view: follow open workspace={}",
        workspace.display()
    );
    let stop = Arc::new(AtomicBool::new(false));
    let stop_reader = Arc::clone(&stop);
    let mut reader = match socket.try_clone() {
        Ok(clone) => clone,
        Err(error) => {
            log::warn!("command_view: follow clone failed: {error}");
            return;
        }
    };
    std::thread::spawn(move || {
        let mut buf = [0u8; 64];
        loop {
            match std::io::Read::read(&mut reader, &mut buf) {
                Ok(0) | Err(_) => {
                    stop_reader.store(true, Ordering::Release);
                    wake().1.notify_all();
                    break;
                }
                Ok(_) => {}
            }
        }
    });
    let hello = json!({"type":"subscribed","event":STREAM,"app_id":PUBLISHER});
    if writeln_line(&mut socket, &hello).is_err() {
        return;
    }
    let mut seen = u64::MAX;
    while !stop.load(Ordering::Acquire) {
        let (lock, cond) = wake();
        let guard = lock.lock().unwrap_or_else(|err| err.into_inner());
        let revision = *guard;
        if revision != seen {
            drop(guard);
            seen = revision;
            let mut event = crate::agent::leads::projection(&workspace);
            if let Some(obj) = event.as_object_mut() {
                obj.insert("type".to_string(), json!("event"));
                obj.insert("event".to_string(), json!(STREAM));
                obj.insert("app_id".to_string(), json!(PUBLISHER));
                obj.insert("revision".to_string(), json!(revision));
            }
            if writeln_line(&mut socket, &event).is_err() {
                return;
            }
            continue;
        }
        if stop.load(Ordering::Acquire) {
            break;
        }
        let (_guard, _result) = cond
            .wait_timeout(guard, Duration::from_secs(15))
            .unwrap_or_else(|err| err.into_inner());
    }
    log::info!("command_view: follow closed");
}

fn writeln_line(socket: &mut crate::platform::ipc::IpcStream, value: &Value) -> Result<(), ()> {
    writeln!(socket, "{value}")
        .and_then(|()| socket.flush())
        .map_err(|error| {
            log::info!("command_view: follow write failed: {error}");
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_advances_the_revision() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".plexi")).unwrap();
        let before = {
            let (lock, _) = wake();
            *lock.lock().unwrap_or_else(|err| err.into_inner())
        };
        publish(dir.path(), "head created");
        let after = {
            let (lock, _) = wake();
            *lock.lock().unwrap_or_else(|err| err.into_inner())
        };
        assert!(after > before, "{before} -> {after}");
    }
}
