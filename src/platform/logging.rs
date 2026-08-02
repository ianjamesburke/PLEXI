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
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

/// Live log level for plexi-namespace targets. The fern filter reads this on
/// every record, so `[log] level` changes apply on config reload without a
/// restart. Third-party targets stay clamped to Warn regardless.
static PLEXI_LEVEL: AtomicUsize = AtomicUsize::new(log::LevelFilter::Info as usize);

fn plexi_level() -> log::LevelFilter {
    match PLEXI_LEVEL.load(Ordering::Relaxed) {
        0 => log::LevelFilter::Off,
        1 => log::LevelFilter::Error,
        2 => log::LevelFilter::Warn,
        3 => log::LevelFilter::Info,
        4 => log::LevelFilter::Debug,
        _ => log::LevelFilter::Trace,
    }
}

/// Change the live log level for plexi-namespace targets. Returns `true` if
/// the level actually changed. Safe to call from any thread after `init`.
pub fn set_level(level: log::LevelFilter) -> bool {
    let prev = PLEXI_LEVEL.swap(level as usize, Ordering::Relaxed);
    // Keep the global gate at the cheapest level that still admits both the
    // plexi level and the third-party Warn clamp.
    log::set_max_level(level.max(log::LevelFilter::Warn));
    prev != level as usize
}

/// Targets that follow the user-configured level: `plexi::*` (the crate is
/// named `plexi` on every channel — channels only rename the binary) and
/// `app::<id>` targets emitted by the SDK log bridge. Everything else is
/// third-party.
fn is_plexi_target(target: &str) -> bool {
    ["plexi", "app"].iter().any(|prefix| {
        target
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with("::"))
    })
}

/// Pure record-admission policy: plexi-namespace targets follow
/// `plexi_level`, third-party targets are clamped to Warn.
fn log_allowed(target: &str, level: log::Level, plexi_level: log::LevelFilter) -> bool {
    if is_plexi_target(target) {
        level <= plexi_level
    } else {
        level <= log::Level::Warn
    }
}

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

    PLEXI_LEVEL.store(level as usize, Ordering::Relaxed);

    let dispatch = fern::Dispatch::new()
        .format(formatter)
        // Record admission is dynamic (see `log_allowed`): plexi-namespace
        // targets follow PLEXI_LEVEL, which `set_level` can change live on
        // config reload; everything else stays clamped to Warn so
        // third-party crates don't flood the log.
        .filter(|meta| log_allowed(meta.target(), meta.level(), plexi_level()));

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
    } else {
        // With a filter-driven dispatch fern sets the global gate to Trace;
        // tighten it to the cheapest level that still admits everything.
        log::set_max_level(level.max(log::LevelFilter::Warn));
    }

    // Emit rotation/pruning messages now that the logger is running.
    for msg in deferred {
        log::info!(target: "plexi::logging", "{msg}");
    }
}

/// Shared counter bumped by the UI thread each frame so the heartbeat can detect freezes.
pub type FrameTick = Arc<AtomicU64>;

/// Last host phase reached by the UI thread. The phase and its monotonic
/// timestamp are packed into one atomic word so the heartbeat can take a
/// coherent snapshot without contending with the UI thread.
pub type UiPhaseTracker = Arc<AtomicU64>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum UiPhase {
    Startup = 0,
    Logic = 1,
    LogicComplete = 2,
    UiRender = 3,
    RendererPresent = 4,
    CloseRequest = 5,
    WorkspaceSave = 6,
    Exit = 7,
    // Preamble-level phases (`update_preamble`), in call order.
    PreambleAdoptedContext = 8,
    PreambleFinderService = 9,
    PreambleSpawnQueue = 10,
    PreambleTerminalActivity = 11,
    PreambleScheduler = 12,
    PreambleNotifyTimeouts = 13,
    PreambleUpdateCheck = 14,
    PreambleRuntimePoll = 15,
    PreamblePaneCmds = 16,
    PreamblePtyEvents = 17,
    PreambleSnapshot = 18,
    PreambleFocusSync = 19,
    // Logic-level phases (`PlexiApp::logic`), in call order.
    Screenshots = 20,
    SlotWaits = 21,
    AgentBoots = 22,
    Submits = 23,
    PaneHeartbeats = 24,
    SubscriptionReplies = 25,
    EventDeliveries = 26,
    PythonRuntimes = 27,
    AgentTick = 28,
    AppCommands = 29,
}

impl UiPhase {
    fn name(self) -> &'static str {
        match self {
            Self::Startup => "startup",
            Self::Logic => "logic",
            Self::LogicComplete => "logic_complete",
            Self::UiRender => "ui_render",
            Self::RendererPresent => "renderer_present",
            Self::CloseRequest => "close_request",
            Self::WorkspaceSave => "workspace_save",
            Self::Exit => "exit",
            Self::PreambleAdoptedContext => "preamble_adopted_context",
            Self::PreambleFinderService => "preamble_finder_service",
            Self::PreambleSpawnQueue => "preamble_spawn_queue",
            Self::PreambleTerminalActivity => "preamble_terminal_activity",
            Self::PreambleScheduler => "preamble_scheduler",
            Self::PreambleNotifyTimeouts => "preamble_notify_timeouts",
            Self::PreambleUpdateCheck => "preamble_update_check",
            Self::PreambleRuntimePoll => "preamble_runtime_poll",
            Self::PreamblePaneCmds => "preamble_pane_cmds",
            Self::PreamblePtyEvents => "preamble_pty_events",
            Self::PreambleSnapshot => "preamble_snapshot",
            Self::PreambleFocusSync => "preamble_focus_sync",
            Self::Screenshots => "screenshots",
            Self::SlotWaits => "slot_waits",
            Self::AgentBoots => "agent_boots",
            Self::Submits => "submits",
            Self::PaneHeartbeats => "pane_heartbeats",
            Self::SubscriptionReplies => "subscription_replies",
            Self::EventDeliveries => "event_deliveries",
            Self::PythonRuntimes => "python_runtimes",
            Self::AgentTick => "agent_tick",
            Self::AppCommands => "app_commands",
        }
    }

    fn from_byte(value: u8) -> Self {
        match value {
            1 => Self::Logic,
            2 => Self::LogicComplete,
            3 => Self::UiRender,
            4 => Self::RendererPresent,
            5 => Self::CloseRequest,
            6 => Self::WorkspaceSave,
            7 => Self::Exit,
            8 => Self::PreambleAdoptedContext,
            9 => Self::PreambleFinderService,
            10 => Self::PreambleSpawnQueue,
            11 => Self::PreambleTerminalActivity,
            12 => Self::PreambleScheduler,
            13 => Self::PreambleNotifyTimeouts,
            14 => Self::PreambleUpdateCheck,
            15 => Self::PreambleRuntimePoll,
            16 => Self::PreamblePaneCmds,
            17 => Self::PreamblePtyEvents,
            18 => Self::PreambleSnapshot,
            19 => Self::PreambleFocusSync,
            20 => Self::Screenshots,
            21 => Self::SlotWaits,
            22 => Self::AgentBoots,
            23 => Self::Submits,
            24 => Self::PaneHeartbeats,
            25 => Self::SubscriptionReplies,
            26 => Self::EventDeliveries,
            27 => Self::PythonRuntimes,
            28 => Self::AgentTick,
            29 => Self::AppCommands,
            _ => Self::Startup,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct UiPhaseSnapshot {
    name: &'static str,
    age: Duration,
}

static UI_PHASE_EPOCH: LazyLock<Instant> = LazyLock::new(Instant::now);

fn ui_phase_millis() -> u64 {
    UI_PHASE_EPOCH
        .elapsed()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX >> 8)
}

fn pack_ui_phase(phase: UiPhase, millis: u64) -> u64 {
    (millis.min(u64::MAX >> 8) << 8) | phase as u64
}

pub fn new_ui_phase_tracker() -> UiPhaseTracker {
    Arc::new(AtomicU64::new(pack_ui_phase(
        UiPhase::Startup,
        ui_phase_millis(),
    )))
}

pub(crate) fn mark_ui_phase(tracker: &UiPhaseTracker, phase: UiPhase) {
    tracker.store(pack_ui_phase(phase, ui_phase_millis()), Ordering::Release);
}

fn ui_phase_snapshot(tracker: &UiPhaseTracker) -> UiPhaseSnapshot {
    let packed = tracker.load(Ordering::Acquire);
    unpack_ui_phase(packed, ui_phase_millis())
}

fn unpack_ui_phase(packed: u64, now_ms: u64) -> UiPhaseSnapshot {
    let phase = UiPhase::from_byte(packed as u8);
    let marked_at_ms = packed >> 8;
    UiPhaseSnapshot {
        name: phase.name(),
        age: Duration::from_millis(now_ms.saturating_sub(marked_at_ms)),
    }
}

/// Test-only accessor: the last phase name reached by the tracker, without
/// locking. Lets integration tests (e.g. `HostHarness` smoke tests in other
/// modules) assert on host progress without exposing `UiPhaseSnapshot`.
#[cfg(test)]
pub(crate) fn current_ui_phase_name(tracker: &UiPhaseTracker) -> &'static str {
    ui_phase_snapshot(tracker).name
}

/// Pure threshold decision for the slow-drain log line: only worth a log line
/// once a single drain call has eaten a meaningful slice of one frame budget.
/// Kept separate from `time_drain` so it's directly unit-testable, matching
/// this file's existing pattern (see `freeze_verdict`).
pub(crate) fn should_log_slow_drain(elapsed: Duration) -> bool {
    elapsed > Duration::from_millis(100)
}

/// Time a drain call and log at `info` when it crosses `should_log_slow_drain`'s
/// threshold. No allocation on the happy path — the format! only runs when the
/// drain was actually slow.
pub(crate) fn time_drain<T>(name: &'static str, f: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let result = f();
    let elapsed = start.elapsed();
    if should_log_slow_drain(elapsed) {
        log::info!(
            target: "plexi::logging",
            "logic_drain: {name} took {}ms",
            elapsed.as_millis()
        );
    }
    result
}

/// Create a new frame tick counter. Pass the clone to `spawn_heartbeat`, store the
/// original in `PlexiApp` and call `.fetch_add(1, Relaxed)` each frame.
pub fn new_frame_tick() -> FrameTick {
    Arc::new(AtomicU64::new(0))
}

/// Shared slot the heartbeat reads the egui context from. Filled by
/// `PlexiApp::new` once eframe hands out the context; `None` until then.
/// The heartbeat needs it to distinguish a genuine UI freeze (repaint
/// requested but no frame produced) from healthy zero-frame idle.
pub type HeartbeatCtxSlot = Arc<Mutex<Option<egui::Context>>>;

/// Create an empty heartbeat context slot. Pass one clone to
/// `spawn_heartbeat` and another into `PlexiApp::new` to fill.
pub fn new_heartbeat_ctx_slot() -> HeartbeatCtxSlot {
    Arc::new(Mutex::new(None))
}

/// Pure freeze decision. A stalled frame counter alone is the *normal* idle
/// state (a fully idle Plexi produces zero frames); a freeze additionally
/// requires egui to have a pending repaint request that the UI thread is
/// failing to service. `repaint_pending` is `None` when the egui context is
/// not yet available — never report a freeze in that window.
pub(crate) fn freeze_verdict(counter_stalled: bool, repaint_pending: Option<bool>) -> bool {
    counter_stalled && repaint_pending == Some(true)
}

/// Spawn a background thread that detects UI-thread freezes via `frame_tick`
/// and logs `[FREEZE]` / `[THAW]` lines.
///
/// `egui_ctx` gates the verdict: with zero-frame idle, a stalled counter only
/// counts as frozen while egui has a repaint request pending (see
/// `freeze_verdict`). Healthy idle produces neither FREEZE nor THAW lines.
///
/// Writes go directly to the file, bypassing the logger, so they survive
/// even if the logger thread is itself blocked by a freeze.
pub fn spawn_heartbeat(
    frame_tick: FrameTick,
    egui_ctx: HeartbeatCtxSlot,
    ui_phase: UiPhaseTracker,
) {
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
                    // Frame counter stalled — frozen only if egui has a
                    // repaint pending that the UI thread isn't servicing.
                    // Zero frames with nothing pending is healthy idle.
                    let repaint_pending = egui_ctx
                        .lock()
                        .ok()
                        .and_then(|slot| slot.as_ref().map(|c| c.has_requested_repaint()));
                    if freeze_verdict(true, repaint_pending) {
                        let frozen_secs = last_tick_seen_at.elapsed().as_secs();
                        if frozen_secs >= FREEZE_THRESHOLD_SECS && !freeze_reported {
                            freeze_reported = true;
                            let phase = ui_phase_snapshot(&ui_phase);
                            let phase_age_ms = phase.age.as_millis();
                            lines.push_str(&format!(
                                "[{now}] [WARN] [plexi::heartbeat] [FREEZE] UI thread unresponsive for {frozen_secs}s phase={} phase_age_ms={phase_age_ms}\n",
                                phase.name
                            ));
                        }
                    } else if !freeze_reported {
                        // Healthy idle: keep the freeze clock pinned to now so
                        // a later pending repaint measures only its own stall,
                        // not the whole idle period before it.
                        last_tick_seen_at = Instant::now();
                    }
                } else {
                    // Frame counter advanced — UI thread is alive.
                    if freeze_reported {
                        let frozen_secs = last_tick_seen_at.elapsed().as_secs();
                        let phase = ui_phase_snapshot(&ui_phase);
                        let phase_age_ms = phase.age.as_millis();
                        lines.push_str(&format!(
                            "[{now}] [INFO] [plexi::heartbeat] [THAW] UI thread resumed after ~{frozen_secs}s phase={} phase_age_ms={phase_age_ms}\n",
                            phase.name
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

    #[test]
    fn ui_phase_snapshot_tracks_last_phase_without_locking() {
        let tracker = Arc::new(AtomicU64::new(pack_ui_phase(UiPhase::Startup, 0)));
        tracker.store(pack_ui_phase(UiPhase::WorkspaceSave, 42), Ordering::Release);
        let snapshot = unpack_ui_phase(tracker.load(Ordering::Acquire), 100);
        assert_eq!(snapshot.name, "workspace_save");
        assert_eq!(snapshot.age, Duration::from_millis(58));
    }

    /// Every `UiPhase` variant must round-trip through `from_byte`, including
    /// the preamble- and logic-level phases added in stint 0716. A gap here
    /// means a variant silently decodes back to `Startup`.
    #[test]
    fn every_ui_phase_variant_round_trips_through_from_byte() {
        let variants = [
            UiPhase::Startup,
            UiPhase::Logic,
            UiPhase::LogicComplete,
            UiPhase::UiRender,
            UiPhase::RendererPresent,
            UiPhase::CloseRequest,
            UiPhase::WorkspaceSave,
            UiPhase::Exit,
            UiPhase::PreambleAdoptedContext,
            UiPhase::PreambleFinderService,
            UiPhase::PreambleSpawnQueue,
            UiPhase::PreambleTerminalActivity,
            UiPhase::PreambleScheduler,
            UiPhase::PreambleNotifyTimeouts,
            UiPhase::PreambleUpdateCheck,
            UiPhase::PreambleRuntimePoll,
            UiPhase::PreamblePaneCmds,
            UiPhase::PreamblePtyEvents,
            UiPhase::PreambleSnapshot,
            UiPhase::PreambleFocusSync,
            UiPhase::Screenshots,
            UiPhase::SlotWaits,
            UiPhase::AgentBoots,
            UiPhase::Submits,
            UiPhase::PaneHeartbeats,
            UiPhase::SubscriptionReplies,
            UiPhase::EventDeliveries,
            UiPhase::PythonRuntimes,
            UiPhase::AgentTick,
            UiPhase::AppCommands,
        ];
        for variant in variants {
            let round_tripped = UiPhase::from_byte(variant as u8);
            assert_eq!(
                round_tripped,
                variant,
                "variant {} (byte {}) did not round-trip; from_byte returned {}",
                variant.name(),
                variant as u8,
                round_tripped.name()
            );
        }
    }

    #[test]
    fn should_log_slow_drain_below_threshold_is_false() {
        assert!(!should_log_slow_drain(Duration::from_millis(50)));
        assert!(!should_log_slow_drain(Duration::from_millis(100)));
    }

    #[test]
    fn should_log_slow_drain_above_threshold_is_true() {
        assert!(should_log_slow_drain(Duration::from_millis(101)));
        assert!(should_log_slow_drain(Duration::from_secs(1)));
    }

    /// Zero frames with no pending repaint is healthy idle — never a freeze.
    #[test]
    fn freeze_verdict_idle_without_pending_repaint_is_not_frozen() {
        assert!(!freeze_verdict(true, Some(false)));
    }

    /// Stalled counter with a pending repaint the UI thread isn't servicing
    /// is the genuine freeze condition.
    #[test]
    fn freeze_verdict_stalled_with_pending_repaint_is_frozen() {
        assert!(freeze_verdict(true, Some(true)));
    }

    /// Before `PlexiApp::new` fills the context slot there is no signal —
    /// never report a freeze in that window.
    #[test]
    fn freeze_verdict_without_ctx_is_not_frozen() {
        assert!(!freeze_verdict(true, None));
    }

    /// An advancing frame counter is never frozen, whatever egui says.
    #[test]
    fn freeze_verdict_advancing_counter_is_not_frozen() {
        assert!(!freeze_verdict(false, Some(true)));
        assert!(!freeze_verdict(false, Some(false)));
        assert!(!freeze_verdict(false, None));
    }

    /// plexi-namespace targets follow the dynamic level.
    #[test]
    fn log_allowed_plexi_targets_follow_dynamic_level() {
        assert!(log_allowed(
            "plexi::frame_diag",
            log::Level::Debug,
            log::LevelFilter::Debug
        ));
        assert!(!log_allowed(
            "plexi::frame_diag",
            log::Level::Debug,
            log::LevelFilter::Info
        ));
        assert!(!log_allowed(
            "plexi::frame_diag",
            log::Level::Info,
            log::LevelFilter::Warn
        ));
    }

    /// SDK log-bridge targets (`app::<id>`) follow the dynamic level too.
    #[test]
    fn log_allowed_app_bridge_targets_follow_dynamic_level() {
        assert!(log_allowed(
            "app::logs",
            log::Level::Info,
            log::LevelFilter::Info
        ));
        assert!(!log_allowed(
            "app::logs",
            log::Level::Info,
            log::LevelFilter::Warn
        ));
    }

    /// Third-party targets are clamped to Warn even at debug plexi level.
    #[test]
    fn log_allowed_third_party_clamped_to_warn() {
        assert!(!log_allowed(
            "wgpu_core::device",
            log::Level::Info,
            log::LevelFilter::Debug
        ));
        assert!(log_allowed(
            "wgpu_core::device",
            log::Level::Warn,
            log::LevelFilter::Error
        ));
    }

    /// Prefix lookalikes (`plexiglass`) are third-party, not plexi-namespace.
    #[test]
    fn log_allowed_prefix_lookalike_is_third_party() {
        assert!(!log_allowed(
            "plexiglass::foo",
            log::Level::Info,
            log::LevelFilter::Debug
        ));
        assert!(!log_allowed(
            "appkit",
            log::Level::Info,
            log::LevelFilter::Debug
        ));
    }

    /// set_level reports whether the level actually changed and round-trips
    /// through the atomic.
    #[test]
    fn set_level_round_trips_and_reports_change() {
        let original = plexi_level();
        assert!(set_level(log::LevelFilter::Debug) || original == log::LevelFilter::Debug);
        assert_eq!(plexi_level(), log::LevelFilter::Debug);
        assert!(!set_level(log::LevelFilter::Debug), "same level is a no-op");
        assert!(set_level(log::LevelFilter::Error));
        assert_eq!(plexi_level(), log::LevelFilter::Error);
        set_level(original);
    }

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
