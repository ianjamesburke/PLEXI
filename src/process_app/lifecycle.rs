//! Observable app lifecycle (issue #316).
//!
//! `LifecycleTracker` is a thread-safe view of an app's health that the
//! background readers (stdout, stderr, `try_wait` poller) write to and the
//! UI thread reads from each frame to render the in-pane status pill.
//!
//! Five distinguishable states:
//!
//! - `Booting`       — process spawned, no FrameDone yet. Faint pill.
//! - `Running`       — last FrameDone within `RUNNING_FRESH_WINDOW`. No pill.
//! - `Hung`          — process alive, no FrameDone in `HUNG_FRAME_GAP`,
//!                     AND user input observed in that window.
//! - `Crashed`       — subprocess exited / stderr matched `Traceback`/`PANIC` /
//!                     stdout closed.
//! - `ProtocolError` — stdout JSON parse failures exceeded
//!                     `PROTOCOL_ERROR_THRESHOLD` within
//!                     `PROTOCOL_ERROR_WINDOW`.
//!
//! Once a state is *terminal* (Crashed / ProtocolError) it never transitions
//! back to a healthier state — those are sticky. `Hung` may transition to
//! `Running` again when a fresh FrameDone arrives.

use std::sync::atomic::{AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::time::{Duration, Instant};

/// Five lifecycle states for an app process.
///
/// Wire encoding is the discriminant value — kept stable so the atomic
/// load/store round-trip stays cheap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LifecycleState {
    Booting = 0,
    Running = 1,
    Hung = 2,
    Crashed = 3,
    ProtocolError = 4,
}

impl LifecycleState {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Booting,
            1 => Self::Running,
            2 => Self::Hung,
            3 => Self::Crashed,
            4 => Self::ProtocolError,
            // Unknown discriminants degrade to Booting — better than panicking.
            _ => Self::Booting,
        }
    }

    /// Terminal states never transition back to a healthier state.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Crashed | Self::ProtocolError)
    }
}

// Tunables. `HUNG_FRAME_GAP` is the spec's 5s stall threshold. The
// protocol-error counter ticks N=3 within M=10s — three malformed lines
// from an app is catastrophic regardless of why; show the user something
// is wrong. `BOOT_TIMEOUT` caps how long an app may stay in Booting before
// the pane transitions to Crashed — prevents silent "starting…" hangs when
// an app deadlocks during init or never emits Ready.
pub const HUNG_FRAME_GAP: Duration = Duration::from_secs(5);
pub const BOOT_TIMEOUT: Duration = Duration::from_secs(10);
pub const PROTOCOL_ERROR_THRESHOLD: u32 = 3;
pub const PROTOCOL_ERROR_WINDOW: Duration = Duration::from_secs(10);

/// Thread-safe lifecycle state shared between the stdout reader,
/// stderr reader, the `try_wait` poller, and the UI thread.
///
/// All counters use a fixed monotonic origin (`launched_at: Instant`) so we
/// can store millisecond offsets in `AtomicU64` instead of locking around an
/// `Instant`. Two reads from different threads can observe slightly stale
/// values; that's fine — the pill only updates per frame anyway.
pub struct LifecycleTracker {
    state: AtomicU8,
    /// Origin instant. Captured once at construction.
    launched_at: Instant,
    /// Milliseconds since `launched_at` of the most recent FrameDone.
    /// Zero means "no frame yet" — distinguishable from any real timestamp
    /// because frame 0 lands at least one tick after launch.
    last_frame_done_ms: AtomicU64,
    /// Milliseconds since `launched_at` of the most recent observed user
    /// input on this pane. Zero means "never interacted".
    last_input_ms: AtomicU64,
    /// Sliding-window protocol-error counter.
    parse_error_count: AtomicU32,
    /// Window start (ms since `launched_at`) for the parse-error counter.
    parse_error_window_start_ms: AtomicU64,
}

impl LifecycleTracker {
    pub fn new() -> Self {
        Self {
            state: AtomicU8::new(LifecycleState::Booting as u8),
            launched_at: Instant::now(),
            last_frame_done_ms: AtomicU64::new(0),
            last_input_ms: AtomicU64::new(0),
            parse_error_count: AtomicU32::new(0),
            parse_error_window_start_ms: AtomicU64::new(0),
        }
    }

    /// Test-only ctor that pins `launched_at` so tests can inject deterministic
    /// timestamps without sleeping.
    #[cfg(test)]
    fn with_origin(origin: Instant) -> Self {
        Self {
            state: AtomicU8::new(LifecycleState::Booting as u8),
            launched_at: origin,
            last_frame_done_ms: AtomicU64::new(0),
            last_input_ms: AtomicU64::new(0),
            parse_error_count: AtomicU32::new(0),
            parse_error_window_start_ms: AtomicU64::new(0),
        }
    }

    pub fn state(&self) -> LifecycleState {
        LifecycleState::from_u8(self.state.load(Ordering::Acquire))
    }

    /// Set state unless we're already in a terminal (sticky) state.
    /// Returns the state that's actually in effect after the call.
    fn set_state(&self, new: LifecycleState) -> LifecycleState {
        let current = self.state();
        if current.is_terminal() && new != current {
            return current;
        }
        self.state.store(new as u8, Ordering::Release);
        new
    }

    /// Force-set a terminal state. Sticky — overrides any prior state.
    fn set_terminal(&self, new: LifecycleState) {
        debug_assert!(new.is_terminal());
        self.state.store(new as u8, Ordering::Release);
    }

    fn elapsed_ms(&self) -> u64 {
        // Saturate on Instant subtraction so we never panic on backwards clocks.
        self.launched_at.elapsed().as_millis() as u64
    }

    /// Called by the stdout reader thread on every successful FrameDone.
    /// Sets state to Running (unless terminal).
    pub fn on_frame_done(&self) {
        self.last_frame_done_ms
            .store(self.elapsed_ms().max(1), Ordering::Release);
        self.set_state(LifecycleState::Running);
    }

    /// Called by the host UI thread when egui reports input events on this
    /// pane. Used to gate the Hung detection — a stalled app that the user
    /// isn't poking is just idle, not hung.
    pub fn on_user_input(&self) {
        self.last_input_ms
            .store(self.elapsed_ms().max(1), Ordering::Release);
    }

    /// Called when `process.try_wait()` returns `Some(_)` — i.e. the
    /// subprocess has exited. Sticky: Crashed.
    pub fn on_process_exited(&self) {
        self.set_terminal(LifecycleState::Crashed);
    }

    /// Called when the stdout reader thread observes a closed pipe
    /// (EOF on stdout). Sticky: Crashed.
    pub fn on_stdout_closed(&self) {
        self.set_terminal(LifecycleState::Crashed);
    }

    /// Called by the stderr reader thread on every captured line.
    /// Recognises Python `Traceback` and Rust `PANIC` patterns and
    /// flips state to Crashed if found.
    pub fn observe_stderr_line(&self, line: &str) {
        if line.contains("Traceback") || line.contains("PANIC") || line.contains("panicked at") {
            self.set_terminal(LifecycleState::Crashed);
        }
    }

    /// Called by the stdout reader thread on a malformed JSON line.
    /// Returns `true` if the threshold was crossed *on this call* (caller
    /// can log a one-shot diagnostic).
    pub fn on_parse_error(&self) -> bool {
        let now = self.elapsed_ms();
        let window_start = self.parse_error_window_start_ms.load(Ordering::Acquire);
        let window_ms = PROTOCOL_ERROR_WINDOW.as_millis() as u64;

        // If the previous window has expired (or this is the first error),
        // reset the count and start a fresh window.
        if window_start == 0 || now.saturating_sub(window_start) > window_ms {
            self.parse_error_window_start_ms
                .store(now, Ordering::Release);
            self.parse_error_count.store(1, Ordering::Release);
            return false;
        }

        let prior = self.parse_error_count.fetch_add(1, Ordering::AcqRel);
        let count = prior + 1;
        if count >= PROTOCOL_ERROR_THRESHOLD {
            self.set_terminal(LifecycleState::ProtocolError);
            return true;
        }
        false
    }

    /// Called once per UI frame. Performs the time-based Hung check that
    /// can't be driven by an event-arrival callback. Returns the resulting
    /// state for caller convenience.
    ///
    /// Hung iff:
    ///   - current state is Booting or Running (not terminal, not already Hung),
    ///   - we've seen a FrameDone (`last_frame_done_ms != 0`),
    ///   - the gap since that FrameDone exceeds `HUNG_FRAME_GAP`,
    ///   - AND user input was observed within that gap (otherwise the
    ///     app is just idle, which is fine).
    pub fn tick_check_hung(&self) -> LifecycleState {
        let current = self.state();
        if current.is_terminal() || current == LifecycleState::Hung {
            return current;
        }
        let now = self.elapsed_ms();
        let last_frame = self.last_frame_done_ms.load(Ordering::Acquire);
        if last_frame == 0 {
            // Still booting — flip to Crashed if boot timeout exceeded.
            if now > BOOT_TIMEOUT.as_millis() as u64 {
                self.set_terminal(LifecycleState::Crashed);
                return LifecycleState::Crashed;
            }
            return current;
        }
        let gap_ms = now.saturating_sub(last_frame);
        if gap_ms < HUNG_FRAME_GAP.as_millis() as u64 {
            return current;
        }
        let last_input = self.last_input_ms.load(Ordering::Acquire);
        if last_input == 0 || last_input < last_frame {
            // No interaction since the last frame — just idle.
            return current;
        }
        self.set_state(LifecycleState::Hung)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn tracker() -> LifecycleTracker {
        // Origin 60s ago so elapsed_ms() returns deterministic large values
        // without us actually sleeping in the test.
        LifecycleTracker::with_origin(Instant::now() - Duration::from_secs(60))
    }

    #[test]
    fn starts_booting() {
        let t = LifecycleTracker::new();
        assert_eq!(t.state(), LifecycleState::Booting);
    }

    #[test]
    fn frame_done_transitions_to_running() {
        let t = tracker();
        t.on_frame_done();
        assert_eq!(t.state(), LifecycleState::Running);
    }

    #[test]
    fn process_exit_is_sticky_crashed() {
        let t = tracker();
        t.on_process_exited();
        assert_eq!(t.state(), LifecycleState::Crashed);
        // Subsequent FrameDone (rogue thread) must not unstick Crashed.
        t.on_frame_done();
        assert_eq!(t.state(), LifecycleState::Crashed);
    }

    #[test]
    fn stderr_traceback_marks_crashed() {
        let t = tracker();
        t.observe_stderr_line("Traceback (most recent call last):");
        assert_eq!(t.state(), LifecycleState::Crashed);
    }

    #[test]
    fn stderr_rust_panic_marks_crashed() {
        let t = tracker();
        t.observe_stderr_line("thread 'main' panicked at src/foo.rs:1:1");
        assert_eq!(t.state(), LifecycleState::Crashed);
    }

    #[test]
    fn stderr_normal_line_is_ignored() {
        let t = tracker();
        t.observe_stderr_line("debug: starting up");
        assert_eq!(t.state(), LifecycleState::Booting);
    }

    #[test]
    fn parse_errors_under_threshold_do_not_trip_protocol_error() {
        let t = tracker();
        assert!(!t.on_parse_error());
        assert!(!t.on_parse_error());
        assert_eq!(t.state(), LifecycleState::Booting);
    }

    #[test]
    fn parse_errors_at_threshold_trip_protocol_error() {
        let t = tracker();
        assert!(!t.on_parse_error());
        assert!(!t.on_parse_error());
        assert!(t.on_parse_error());
        assert_eq!(t.state(), LifecycleState::ProtocolError);
    }

    #[test]
    fn protocol_error_is_sticky() {
        let t = tracker();
        for _ in 0..3 {
            t.on_parse_error();
        }
        assert_eq!(t.state(), LifecycleState::ProtocolError);
        // FrameDone must not unstick ProtocolError.
        t.on_frame_done();
        assert_eq!(t.state(), LifecycleState::ProtocolError);
    }

    #[test]
    fn parse_errors_outside_window_reset_counter() {
        // Origin far in the past so the simulated "first error" is far
        // enough back that the second error window has already expired.
        let origin = Instant::now() - Duration::from_secs(60);
        let t = LifecycleTracker::with_origin(origin);
        // Force two errors then rewind the window-start so the next
        // error counts as "in a fresh window".
        t.on_parse_error();
        t.on_parse_error();
        // Manually mark the window start as expired by setting it to 0.
        // (Equivalent to the natural M=10s gap passing.)
        t.parse_error_window_start_ms.store(0, Ordering::Release);
        assert!(!t.on_parse_error());
        assert_eq!(t.state(), LifecycleState::Booting);
    }

    #[test]
    fn hung_requires_a_frame_already_landed() {
        // Use a fresh tracker so we're well within BOOT_TIMEOUT.
        let t = LifecycleTracker::new();
        // No FrameDone ever — must stay Booting, not flip to Hung.
        t.on_user_input();
        assert_eq!(t.tick_check_hung(), LifecycleState::Booting);
    }

    #[test]
    fn boot_timeout_flips_to_crashed() {
        // tracker() pins origin 60s in the past — well beyond BOOT_TIMEOUT.
        let t = tracker();
        // No FrameDone ever → boot timeout fires.
        assert_eq!(t.tick_check_hung(), LifecycleState::Crashed);
        // Must be sticky.
        t.on_frame_done();
        assert_eq!(t.state(), LifecycleState::Crashed);
    }

    #[test]
    fn hung_requires_user_input_during_stall() {
        // Frame at t=launch+10ms (since `tracker()` rewinds origin 60s),
        // no input at all → idle, not hung.
        let t = tracker();
        t.on_frame_done();
        // Far future check (the 60s rewind makes elapsed_ms() ~60_000).
        assert_eq!(t.tick_check_hung(), LifecycleState::Running);
    }

    #[test]
    fn hung_triggers_when_input_observed_after_stale_frame() {
        let origin = Instant::now() - Duration::from_secs(60);
        let t = LifecycleTracker::with_origin(origin);
        // Frame landed 30s after launch, input at 50s (still 60s after origin
        // total elapsed when we tick). Gap 30s+ since frame, input post-frame.
        t.last_frame_done_ms.store(30_000, Ordering::Release);
        t.set_state(LifecycleState::Running);
        t.last_input_ms.store(50_000, Ordering::Release);
        assert_eq!(t.tick_check_hung(), LifecycleState::Hung);
    }

    #[test]
    fn hung_does_not_overwrite_terminal() {
        let origin = Instant::now() - Duration::from_secs(60);
        let t = LifecycleTracker::with_origin(origin);
        t.last_frame_done_ms.store(1_000, Ordering::Release);
        t.last_input_ms.store(50_000, Ordering::Release);
        t.set_terminal(LifecycleState::Crashed);
        assert_eq!(t.tick_check_hung(), LifecycleState::Crashed);
    }

    #[test]
    fn from_u8_round_trips_known_values() {
        for s in [
            LifecycleState::Booting,
            LifecycleState::Running,
            LifecycleState::Hung,
            LifecycleState::Crashed,
            LifecycleState::ProtocolError,
        ] {
            assert_eq!(LifecycleState::from_u8(s as u8), s);
        }
    }

    #[test]
    fn from_u8_unknown_degrades_to_booting() {
        assert_eq!(LifecycleState::from_u8(99), LifecycleState::Booting);
    }
}

/// Integration test: spawn a real subprocess that exits with a Python
/// `Traceback`, drive it through `ProcessApp::launch`, and assert the
/// lifecycle reaches Crashed within 1s.
///
/// Lives in this file (not a `tests/` directory) to keep crate-private
/// access to `ProcessApp` internals — same pattern as the unit tests above.
#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::process_app::ProcessApp;
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::time::Instant;

    /// Find a usable shell on the host. Tests skip gracefully if none is
    /// available (the CI smoke-test always runs on macOS where `/bin/sh`
    /// exists).
    fn find_shell() -> Option<PathBuf> {
        for candidate in ["/bin/sh", "/usr/bin/sh"] {
            let p = PathBuf::from(candidate);
            if p.exists() {
                return Some(p);
            }
        }
        None
    }

    #[test]
    fn crashing_subprocess_reaches_crashed_within_1s() {
        let Some(sh) = find_shell() else {
            eprintln!("skipping: no /bin/sh available");
            return;
        };
        // tmpdir as workspace_root — must be absolute + existing.
        let workspace_root = std::env::temp_dir();
        // Script that prints a Python-style Traceback to stderr and exits.
        let args = vec![
            "-c".to_string(),
            "echo 'Traceback (most recent call last):' >&2; exit 1".to_string(),
        ];
        let app = ProcessApp::launch(
            "test_crashing_app",
            "Test Crashing App",
            &sh,
            &workspace_root,
            &args,
            workspace_root.clone(),
            HashSet::new(),
            false,
            None,
        )
        .expect("launch crashing app");

        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            // The stderr-reader thread observes the Traceback line; the
            // stdout-reader thread observes EOF when the process exits.
            // Either path flips us to Crashed. We *don't* call try_wait()
            // here — that's the host UI thread's job — but stdout EOF is
            // detected by the background reader.
            app.lifecycle.observe_stderr_lines_drain_for_test();
            if app.lifecycle.state() == LifecycleState::Crashed {
                return;
            }
            if Instant::now() > deadline {
                panic!(
                    "expected Crashed within 1s, got {:?}",
                    app.lifecycle.state()
                );
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

// ── Test-only helper exposed on LifecycleTracker ──────────────────────────────
//
// The integration test above polls without a UI thread, so it has nowhere
// for the stderr-reader-thread's writes to "land" except directly on the
// tracker. This no-op exists purely to make the polling loop's intent
// explicit. The real stderr thread already calls `observe_stderr_line` per
// line; the test's only job is to wait.
#[cfg(test)]
impl LifecycleTracker {
    fn observe_stderr_lines_drain_for_test(&self) {
        // Intentionally empty — the stderr reader thread does the work.
    }
}
