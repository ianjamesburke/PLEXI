//! Append-only JSONL audit trail for Assistant tool calls and permission
//! decisions — `<config_dir>/audit.jsonl` in production (channel-aware via
//! `crate::config::config_dir()`), injectable for tests. Separate from
//! transcripts: deleting a conversation never deletes audit history.

use std::io::Write;
use std::path::PathBuf;

/// One audit line.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AuditEvent {
    /// RFC 3339.
    pub ts: String,
    /// Broker actor identity, e.g. `agent:assistant`.
    pub actor: String,
    /// `tool_call` | `permission_decision` | `revoke`.
    pub kind: String,
    /// Broker target id, e.g. `app.csv.write_range`.
    pub target: String,
    /// Outcome: `ok`, `error`, `allow_once`, `allow_session`, `allow_always`,
    /// `deny`, `revoked`.
    pub decision: String,
    /// Short human-readable detail (error text, input summary, grant count).
    pub summary: String,
}

impl AuditEvent {
    pub fn now(actor: &str, kind: &str, target: &str, decision: &str, summary: &str) -> Self {
        Self {
            ts: crate::host::event_log::now_timestamp(),
            actor: actor.to_string(),
            kind: kind.to_string(),
            target: target.to_string(),
            decision: decision.to_string(),
            summary: summary.to_string(),
        }
    }
}

/// Handle to one on-disk audit log.
pub struct AuditLog {
    path: PathBuf,
}

impl AuditLog {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Append one event. A failure is returned so a caller can block an
    /// action that has not started. Callers that already executed log it.
    pub fn append(&self, event: &AuditEvent) -> Result<(), String> {
        let line = match serde_json::to_string(event) {
            Ok(line) => line,
            Err(e) => {
                log::error!("assistant audit: serialize failed: {e}");
                return Err(e.to_string());
            }
        };
        if let Some(parent) = self.path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                log::error!("assistant audit: create {}: {e}", parent.display());
                return Err(e.to_string());
            }
        }
        let write = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .and_then(|mut f| writeln!(f, "{line}"));
        match write {
            Ok(()) => {
                log::info!(
                    "assistant audit: {} actor={} {} = {} ({})",
                    event.kind,
                    event.actor,
                    event.target,
                    event.decision,
                    self.path.display()
                );
                Ok(())
            }
            Err(e) => {
                log::error!("assistant audit: write {}: {e}", self.path.display());
                Err(e.to_string())
            }
        }
    }

    /// Last `n` events, oldest first. Missing file = empty; corrupt lines are
    /// logged and skipped.
    pub fn tail(&self, n: usize) -> Vec<AuditEvent> {
        let raw = match std::fs::read_to_string(&self.path) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
            Err(e) => {
                log::error!("assistant audit: read {}: {e}", self.path.display());
                return Vec::new();
            }
        };
        let mut events: Vec<AuditEvent> = Vec::new();
        for (i, line) in raw.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<AuditEvent>(line) {
                Ok(ev) => events.push(ev),
                Err(e) => log::error!(
                    "assistant audit: skipping corrupt line {} in {}: {e}",
                    i + 1,
                    self.path.display()
                ),
            }
        }
        let skip = events.len().saturating_sub(n);
        events.split_off(skip)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_and_tail_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let log = AuditLog::new(tmp.path().join("audit.jsonl"));
        assert!(log.tail(10).is_empty());

        for i in 0..5 {
            log.append(&AuditEvent::now(
                "agent:assistant",
                "tool_call",
                &format!("app.t{i}"),
                "ok",
                "",
            ))
            .unwrap();
        }
        let last3 = log.tail(3);
        assert_eq!(last3.len(), 3);
        assert_eq!(last3[0].target, "app.t2");
        assert_eq!(last3[2].target, "app.t4");
        assert_eq!(last3[0].actor, "agent:assistant");
    }

    #[test]
    fn corrupt_lines_are_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("audit.jsonl");
        let log = AuditLog::new(path.clone());
        log.append(&AuditEvent::now("agent:assistant", "tool_call", "app.x", "ok", "")).unwrap();
        let mut raw = std::fs::read_to_string(&path).unwrap();
        raw.push_str("not json\n");
        std::fs::write(&path, raw).unwrap();
        log.append(&AuditEvent::now("agent:assistant", "tool_call", "app.y", "error", "boom")).unwrap();
        let events = log.tail(10);
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].target, "app.y");
    }
}
