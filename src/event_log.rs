//! Plexi host event bus — append-only JSONL event log.
//!
//! The `EventLog` struct owns a background writer thread that receives
//! `HostEvent` values via a bounded mpsc channel and appends them as
//! newline-delimited JSON to one or two log files:
//!
//! - Global:    `~/.plexi/events.jsonl`
//! - Workspace: `.plexi/events.jsonl` relative to the workspace root,
//!   when Plexi is running inside a `.plexi/` workspace.
//!
//! The host stamps every event with a `source` field before enqueueing;
//! apps cannot forge provenance.
//!
//! Drop-on-full policy: if the channel is at capacity (4096 events), the
//! event is silently discarded and `dropped_count` is incremented. No retry,
//! no blocking, no rotation.
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, OnceLock};

// ── HostEvent enum ────────────────────────────────────────────────────────────

/// All events the host can emit to the event log.
///
/// Variant set matches spec §6.1 exactly. Any addition must land in both.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HostEvent {
    /// A ProcessApp was successfully spawned.
    AppSpawned {
        app_id: String,
        type_id: String,
        pane_id: u64,
        timestamp: String,
    },
    /// A ProcessApp was closed.
    AppClosed {
        app_id: String,
        type_id: String,
        pane_id: u64,
        timestamp: String,
        reason: Option<String>,
    },
    /// A capability request was decided by the user (granted or denied).
    PermissionDecision {
        app_id: String,
        capability: String,
        granted: bool,
        timestamp: String,
    },
    /// The user was prompted for a secret value (key not in Keychain).
    SecretPrompted {
        app_id: String,
        key: String,
        timestamp: String,
    },
    /// A secret request was denied — capability gate, scope mismatch, or
    /// user denial at the prompt.
    SecretDenied {
        app_id: String,
        key: String,
        reason: String,
        timestamp: String,
    },
    /// A Run was created.
    RunStarted {
        run_id: String,
        app_id: String,
        timestamp: String,
    },
    /// A Run's status changed (Running, BlockedOnUser, etc.).
    RunUpdated {
        run_id: String,
        status: String,
        timestamp: String,
    },
    /// A Run terminated (Completed or Failed).
    RunCompleted {
        run_id: String,
        status: String,
        timestamp: String,
    },
    /// A notification was raised by an app.
    NotificationPosted {
        id: String,
        title: String,
        urgency: String,
        timestamp: String,
    },
    /// The user invoked a notification action.
    NotificationActionInvoked {
        id: String,
        action: String,
        timestamp: String,
    },
    /// An agent-mode LLM turn completed.
    AgentTurn {
        pane_id: Option<u64>,
        tokens_in: u32,
        tokens_out: u32,
        cost_cents: u64,
        timestamp: String,
    },
    /// A typed pipe was opened.
    PipeOpened {
        from_app: String,
        channel: String,
        mode: String,
        timestamp: String,
    },
    /// A typed pipe was closed.
    PipeClosed {
        from_app: String,
        channel: String,
        timestamp: String,
    },
}

// ── Wire envelope ─────────────────────────────────────────────────────────────

/// The on-disk envelope that wraps every event with host-stamped provenance.
#[derive(Debug, Serialize, Deserialize)]
struct EventEnvelope {
    /// Host-stamped provenance. Always set internally — apps cannot forge it.
    source: String,
    #[serde(flatten)]
    event: HostEvent,
}

// ── EventLog ──────────────────────────────────────────────────────────────────

pub struct EventLog {
    tx: mpsc::SyncSender<EventEnvelope>,
    /// Monotonic count of events dropped due to a full channel.
    pub dropped_count: Arc<AtomicU64>,
}

impl EventLog {
    /// Create a new `EventLog` and start the background writer thread.
    ///
    /// `global_path` — path to the global events.jsonl file (created if absent).
    /// `workspace_path` — optional workspace-scoped events.jsonl path.
    pub fn new(global_path: PathBuf, workspace_path: Option<PathBuf>) -> Self {
        let (tx, rx) = mpsc::sync_channel::<EventEnvelope>(4096);
        let dropped_count = Arc::new(AtomicU64::new(0));

        std::thread::Builder::new()
            .name("plexi-event-log".into())
            .spawn(move || {
                // Open (or create) the log files for appending.
                let mut global_file = open_log_file(&global_path);
                let mut workspace_file = workspace_path.as_ref().and_then(|p| open_log_file(p));

                while let Ok(envelope) = rx.recv() {
                    match serde_json::to_string(&envelope) {
                        Ok(line) => {
                            if let Some(ref mut f) = global_file {
                                append_line(f, &line);
                            }
                            if let Some(ref mut f) = workspace_file {
                                append_line(f, &line);
                            }
                        }
                        Err(e) => {
                            log::warn!("event_log: failed to serialize event: {e}");
                        }
                    }
                }
            })
            .expect("failed to spawn event-log writer thread");

        Self { tx, dropped_count }
    }

    /// Emit a host event. Non-blocking — drops the event (incrementing
    /// `dropped_count`) if the channel is at capacity.
    pub fn emit(&self, event: HostEvent) {
        let source = format!("plexi/{}", env!("CARGO_CRATE_NAME"));
        let envelope = EventEnvelope { source, event };
        match self.tx.try_send(envelope) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Full(_)) => {
                self.dropped_count.fetch_add(1, Ordering::Relaxed);
                log::debug!(
                    "event_log: channel full, dropping event (total dropped: {})",
                    self.dropped_count.load(Ordering::Relaxed)
                );
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                log::warn!("event_log: writer thread has exited");
            }
        }
    }
}

// ── File helpers ──────────────────────────────────────────────────────────────

fn open_log_file(path: &std::path::Path) -> Option<std::fs::File> {
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            log::warn!("event_log: failed to create log dir {:?}: {e}", parent);
            return None;
        }
    }
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        Ok(f) => Some(f),
        Err(e) => {
            log::warn!("event_log: failed to open {:?}: {e}", path);
            None
        }
    }
}

fn append_line(file: &mut std::fs::File, line: &str) {
    if let Err(e) = writeln!(file, "{}", line) {
        log::warn!("event_log: write failed: {e}");
    }
}

// ── Workspace detection ───────────────────────────────────────────────────────

/// Returns the path to `.plexi/events.jsonl` for the workspace containing
/// `start`, or `None` if `start` is not inside a workspace. Uses
/// [`crate::app_registry::resolve_workspace_root`] so workspace detection
/// is consistent across registry, secrets, and event logging.
pub fn find_workspace_events_path(start: &std::path::Path) -> Option<PathBuf> {
    crate::app_registry::resolve_workspace_root(start)
        .map(|root| root.join(".plexi").join("events.jsonl"))
}

/// Returns the current UTC timestamp as an RFC 3339 string.
pub fn now_timestamp() -> String {
    chrono::Utc::now().to_rfc3339()
}

// ── Global singleton ──────────────────────────────────────────────────────────

static GLOBAL_EVENT_LOG: OnceLock<EventLog> = OnceLock::new();

/// Initialize the global event log. Call once at app startup.
/// Subsequent calls are no-ops (OnceLock guarantees at-most-once init).
pub fn init_global(global_path: PathBuf, workspace_path: Option<PathBuf>) {
    let _ = GLOBAL_EVENT_LOG.get_or_init(|| EventLog::new(global_path, workspace_path));
}

/// Emit an event to the global log. No-op if the log was not initialized.
pub fn emit(event: HostEvent) {
    if let Some(log) = GLOBAL_EVENT_LOG.get() {
        log.emit(event);
    }
}

// ── Typed emit helpers ────────────────────────────────────────────────────────

/// Emit a `PipeOpened` event with a host-stamped timestamp.
pub fn emit_pipe_opened(
    from_app: impl Into<String>,
    channel: impl Into<String>,
    mode: impl Into<String>,
) {
    let from_app = from_app.into();
    let channel = channel.into();
    let mode = mode.into();
    log::debug!(
        "event_log: emitting PipeOpened from '{from_app}' channel '{channel}' mode '{mode}'"
    );
    emit(HostEvent::PipeOpened {
        from_app,
        channel,
        mode,
        timestamp: now_timestamp(),
    });
}



