//! Wall-clock readings. One place that spells
//! `SystemTime::now().duration_since(UNIX_EPOCH)`.
//!
//! **Error posture, decided once here:** a clock reading before the Unix
//! epoch is not a recoverable condition any caller can do anything about, and
//! every previous call site already collapsed it to zero (`unwrap_or(0)`,
//! `unwrap_or_default()`, or a bare `unwrap()`). These functions return `0`
//! for a pre-epoch clock rather than making thirty callers restate that.

use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since the Unix epoch.
pub fn now_secs() -> u64 {
    since_epoch().as_secs()
}

/// Milliseconds since the Unix epoch.
pub fn now_millis() -> u128 {
    since_epoch().as_millis()
}

/// Nanoseconds since the Unix epoch. Used for unique-enough temp names and
/// queue ordering, never for display.
pub fn now_nanos() -> u128 {
    since_epoch().as_nanos()
}

/// Current UTC time as an RFC 3339 string (`2026-09-10T12:34:56.789+00:00`).
/// This is the event-log timestamp format.
pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Format a wall-clock instant as second-resolution ISO-8601 UTC with a `Z`
/// suffix (`2026-09-10T12:34:56Z`). Distinct from [`now_rfc3339`]: the focus
/// journal is parsed back by exact field positions, so the sub-second part
/// and the numeric offset must stay absent.
pub fn iso_z(time: SystemTime) -> String {
    let secs = time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    chrono::DateTime::from_timestamp(secs as i64, 0)
        .unwrap_or_else(chrono::Utc::now)
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

fn since_epoch() -> std::time::Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readings_agree_across_units() {
        let secs = now_secs();
        let millis = now_millis();
        let nanos = now_nanos();
        assert!(secs > 1_700_000_000, "clock is before 2023: {secs}");
        assert!(millis / 1000 >= u128::from(secs));
        assert!(nanos / 1_000_000 >= millis);
    }

    /// Guards the exact wire format the focus journal writes and parses back.
    #[test]
    fn iso_z_is_second_resolution_with_z_suffix() {
        let t = UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        assert_eq!(iso_z(t), "2023-11-14T22:13:20Z");
    }

    #[test]
    fn now_rfc3339_round_trips() {
        let raw = now_rfc3339();
        chrono::DateTime::parse_from_rfc3339(&raw).expect("rfc3339 parses back");
    }
}
