//! Live command view over agents API records.
//!
//! The pane and `agents-dashboard --follow` read [`crate::agent::runs::projection`].
//! A socket reader reaches it only through [`read`], which commits against the
//! caller's [`ReadDashboard`] permit (stint 1716).
//! This module does not keep a board. `enqueue`, `pause`, and `block` are not
//! commands here, and `resolve` / `allow` do not grant.

use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

use serde_json::{json, Value};

use crate::broker::permit::{Permit, ReadDashboard};

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
    let payload = crate::agent::runs::projection(workspace);
    let timeline = crate::host::plugin_timeline::global();
    let Ok(mut timeline) = timeline.lock() else {
        log::warn!("command_view: timeline lock failed");
        return;
    };
    if let Err(error) = timeline.record_command_view(revision, summary, payload) {
        log::warn!("command_view: event record failed: {error}");
    }
}

/// The projection of `workspace` for the permit's holder: staged first, then
/// handed over only while the permit still covers the workspace, so a revoke,
/// a scope move or a closed pane stops the next read.
pub(crate) fn read(permit: &Permit<ReadDashboard>, workspace: &Path) -> Result<Value, String> {
    let body = crate::agent::runs::projection(workspace);
    permit.recheck(workspace)?;
    Ok(body)
}

/// Stream `command.view` until the client disconnects or `permit` stops
/// covering the workspace, which ends the stream with an error line.
pub(crate) fn serve_follow(
    mut socket: crate::platform::ipc::IpcStream,
    workspace: String,
    permit: &Permit<ReadDashboard>,
) {
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
    let hello = json!({"type":"subscribed","event":STREAM,"plugin_id":PUBLISHER});
    if writeln_line(&mut socket, &hello).is_err() {
        return;
    }
    let mut seen = u64::MAX;
    while !stop.load(Ordering::Acquire) {
        // An idle stream notices a revoke or a moved scope at each wake. The
        // wake lock is not held: the broker's admission lock is taken here.
        if let Err(error) = permit.recheck(&workspace) {
            return end_follow(&mut socket, &workspace, &error);
        }
        let (lock, cond) = wake();
        let guard = lock.lock().unwrap_or_else(|err| err.into_inner());
        let revision = *guard;
        if revision != seen {
            drop(guard);
            seen = revision;
            let mut event = match read(permit, &workspace) {
                Ok(event) => event,
                Err(error) => return end_follow(&mut socket, &workspace, &error),
            };
            if let Some(obj) = event.as_object_mut() {
                obj.insert("type".to_string(), json!("event"));
                obj.insert("event".to_string(), json!(STREAM));
                obj.insert("plugin_id".to_string(), json!(PUBLISHER));
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

fn end_follow(socket: &mut crate::platform::ipc::IpcStream, workspace: &Path, error: &str) {
    log::info!(
        "command_view: follow of {} ended: {error}",
        workspace.display()
    );
    let reply = json!({
        "type": "error", "ok": false, "error_code": "permission_denied",
        "error": error, "message": error,
    });
    // The client may already be gone.
    let _ = writeln_line(socket, &reply);
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

    /// Stint 1716: a followed stream ends, with an error line and no further
    /// projection, once the follower's pane scope moves. Waking it with a
    /// publish is the same path a real mutation takes.
    #[cfg(unix)]
    #[test]
    fn a_follow_ends_when_its_permit_stops_covering_the_workspace() {
        use crate::broker::pane_authority::{Ceiling, LivePaneAuthority, PaneAuthority};
        use std::io::BufRead as _;
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".plexi")).unwrap();
        let live = LivePaneAuthority::admitted(PaneAuthority::for_test(
            7,
            Ceiling::Baseline,
            1,
            dir.path().to_path_buf(),
        ));
        let permit = Permit::<ReadDashboard>::contained(
            crate::host::plugin_timeline::AdmittedScope::of(&live),
            dir.path(),
            crate::broker::gate::PermissionMonitor::ephemeral(),
        )
        .unwrap();
        let (server, client) = std::os::unix::net::UnixStream::pair().unwrap();
        let workspace = dir.path().to_string_lossy().to_string();
        let worker = std::thread::spawn(move || serve_follow(server, workspace, &permit));
        let mut lines = std::io::BufReader::new(client);
        let mut next = || {
            let mut line = String::new();
            lines.read_line(&mut line).unwrap();
            serde_json::from_str::<Value>(&line).unwrap_or(Value::Null)
        };
        assert_eq!(next()["type"], "subscribed");
        assert_eq!(next()["type"], "event");
        publish(dir.path(), "still covered");
        assert_eq!(next()["type"], "event");
        live.publish_scope(dir.path().join("elsewhere"));
        publish(dir.path(), "after the scope moved");
        let ended = next();
        assert_eq!(ended["type"], "error", "{ended}");
        assert_eq!(ended["error_code"], "permission_denied", "{ended}");
        worker.join().unwrap();
        assert_eq!(next(), Value::Null, "nothing follows the refusal");
    }

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
