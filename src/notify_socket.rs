//! Unix socket listener for external notification ingestion.
//!
//! External processes (Claude Code hooks, scripts) write a JSON `Notification`
//! payload to `~/.plexi-alpha/notify.sock` (or `~/.plexi/notify.sock` on
//! stable) and disconnect. Plexi appends each received notification to the
//! global `NotificationLog`.
//!
//! The socket is line-oriented: each connection may send one or more
//! newline-terminated JSON objects. Empty lines are ignored.

use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixListener;
use std::sync::mpsc;

use crate::notification_log::Notification;

/// Spawn the socket listener thread. Returns immediately; the listener runs in
/// the background. Notifications are sent to `tx` for the main thread to drain
/// via `PlexiApp::notify_rx` in the egui update loop.
pub fn start(tx: mpsc::Sender<Notification>) {
    let sock_path = crate::config::config_dir().join("notify.sock");

    // Remove a stale socket from a previous run so bind succeeds.
    let _ = std::fs::remove_file(&sock_path);

    std::thread::spawn(move || {
        let listener = match UnixListener::bind(&sock_path) {
            Ok(l) => l,
            Err(e) => {
                log::error!(
                    "notify_socket: failed to bind {}: {e}",
                    sock_path.display()
                );
                return;
            }
        };
        log::info!("notify_socket: listening at {}", sock_path.display());

        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let tx = tx.clone();
                    std::thread::spawn(move || {
                        let reader = BufReader::new(stream);
                        for line in reader.lines() {
                            match line {
                                Ok(l) if !l.trim().is_empty() => {
                                    match serde_json::from_str::<Notification>(&l) {
                                        Ok(mut n) => {
                                            // Default source to "socket" if the
                                            // sender didn't set one.
                                            if n.source.is_none() {
                                                n.source = Some("socket".to_string());
                                            }
                                            if let Err(e) = tx.send(n) {
                                                log::warn!(
                                                    "notify_socket: channel closed: {e}"
                                                );
                                            }
                                        }
                                        Err(e) => {
                                            log::warn!(
                                                "notify_socket: malformed payload: {e}"
                                            );
                                        }
                                    }
                                }
                                Ok(_) => {} // blank line
                                Err(e) => {
                                    log::warn!("notify_socket: read error: {e}");
                                    break;
                                }
                            }
                        }
                    });
                }
                Err(e) => log::warn!("notify_socket: accept error: {e}"),
            }
        }
    });
}
