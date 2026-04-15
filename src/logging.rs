/// Centralized file logger for Plexi.
///
/// Writes to `~/.plexi-alpha/plexi.log` (or the appropriate config dir) and
/// also mirrors output to stderr so `cargo run` / CLI invocations stay usable.
///
/// Rolling on startup: if `plexi.log` exists and is over 10 MB, it is renamed
/// to `plexi.log.1` (overwriting any previous backup) before the new log is opened.
///
/// Third-party crates (egui, wgpu, winit, etc.) are clamped to `warn` to avoid
/// noise; everything under `plexi::` logs at the caller-supplied level.

use std::path::PathBuf;

const MAX_LOG_BYTES: u64 = 10 * 1024 * 1024; // 10 MB

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
    let formatter = |out: fern::FormatCallback, message: &std::fmt::Arguments, record: &log::Record| {
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

    // CARGO_CRATE_NAME is the crate name with hyphens replaced by underscores,
    // which matches the module path prefix Rust uses for log targets.
    // "plexi" → "plexi::*", "plexi-alpha" → "plexi_alpha::*", etc.
    // Using a hardcoded "plexi" would NOT match "plexi_alpha::*" because fern
    // checks `target.starts_with("plexi::")` — note the "::" separator.
    let crate_log_target: &'static str = env!("CARGO_CRATE_NAME");

    let dispatch = fern::Dispatch::new()
        .format(formatter)
        // plexi::* (or plexi_alpha::*, plexi_beta::*) at the configured level;
        // all third-party crates fall back to Warn.
        .level(log::LevelFilter::Warn)
        .level_for(crate_log_target, level);

    let dispatch = match file_result {
        Ok(file) => dispatch
            .chain(std::io::stderr())
            .chain(file),
        Err(e) => {
            eprintln!("[plexi::logging] could not open log file {log_file_path:?}: {e}; falling back to stderr only");
            dispatch.chain(std::io::stderr())
        }
    };

    if let Err(e) = dispatch.apply() {
        eprintln!("[plexi::logging] failed to install logger: {e}");
    }
}
