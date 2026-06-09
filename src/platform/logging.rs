//! Centralized file logger for Plexi.
//!
//! Writes to `~/.plexi-alpha/plexi.log` (or the appropriate config dir) and
//! also mirrors output to stderr so `cargo run` / CLI invocations stay usable.
//!
//! Date-based rotation on startup: if `plexi.log` exists and is non-empty and
//! was last written on a previous calendar day, it is renamed to
//! `plexi-YYYY-MM-DD.log` (the modification date). Same-day restarts continue
//! appending to `plexi.log`. Files older than `retention_days` (default 30)
//! are pruned automatically on each startup.
//!
//! Third-party crates (egui, wgpu, winit, etc.) are clamped to `warn` to avoid
//! noise; everything under `plexi::` logs at the caller-supplied level.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How often the watchdog samples the frame counter. Short enough to catch brief freezes.
const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);
/// UI thread is considered frozen after this many seconds without a frame tick.
/// Lowered from 5s — real stalls of 3–4s (enough for the macOS spinning ball)
/// were silently missed by the old threshold.
const FREEZE_THRESHOLD_SECS: u64 = 2;

/// Return the path of the current log file.
pub fn log_path() -> PathBuf {
    crate::config::config_dir().join("plexi.log")
}

/// Rotate `log_path` to a dated archive if it was last modified on a prior day,
/// then prune any dated files older than `cutoff` (`today - retention_days`).
/// Returns informational messages to be emitted after the logger initialises.
fn rotate_and_prune(
    log_path: &Path,
    config_dir: &Path,
    today: chrono::NaiveDate,
    retention_days: u32,
) -> Vec<String> {
    let mut messages = Vec::new();

    // Rotation: rename plexi.log → plexi-PREV-DATE.log if modified on a prior day.
    if let Ok(meta) = std::fs::metadata(log_path) {
        if meta.len() > 0 {
            let prev_date = meta
                .modified()
                .ok()
                .map(|mtime| chrono::DateTime::<chrono::Local>::from(mtime).date_naive());

            if let Some(prev_date) = prev_date {
                if prev_date < today {
                    let dated =
                        config_dir.join(format!("plexi-{}.log", prev_date.format("%Y-%m-%d")));
                    if !dated.exists() {
                        if let Err(e) = std::fs::rename(log_path, &dated) {
                            eprintln!("[plexi::logging] could not rotate log to {dated:?}: {e}");
                        } else {
                            messages.push(format!(
                                "rotated log → plexi-{}.log",
                                prev_date.format("%Y-%m-%d")
                            ));
                        }
                    } else {
                        // Dated file already exists — append plexi.log content to it.
                        match (
                            std::fs::read(log_path),
                            std::fs::OpenOptions::new().append(true).open(&dated),
                        ) {
                            (Ok(content), Ok(mut dst)) => {
                                use std::io::Write;
                                if dst.write_all(&content).is_ok() {
                                    let _ = std::fs::remove_file(log_path);
                                    messages.push(format!(
                                        "appended and rotated log → plexi-{}.log",
                                        prev_date.format("%Y-%m-%d")
                                    ));
                                }
                            }
                            _ => eprintln!(
                                "[plexi::logging] could not append to existing dated log {dated:?}"
                            ),
                        }
                    }
                }
            }
        }
    }

    // Pruning: delete dated files whose name-date is before the cutoff.
    let cutoff = today - chrono::Duration::days(retention_days as i64);
    if let Ok(entries) = std::fs::read_dir(config_dir) {
        let mut pruned = 0u32;
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if let Some(date_str) = name_str
                .strip_prefix("plexi-")
                .and_then(|s| s.strip_suffix(".log"))
            {
                if let Ok(date) = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                    if date < cutoff {
                        if let Err(e) = std::fs::remove_file(entry.path()) {
                            eprintln!("[plexi::logging] could not prune {name_str}: {e}");
                        } else {
                            pruned += 1;
                        }
                    }
                }
            }
        }
        if pruned > 0 {
            messages.push(format!(
                "pruned {pruned} log file(s) older than {retention_days} days"
            ));
        }
    }

    messages
}

/// Initialise the logger. Must be called before any `log::` macro is used.
/// If the log file cannot be opened, falls back to stderr-only and logs a warning.
/// When `cli_mode` is true, INFO/DEBUG logs are suppressed on stderr (file-only)
/// so CLI callers (especially agents) don't see noisy socket-level trace lines.
pub fn init(level: log::LevelFilter, retention_days: u32, cli_mode: bool) {
    let log_file_path = log_path();
    let config_dir = crate::config::config_dir();

    // Ensure the config directory exists.
    if let Some(parent) = log_file_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("[plexi::logging] could not create config dir {parent:?}: {e}");
        }
    }

    let today = chrono::Local::now().date_naive();
    let deferred = rotate_and_prune(&log_file_path, &config_dir, today, retention_days);

    // Build format: [YYYY-MM-DD HH:MM:SS] [LEVEL] [target] message
    let formatter =
        |out: fern::FormatCallback, message: &std::fmt::Arguments, record: &log::Record| {
            let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
            out.finish(format_args!(
                "[{now}] [{level}] [{target}] {message}",
                now = now,
                level = record.level(),
                target = record.target(),
                message = message,
            ))
        };

    // Try to open the log file; if it fails, use stderr only.
    let file_result = fern::log_file(&log_file_path);

    let dispatch = fern::Dispatch::new()
        .format(formatter)
        // plexi::*, plexi_v3::*, plexi_alpha::* (renamed crate for alpha
        // builds), and app::<id> targets emitted by the SDK log bridge
        // all follow the user-configured level. Everything else stays at
        // Warn so third-party crates don't flood the log.
        .level(log::LevelFilter::Warn) // default for third-party
        .level_for("plexi", level)
        .level_for("plexi_v3", level)
        .level_for("plexi_alpha", level)
        .level_for("app", level);

    let dispatch = match file_result {
        Ok(file) => {
            if cli_mode {
                let stderr_dispatch = fern::Dispatch::new()
                    .level(log::LevelFilter::Warn)
                    .chain(std::io::stderr());
                dispatch.chain(stderr_dispatch).chain(file)
            } else {
                dispatch.chain(std::io::stderr()).chain(file)
            }
        }
        Err(e) => {
            eprintln!("[plexi::logging] could not open log file {log_file_path:?}: {e}; falling back to stderr only");
            dispatch.chain(std::io::stderr())
        }
    };

    if let Err(e) = dispatch.apply() {
        eprintln!("[plexi::logging] failed to install logger: {e}");
    }

    // Emit rotation/pruning messages now that the logger is running.
    for msg in deferred {
        log::info!(target: "plexi::logging", "{msg}");
    }
}

/// Shared counter bumped by the UI thread each frame so the heartbeat can detect freezes.
pub type FrameTick = Arc<AtomicU64>;

/// Create a new frame tick counter. Pass the clone to `spawn_heartbeat`, store the
/// original in `PlexiApp` and call `.fetch_add(1, Relaxed)` each frame.
pub fn new_frame_tick() -> FrameTick {
    Arc::new(AtomicU64::new(0))
}

/// Spawn a background thread that:
/// - Writes a heartbeat line every 30s (grep `[HEARTBEAT]` for last-alive timestamp)
/// - Detects UI-thread freezes via `frame_tick` and logs `[FREEZE]` / `[THAW]` lines
///
/// Both writes go directly to the file, bypassing the logger, so they survive
/// even if the logger thread is itself blocked by a freeze.
pub fn spawn_heartbeat(frame_tick: FrameTick) {
    let log_path = log_path();
    std::thread::Builder::new()
        .name("plexi-heartbeat".into())
        .spawn(move || {
            let mut last_tick = frame_tick.load(Ordering::Relaxed);
            // Track when we last saw the frame counter advance so freeze duration is accurate.
            let mut last_tick_seen_at = Instant::now();
            let mut freeze_reported = false;

            loop {
                std::thread::sleep(SAMPLE_INTERVAL);

                let now_tick = frame_tick.load(Ordering::Relaxed);
                let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");

                let mut lines = String::new();

                if now_tick == last_tick {
                    // Frame counter stalled — UI thread may be frozen.
                    let frozen_secs = last_tick_seen_at.elapsed().as_secs();
                    if frozen_secs >= FREEZE_THRESHOLD_SECS && !freeze_reported {
                        freeze_reported = true;
                        lines.push_str(&format!(
                            "[{now}] [WARN] [plexi::heartbeat] [FREEZE] UI thread unresponsive for {frozen_secs}s\n"
                        ));
                    }
                } else {
                    // Frame counter advanced — UI thread is alive.
                    if freeze_reported {
                        let frozen_secs = last_tick_seen_at.elapsed().as_secs();
                        lines.push_str(&format!(
                            "[{now}] [INFO] [plexi::heartbeat] [THAW] UI thread resumed after ~{frozen_secs}s\n"
                        ));
                        freeze_reported = false;
                    }
                    last_tick = now_tick;
                    last_tick_seen_at = Instant::now();
                }

                if !lines.is_empty() {
                    if let Ok(mut f) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&log_path)
                    {
                        use std::io::Write;
                        let _ = f.write_all(lines.as_bytes());
                    }
                }
            }
        })
        .expect("failed to spawn heartbeat thread");
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use std::fs;

    fn today_plus(days: i64) -> NaiveDate {
        chrono::Local::now().date_naive() + chrono::Duration::days(days)
    }

    #[test]
    fn rotates_previous_day_log() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("plexi.log");
        fs::write(&log_path, b"session data\n").unwrap();

        // Advance "today" by 1 day — the file's mtime (real today) becomes "yesterday".
        let simulated_today = today_plus(1);
        let expected_prev = today_plus(0);
        let expected_name = format!("plexi-{}.log", expected_prev.format("%Y-%m-%d"));

        let msgs = rotate_and_prune(&log_path, dir.path(), simulated_today, 30);

        assert!(
            dir.path().join(&expected_name).exists(),
            "should have rotated to {expected_name}"
        );
        assert!(!log_path.exists(), "plexi.log should have been renamed");
        assert!(msgs.iter().any(|m| m.contains("rotated")));
    }

    #[test]
    fn no_rotation_same_day() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("plexi.log");
        fs::write(&log_path, b"session data\n").unwrap();

        // "Today" is the real today — file mtime equals today, no rotation.
        let msgs = rotate_and_prune(&log_path, dir.path(), today_plus(0), 30);

        assert!(log_path.exists(), "plexi.log should not have been renamed");
        assert!(!msgs.iter().any(|m| m.contains("rotated")));
    }

    #[test]
    fn appends_when_dated_file_already_exists() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("plexi.log");
        let real_today = today_plus(0);
        let simulated_today = today_plus(1);

        // plexi.log has content from "today" (mtime = now).
        fs::write(&log_path, b"evening session\n").unwrap();
        // A dated file for real_today already exists (morning rotation already ran).
        let dated = dir
            .path()
            .join(format!("plexi-{}.log", real_today.format("%Y-%m-%d")));
        fs::write(&dated, b"morning session\n").unwrap();

        let msgs = rotate_and_prune(&log_path, dir.path(), simulated_today, 30);

        assert!(!log_path.exists(), "plexi.log should have been removed");
        let content = fs::read_to_string(&dated).unwrap();
        assert!(
            content.contains("morning session"),
            "original content preserved"
        );
        assert!(content.contains("evening session"), "new content appended");
        assert!(msgs.iter().any(|m| m.contains("appended")));
    }

    #[test]
    fn no_rotation_when_log_empty() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("plexi.log");
        fs::write(&log_path, b"").unwrap();

        let msgs = rotate_and_prune(&log_path, dir.path(), today_plus(1), 30);

        assert!(log_path.exists(), "empty plexi.log should not be renamed");
        assert!(!msgs.iter().any(|m| m.contains("rotated")));
    }

    #[test]
    fn prunes_old_dated_files() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("plexi.log");
        let today = NaiveDate::from_ymd_opt(2026, 5, 10).unwrap();

        // 40 days old — should be pruned (retention = 30).
        fs::write(dir.path().join("plexi-2026-03-31.log"), b"old").unwrap();
        // 10 days old — should be kept.
        fs::write(dir.path().join("plexi-2026-04-30.log"), b"recent").unwrap();
        // Non-log file — not touched.
        fs::write(dir.path().join("config.toml"), b"[log]").unwrap();

        let msgs = rotate_and_prune(&log_path, dir.path(), today, 30);

        assert!(
            !dir.path().join("plexi-2026-03-31.log").exists(),
            "40-day-old file should be pruned"
        );
        assert!(
            dir.path().join("plexi-2026-04-30.log").exists(),
            "10-day-old file should be kept"
        );
        assert!(
            dir.path().join("config.toml").exists(),
            "non-log files must not be touched"
        );
        assert!(msgs.iter().any(|m| m.contains("pruned 1")));
    }

    #[test]
    fn respects_custom_retention_days() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("plexi.log");
        let today = NaiveDate::from_ymd_opt(2026, 5, 10).unwrap();

        // 8 days old — pruned with retention=7, kept with retention=30.
        fs::write(dir.path().join("plexi-2026-05-02.log"), b"old").unwrap();

        rotate_and_prune(&log_path, dir.path(), today, 7);
        assert!(
            !dir.path().join("plexi-2026-05-02.log").exists(),
            "8-day-old file should be pruned with 7-day retention"
        );
    }
}
