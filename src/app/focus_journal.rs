//! Focus journal — single-file crash-recovery checkpoint for the current focus
//! segment. Written on each heartbeat and on clean pane transitions; deleted on
//! clean shutdown. If the file exists at startup it means Plexi crashed while
//! a pane was focused; the host reads it, emits a `crash_recovery` event to
//! `events.jsonl`, and deletes the file before normal operation begins.

use std::path::Path;

/// Snapshot of the currently focused pane written to the journal file.
/// Serialised as a single JSON line; the file always contains exactly one line.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct FocusJournalEntry {
    pub pane_id: u64,
    pub context_name: String,
    pub context_description: String,
    pub context_root: Option<String>,
    pub cwd: Option<String>,
    pub pty_title: Option<String>,
    pub pane_name: Option<String>,
    pub app_type_id: Option<String>,
    /// ISO-8601 timestamp when this focus segment started.
    pub started_at: String,
    /// ISO-8601 timestamp of the last checkpoint write.
    pub last_checkpoint_at: String,
}

/// Write (overwrite) the journal file with the current checkpoint.
/// Called on every heartbeat tick and on clean pane transitions.
pub(crate) fn write_checkpoint(path: &Path, entry: &FocusJournalEntry) {
    let json = match serde_json::to_string(entry) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("focus_journal: failed to serialize checkpoint: {e}");
            return;
        }
    };
    let tmp = path.with_extension("jsonl.tmp");
    if let Err(e) = std::fs::write(&tmp, format!("{json}\n")) {
        log::warn!("focus_journal: failed to write tmp checkpoint {:?}: {e}", tmp);
        return;
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        log::warn!("focus_journal: failed to rename checkpoint {:?}: {e}", path);
    }
}

/// Delete the journal file on clean close (pane_switch or shutdown).
pub(crate) fn clear_journal(path: &Path) {
    if path.exists() {
        if let Err(e) = std::fs::remove_file(path) {
            log::warn!("focus_journal: failed to delete journal {:?}: {e}", path);
        }
    }
}

/// On startup: if the journal file exists, read it, emit a `crash_recovery`
/// event to `events.jsonl`, and delete the file.
///
/// The `crash_recovery` event uses the same `FocusChanged` schema so Stats can
/// account for the time without special-casing.
pub(crate) fn recover_from_focus_journal(path: &Path) {
    let content = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(e) => {
            log::warn!("focus_journal: failed to read journal {:?}: {e}", path);
            return;
        }
    };

    let line = content.trim();
    if line.is_empty() {
        log::warn!("focus_journal: empty journal file, deleting");
        clear_journal(path);
        return;
    }

    let entry: FocusJournalEntry = match serde_json::from_str(line) {
        Ok(e) => e,
        Err(e) => {
            log::warn!("focus_journal: failed to parse journal entry: {e} — deleting");
            clear_journal(path);
            return;
        }
    };

    // Compute duration from started_at to now.
    let duration_secs = parse_iso_to_unix(&entry.started_at)
        .map(|started_unix| {
            let now_unix = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            now_unix.saturating_sub(started_unix)
        })
        .unwrap_or(0);

    log::info!(
        "focus_journal: crash_recovery — pane_id={} duration_secs={} context={:?}",
        entry.pane_id,
        duration_secs,
        entry.context_name
    );

    crate::host::event_log::emit(crate::host::event_log::HostEvent::FocusChanged {
        pane_id: entry.pane_id,
        context_name: entry.context_name,
        context_description: entry.context_description,
        context_root: entry.context_root,
        cwd: entry.cwd,
        pty_title: entry.pty_title,
        pane_name: entry.pane_name,
        app_type_id: entry.app_type_id,
        reason: Some("crash_recovery".to_string()),
        duration_secs,
        timestamp: crate::host::event_log::now_timestamp(),
    });

    clear_journal(path);
}

/// Parse an ISO-8601 timestamp string (RFC 3339 / UTC Z suffix) to a Unix
/// seconds value. Returns `None` on parse failure.
fn parse_iso_to_unix(ts: &str) -> Option<u64> {
    // Expected format: "2024-01-15T12:34:56.789Z"
    // We use a manual approach to avoid adding a chrono dependency just for
    // this one conversion.
    use std::time::{Duration, SystemTime};

    // Try parsing with the `time` crate's OffsetDateTime if available;
    // fall back to a simple manual parse.
    //
    // We use SystemTime::UNIX_EPOCH arithmetic directly from the parts so
    // there is no external dependency beyond what's already in std.
    let ts = ts.trim_end_matches('Z');
    let (date_part, time_part) = ts.split_once('T')?;
    let mut date_parts = date_part.split('-');
    let year: u64 = date_parts.next()?.parse().ok()?;
    let month: u64 = date_parts.next()?.parse().ok()?;
    let day: u64 = date_parts.next()?.parse().ok()?;

    let time_no_sub = time_part.split('.').next().unwrap_or(time_part);
    let mut time_parts = time_no_sub.split(':');
    let hour: u64 = time_parts.next()?.parse().ok()?;
    let minute: u64 = time_parts.next()?.parse().ok()?;
    let second: u64 = time_parts.next()?.parse().ok()?;

    // Days since Unix epoch (1970-01-01) using the Gregorian calendar formula.
    let days = days_since_epoch(year, month, day)?;
    let unix = days * 86400 + hour * 3600 + minute * 60 + second;

    // Sanity check: must be a plausible Plexi session timestamp (after 2020).
    let min_plausible = 1_577_836_800u64; // 2020-01-01T00:00:00Z
    if unix < min_plausible {
        return None;
    }

    // Cross-check via SystemTime to catch overflow.
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(unix))
        .map(|_| unix)
}

/// Number of days from the Unix epoch (1970-01-01) to the given Gregorian date.
/// Returns None if the date is before 1970 or implausibly large.
fn days_since_epoch(year: u64, month: u64, day: u64) -> Option<u64> {
    if year < 1970 || month < 1 || month > 12 || day < 1 || day > 31 {
        return None;
    }
    // Days in each month (non-leap year).
    let days_in_month = [31u64, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let is_leap = |y: u64| (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;

    let mut total: u64 = 0;
    for y in 1970..year {
        total += if is_leap(y) { 366 } else { 365 };
    }
    for m in 1..month {
        let idx = (m - 1) as usize;
        total += days_in_month[idx];
        if m == 2 && is_leap(year) {
            total += 1;
        }
    }
    total += day - 1;
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn make_entry(started_at: &str) -> FocusJournalEntry {
        FocusJournalEntry {
            pane_id: 42,
            context_name: "my-project".to_string(),
            context_description: "".to_string(),
            context_root: Some("/Users/test/project".to_string()),
            cwd: Some("/Users/test/project".to_string()),
            pty_title: Some("nvim".to_string()),
            pane_name: None,
            app_type_id: None,
            started_at: started_at.to_string(),
            last_checkpoint_at: started_at.to_string(),
        }
    }

    #[test]
    fn write_then_read_roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("focus-journal.jsonl");

        let entry = make_entry("2024-06-15T10:00:00Z");
        write_checkpoint(&path, &entry);

        assert!(path.exists());
        let content = fs::read_to_string(&path).unwrap();
        let parsed: FocusJournalEntry = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(parsed.pane_id, 42);
        assert_eq!(parsed.context_name, "my-project");
    }

    #[test]
    fn clear_deletes_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("focus-journal.jsonl");

        let entry = make_entry("2024-06-15T10:00:00Z");
        write_checkpoint(&path, &entry);
        assert!(path.exists());

        clear_journal(&path);
        assert!(!path.exists());
    }

    #[test]
    fn clear_noop_when_missing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nonexistent.jsonl");
        // Must not panic.
        clear_journal(&path);
    }

    #[test]
    fn parse_iso_to_unix_known_value() {
        // 2024-01-15T12:00:00Z → known Unix value
        let unix = parse_iso_to_unix("2024-01-15T12:00:00Z").unwrap();
        // Verified: 2024-01-15 12:00:00 UTC = 1705320000
        assert_eq!(unix, 1_705_320_000);
    }

    #[test]
    fn parse_iso_subsecond() {
        let unix = parse_iso_to_unix("2024-01-15T12:00:00.123Z").unwrap();
        assert_eq!(unix, 1_705_320_000);
    }

    #[test]
    fn parse_iso_rejects_pre_2020() {
        assert!(parse_iso_to_unix("1969-12-31T23:59:59Z").is_none());
    }

    #[test]
    fn recover_from_missing_journal_is_noop() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("focus-journal.jsonl");
        // Just verifies no panic/error logged; we can't assert event emission
        // without wiring up the global event log in tests.
        recover_from_focus_journal(&path);
        assert!(!path.exists());
    }

    #[test]
    fn recover_deletes_journal_after_reading() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("focus-journal.jsonl");

        let entry = make_entry("2024-06-15T10:00:00Z");
        write_checkpoint(&path, &entry);
        assert!(path.exists());

        // We can't assert the event was emitted (event_log global not initialised in
        // unit tests), but we can verify the file is consumed.
        recover_from_focus_journal(&path);
        assert!(!path.exists());
    }

    #[test]
    fn recover_handles_corrupt_journal() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("focus-journal.jsonl");
        fs::write(&path, "NOT_VALID_JSON\n").unwrap();

        recover_from_focus_journal(&path);
        assert!(!path.exists());
    }
}
