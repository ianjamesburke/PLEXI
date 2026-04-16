/// Plexi notification log — MVP.
///
/// Apps emit `DrawCommand::Notification` via the existing draw command channel.
/// `ProcessApp` forwards each one into the global `NotificationLog`, which
/// appends to an in-memory Vec *and* to `~/.plexi-alpha/notifications.jsonl`
/// (append-only, one notification per line).
///
/// The log is a process-wide singleton so that any pane's subprocess can push
/// into it and the main UI (status bar, palette) can read from it without
/// threading an Arc through every construction site.
///
/// This is intentionally minimal — no delivery guarantees, no per-priority
/// styling, no dismissal persistence, no tray. See issue #74 for the full
/// attention queue vision; this is the unblock-Parallax MVP.

use crate::app_protocol::NotificationAction;
use crate::config;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

fn default_urgency() -> String {
    "low".to_string()
}

/// A notification record.
///
/// `timestamp` is UTC; `read` defaults to false and flips when the user clicks
/// the notification in the palette. `read` is NOT persisted — on restart,
/// reloaded notifications come back as read since the user has presumably
/// already seen them in a previous session.
///
/// v2.0: `action` replaces the old `action_type`/`action_payload` string pair.
/// Both fields are kept for backwards-compat deserialization of JSONL written
/// by older builds; new code should read `action` first.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Notification {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    pub source_app: String,
    #[serde(default)]
    pub read: bool,
    /// Urgency level: "low" | "medium" | "high". Replaces the old `priority: u8`.
    #[serde(default = "default_urgency")]
    pub urgency: String,
    /// Unix timestamp (seconds). If set and already past, the notification is
    /// dropped on ingestion.
    #[serde(default)]
    pub expires_at: Option<i64>,
    /// Unix timestamp (seconds). Defer rendering until this time has passed.
    #[serde(default)]
    pub visible_after: Option<i64>,
    /// Run this notification is associated with. When set, the palette renders
    /// this notification as a run card (head_task + elapsed + status pill).
    #[serde(default)]
    pub run_id: Option<String>,
    /// Structured action (v2.0+). Supersedes `action_type`/`action_payload`.
    #[serde(default)]
    pub action: Option<NotificationAction>,
    /// Legacy action type string — kept for JSONL round-trip compat.
    /// Prefer `action` for new code.
    #[serde(default)]
    pub action_type: Option<String>,
    /// Legacy action payload — kept for JSONL round-trip compat.
    #[serde(default)]
    pub action_payload: Option<serde_json::Value>,
    /// Who emitted this notification: "app:<id>", "claude-code", "socket", etc.
    #[serde(default)]
    pub source: Option<String>,
}

/// In-memory + on-disk notification log.
pub struct NotificationLog {
    notifications: Vec<Notification>,
    log_path: PathBuf,
}

impl NotificationLog {
    /// Construct a log, reading any existing `notifications.jsonl` from disk.
    /// Notifications loaded from disk come back with `read = true` so old
    /// events from previous sessions don't spam the unread counter.
    pub fn new() -> Self {
        let log_path = config::config_dir().join("notifications.jsonl");
        let mut notifications = Vec::new();
        if let Ok(contents) = std::fs::read_to_string(&log_path) {
            for line in contents.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                match serde_json::from_str::<Notification>(line) {
                    Ok(mut n) => {
                        n.read = true;
                        notifications.push(n);
                    }
                    Err(e) => {
                        log::warn!("NotificationLog: skipping malformed line: {e}");
                    }
                }
            }
        }
        Self {
            notifications,
            log_path,
        }
    }

    /// Append a new notification to memory and to the JSONL log on disk.
    /// Drops the notification silently if `expires_at` is set and already past.
    pub fn append(&mut self, n: Notification) {
        if let Some(exp) = n.expires_at {
            if exp < chrono::Utc::now().timestamp() {
                log::debug!(
                    "NotificationLog: dropping expired notification \"{}\" (expires_at={exp})",
                    n.title
                );
                return;
            }
        }
        if let Err(e) = self.append_to_disk(&n) {
            log::warn!("NotificationLog: failed to write notifications.jsonl: {e}");
        }
        log::info!(
            "notification from {}: [{}] {}",
            n.source_app, n.urgency, n.title
        );
        self.notifications.push(n);
    }

    /// Append a notification received from the Unix socket listener.
    pub fn push_from_socket(&mut self, n: Notification) {
        self.append(n);
    }

    /// Number of unread notifications currently in the log.
    pub fn unread_count(&self) -> usize {
        self.notifications.iter().filter(|n| !n.read).count()
    }

    /// All notifications, oldest first. Callers that want newest-first should
    /// iterate in reverse.
    pub fn list(&self) -> &[Notification] {
        &self.notifications
    }

    /// Mark the notification at `idx` as read. No-op if out of bounds.
    pub fn mark_read(&mut self, idx: usize) {
        if let Some(n) = self.notifications.get_mut(idx) {
            n.read = true;
        }
    }

    /// Mark every notification as read.
    pub fn mark_all_read(&mut self) {
        for n in &mut self.notifications {
            n.read = true;
        }
    }

    fn append_to_disk(&self, n: &Notification) -> Result<(), std::io::Error> {
        if let Some(parent) = self.log_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)?;
        let mut line = serde_json::to_string(n)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        line.push('\n');
        file.write_all(line.as_bytes())
    }
}

// ── Process-wide singleton ───────────────────────────────────────────────────
//
// Same shape as `cost_tracker` in spirit, but shared across all ProcessApps
// and the main UI rather than per-app.

static GLOBAL: OnceLock<Mutex<NotificationLog>> = OnceLock::new();

/// Access the global notification log, lazily initializing it on first use.
pub fn global() -> &'static Mutex<NotificationLog> {
    GLOBAL.get_or_init(|| Mutex::new(NotificationLog::new()))
}

/// Record a notification into the global log. Convenience wrapper that
/// acquires the mutex and builds the `Notification` struct.
pub fn record(
    title: String,
    body: Option<String>,
    source_app: String,
    urgency: String,
    expires_at: Option<i64>,
    visible_after: Option<i64>,
    run_id: Option<String>,
    action: Option<NotificationAction>,
    source: Option<String>,
) {
    let n = Notification {
        timestamp: chrono::Utc::now(),
        title,
        body,
        source_app,
        read: false,
        urgency,
        expires_at,
        visible_after,
        run_id,
        action,
        action_type: None,
        action_payload: None,
        source,
    };
    match global().lock() {
        Ok(mut log) => log.append(n),
        Err(e) => log::error!("NotificationLog: global mutex poisoned: {e}"),
    }
}

/// Current unread count from the global log. Returns 0 if the mutex is
/// poisoned — the UI should never panic because of a poisoned notification
/// mutex.
pub fn unread_count() -> usize {
    global().lock().map(|l| l.unread_count()).unwrap_or(0)
}
