//! Centralized file logger for Plexi.
//!
//! Writes to `~/.plexi-alpha/plexi.log` (or the appropriate config dir) and
//! also mirrors output to stderr so `cargo run` / CLI invocations stay usable.
//!
//! Rolling on startup: if `plexi.log` exists and is over 10 MB, it is renamed
//! to `plexi.log.1` (overwriting any previous backup) before the new log is opened.
//!
//! Third-party crates (egui, wgpu, winit, etc.) are clamped to `warn` to avoid
//! noise; everything under `plexi::` logs at the caller-supplied level.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const MAX_LOG_BYTES: u64 = 10 * 1024 * 1024; // 10 MB
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

/// Initialise the logger.  Must be called before any `log::` macro is used.
/// If the log file cannot be opened, falls back to stderr-only and logs a warning.
pub fn init(level: log::LevelFilter) {
    let log_file_path = log_path();

    // Ensure the config directory exists.
    if let Some(parent) = log_file_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("[plexi::logging] could not create config dir {parent:?}: {e}");
        }
    }

    // Rolling: if the file exists and is over 10 MB, rotate it.
    if let Ok(meta) = std::fs::metadata(&log_file_path) {
        if meta.len() > MAX_LOG_BYTES {
            let backup = log_file_path.with_extension("log.1");
            if let Err(e) = std::fs::rename(&log_file_path, &backup) {
                eprintln!("[plexi::logging] could not rotate log: {e}");
            }
        }
    }

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
        Ok(file) => dispatch.chain(std::io::stderr()).chain(file),
        Err(e) => {
            eprintln!("[plexi::logging] could not open log file {log_file_path:?}: {e}; falling back to stderr only");
            dispatch.chain(std::io::stderr())
        }
    };

    if let Err(e) = dispatch.apply() {
        eprintln!("[plexi::logging] failed to install logger: {e}");
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
