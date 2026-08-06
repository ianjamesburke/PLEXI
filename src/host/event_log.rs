//! Plexi host event bus — append-only JSONL event log.
//!
//! The `EventLog` struct owns a background writer thread that receives
//! `HostEvent` values via a bounded mpsc channel and appends them as
//! newline-delimited JSON to:
//!
//! - Global:    `~/.plexi/events.jsonl` — always, for every event.
//! - Per-root:  `<context_root>/<channel>/events.jsonl` — additionally,
//!   whenever the emitting call site can resolve a `context_root` for the
//!   event (stint 0724 Phase E). This replaces the old single
//!   startup-cwd-resolved workspace file: every event used to attribute to
//!   whichever workspace was current cwd when the process started, for the
//!   life of the session, regardless of which context actually raised it.
//!   Per-record routing means a session with panes open across several
//!   contexts writes each event to its own context's file, not to one
//!   startup-chosen file. File handles are cached per root in the writer
//!   thread so repeated events from the same root never reopen the file.
//!
//! The host stamps every event with a `source` field before enqueueing;
//! apps cannot forge provenance.
//!
//! Drop-on-full policy: if the channel is at capacity (4096 events), the
//! event is silently discarded and `dropped_count` is incremented. No retry,
//! no blocking, no rotation.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
    /// An app pane was successfully spawned.
    AppSpawned {
        app_id: String,
        type_id: String,
        pane_id: u64,
        timestamp: String,
    },
    /// A WASM app runtime was closed.
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
    /// The focused pane changed. Carries the *departing* pane's metadata and
    /// how long it held focus, enabling a queryable attention timeline.
    FocusChanged {
        pane_id: u64,
        /// Name of the context (sidebar project) this pane belongs to.
        context_name: String,
        /// Description of the context.
        context_description: String,
        /// Root path assigned to the context, if any.
        context_root: Option<String>,
        /// CWD of the departing pane (terminals via proc_info; apps via workspace_root).
        cwd: Option<String>,
        /// Last OSC 2 title string the process wrote, if any.
        pty_title: Option<String>,
        /// User-assigned pane name, if any.
        pane_name: Option<String>,
        /// For App panes: the manifest_id (e.g. "gh-issues"). Null for terminals.
        app_type_id: Option<String>,
        /// Why this focus segment was banked: pane_switch, heartbeat, or shutdown.
        reason: Option<String>,
        /// Seconds this pane held focus before the switch.
        duration_secs: u64,
        timestamp: String,
    },
    /// A pane was created via a split action.
    PaneSplit {
        pane_id: u64,
        direction: String,
        timestamp: String,
    },
    /// A pane was closed.
    PaneClosed { pane_id: u64, timestamp: String },
    /// A pane was renamed.
    PaneRenamed {
        pane_id: u64,
        name: String,
        timestamp: String,
    },
    /// A new context was created.
    ContextCreated {
        context_id: u64,
        name: String,
        timestamp: String,
    },
    /// A context was renamed.
    ContextRenamed {
        context_id: u64,
        name: String,
        timestamp: String,
    },
    /// A scratch note was created and opened.
    ScratchpadOpened {
        pane_id: u64,
        path: String,
        timestamp: String,
    },
    /// The notes picker was opened.
    NotesPickerOpened {
        /// How many notes tiers were in scope (own, nested, and global).
        tier_count: usize,
        note_count: usize,
        timestamp: String,
    },
    /// A note was opened from the notes picker.
    NoteOpened { path: String, timestamp: String },
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

/// One message on the writer-thread channel: the envelope to serialize, plus
/// the per-record routing destination (`None` = global-only).
struct EmitMessage {
    envelope: EventEnvelope,
    context_root: Option<PathBuf>,
}

pub struct EventLog {
    tx: mpsc::SyncSender<EmitMessage>,
    /// Monotonic count of events dropped due to a full channel.
    pub dropped_count: Arc<AtomicU64>,
}

impl EventLog {
    /// Create a new `EventLog` and start the background writer thread.
    ///
    /// `global_path` — path to the global events.jsonl file (created if
    /// absent). Per-root workspace files are resolved and opened lazily, one
    /// per distinct `context_root` an emitted event names — see
    /// [`Self::emit_scoped`].
    pub fn new(global_path: PathBuf) -> Self {
        let (tx, rx) = mpsc::sync_channel::<EmitMessage>(4096);
        let dropped_count = Arc::new(AtomicU64::new(0));

        std::thread::Builder::new()
            .name("plexi-event-log".into())
            .spawn(move || {
                // Open (or create) the always-on global log file.
                let mut global_file = open_log_file(&global_path);
                // Per-root files, opened lazily on first use and cached by
                // root so a long session with many contexts never reopens a
                // file it already holds a handle for. `None` is cached too
                // (a root whose file failed to open once is not retried
                // every subsequent event).
                let mut root_files: HashMap<PathBuf, Option<std::fs::File>> = HashMap::new();

                while let Ok(msg) = rx.recv() {
                    match serde_json::to_string(&msg.envelope) {
                        Ok(line) => {
                            if let Some(ref mut f) = global_file {
                                append_line(f, &line);
                            }
                            if let Some(root) = msg.context_root {
                                let file = root_files.entry(root.clone()).or_insert_with(|| {
                                    let path = root
                                        .join(crate::config::workspace_channel_dir())
                                        .join("events.jsonl");
                                    open_log_file(&path)
                                });
                                if let Some(ref mut f) = file {
                                    append_line(f, &line);
                                }
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

    /// Emit a host event. Always appends to the global log; additionally
    /// appends to `context_root`'s per-context `<channel>/events.jsonl` when
    /// `context_root` is `Some` — the caller supplies whatever
    /// `ScopeOrigin`/context-root it already resolved nearby (this function
    /// performs no resolution of its own). Non-blocking — drops the event
    /// (incrementing `dropped_count`) if the channel is at capacity.
    pub fn emit_scoped(&self, event: HostEvent, context_root: Option<&std::path::Path>) {
        let source = format!("plexi/{}", env!("CARGO_CRATE_NAME"));
        let envelope = EventEnvelope { source, event };
        let msg = EmitMessage {
            envelope,
            context_root: context_root.map(PathBuf::from),
        };
        match self.tx.try_send(msg) {
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

/// Returns the current UTC timestamp as an RFC 3339 string.
pub fn now_timestamp() -> String {
    chrono::Utc::now().to_rfc3339()
}

// ── Global singleton ──────────────────────────────────────────────────────────

static GLOBAL_EVENT_LOG: OnceLock<EventLog> = OnceLock::new();

/// Initialize the global event log. Call once at app startup.
/// Subsequent calls are no-ops (OnceLock guarantees at-most-once init).
///
/// Takes only the global log path — there is no startup-resolved workspace
/// path any more. Per-root routing (stint 0724 Phase E) is decided at each
/// emit call site via [`emit_scoped`], not once at process start from
/// whatever the cwd happened to be.
pub fn init_global(global_path: PathBuf) {
    let _ = GLOBAL_EVENT_LOG.get_or_init(|| EventLog::new(global_path));
}

/// Emit an event to the global log, additionally routing it to
/// `context_root`'s per-context log when `context_root` is `Some`. No-op if
/// the log was not initialized. This is the one entry point every call site
/// uses — pass `None` when no `ScopeOrigin`/context-root is resolvable
/// nearby (global-only, same behavior as before Phase E).
pub fn emit_scoped(event: HostEvent, context_root: Option<&std::path::Path>) {
    if let Some(log) = GLOBAL_EVENT_LOG.get() {
        log.emit_scoped(event, context_root);
    }
}

// ── Typed emit helpers ────────────────────────────────────────────────────────

/// Read the last `limit` lines from a JSONL file and return them as raw
/// `serde_json::Value`s. Lines that fail to parse are silently skipped.
/// Returns an empty vec when the file doesn't exist or can't be read.
pub fn read_recent(path: &std::path::Path, limit: usize) -> Vec<serde_json::Value> {
    use std::io::BufRead;
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    let reader = std::io::BufReader::new(file);
    let mut lines: std::collections::VecDeque<serde_json::Value> =
        std::collections::VecDeque::with_capacity(limit + 1);
    for line in reader.lines().map_while(|l| l.ok()) {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
            lines.push_back(v);
            if lines.len() > limit {
                lines.pop_front();
            }
        }
    }
    lines.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Poll `read_recent` until it returns at least one line or the deadline
    /// passes. The writer thread appends asynchronously off the emit call, so
    /// tests must wait for delivery rather than assume synchronous flush.
    fn wait_for_line(path: &std::path::Path) -> Vec<serde_json::Value> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let lines = read_recent(path, 10);
            if !lines.is_empty() || std::time::Instant::now() >= deadline {
                return lines;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// Stint 0724 Phase E: per-record attribution replaces the old
    /// startup-chosen single workspace file. Two events emitted with
    /// DIFFERENT `context_root`s must land in their OWN root's
    /// `<channel>/events.jsonl` — never both in whichever root happened to be
    /// active first, and never cross-contaminating the other root's file.
    #[test]
    fn emit_scoped_routes_each_root_to_its_own_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let global_path = dir.path().join("global-events.jsonl");
        let root_a = dir.path().join("root-a");
        let root_b = dir.path().join("root-b");
        std::fs::create_dir_all(&root_a).expect("mkdir root_a");
        std::fs::create_dir_all(&root_b).expect("mkdir root_b");

        let log = EventLog::new(global_path.clone());

        log.emit_scoped(
            HostEvent::NoteOpened {
                path: "a-note.md".to_string(),
                timestamp: now_timestamp(),
            },
            Some(&root_a),
        );
        log.emit_scoped(
            HostEvent::NoteOpened {
                path: "b-note.md".to_string(),
                timestamp: now_timestamp(),
            },
            Some(&root_b),
        );

        let channel_dir = crate::config::workspace_channel_dir();
        let path_a = root_a.join(&channel_dir).join("events.jsonl");
        let path_b = root_b.join(&channel_dir).join("events.jsonl");

        let lines_a = wait_for_line(&path_a);
        let lines_b = wait_for_line(&path_b);

        assert_eq!(
            lines_a.len(),
            1,
            "root_a's file must contain exactly its own event"
        );
        assert_eq!(
            lines_a[0]["path"], "a-note.md",
            "root_a's file must contain root_a's event, not root_b's"
        );
        assert_eq!(
            lines_b.len(),
            1,
            "root_b's file must contain exactly its own event"
        );
        assert_eq!(
            lines_b[0]["path"], "b-note.md",
            "root_b's file must contain root_b's event, not root_a's"
        );

        // The always-on global file gets both, regardless of per-root routing.
        let global_lines = wait_for_line(&global_path);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut global_lines = global_lines;
        while global_lines.len() < 2 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
            global_lines = read_recent(&global_path, 10);
        }
        assert_eq!(
            global_lines.len(),
            2,
            "the global log must receive every event regardless of context_root"
        );
    }

    /// A `context_root: None` emit (no resolvable origin at the call site)
    /// must still reach the global file and must not create any per-root
    /// file at all.
    #[test]
    fn emit_scoped_with_no_context_root_is_global_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        let global_path = dir.path().join("global-events.jsonl");
        let log = EventLog::new(global_path.clone());

        log.emit_scoped(
            HostEvent::NoteOpened {
                path: "global-note.md".to_string(),
                timestamp: now_timestamp(),
            },
            None,
        );

        let global_lines = wait_for_line(&global_path);
        assert_eq!(
            global_lines.len(),
            1,
            "global-only emit must reach the global file"
        );
        assert_eq!(global_lines[0]["path"], "global-note.md");
    }
}
