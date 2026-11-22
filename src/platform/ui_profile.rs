//! Per-frame `App::ui` cost profiling, attributed by pane kind (stint 0731).
//!
//! Stint 0716's `[FREEZE]` / `logic_drain:` instrumentation lives in
//! `App::logic` and is structurally blind to drag-resize stutter, which
//! happens in `App::ui`. This module measures where `ui()` frame time goes
//! during an active drag or window resize, gated behind `PLEXI_UI_PROFILE=1`
//! so it costs nothing on the default hot path. [`ResizeGesture`] is the one
//! always-on part: a start and end line per window resize (stint 1224).

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Resolves once from `PLEXI_UI_PROFILE=1`. Every other fn in this module
/// must check this before any `Instant::now()`/alloc/lock — zero cost when
/// off is a hard requirement.
pub fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("PLEXI_UI_PROFILE").as_deref() == Ok("1"))
}

/// Accumulated (nanos, calls) per label for the active drag window.
static SPANS: LazyLock<Mutex<HashMap<String, (u64, u32)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Time a closure and accumulate it into `SPANS` under `label`. A no-op call
/// to `f()` when profiling is disabled.
pub fn time<T>(label: &str, f: impl FnOnce() -> T) -> T {
    if !enabled() {
        return f();
    }
    let start = Instant::now();
    let result = f();
    note_nanos(label, start.elapsed().as_nanos() as u64);
    result
}

/// Record `nanos` of cost against `label`. Used directly by the egui_term
/// span hook, which times its own closure and reports the result across the
/// crate boundary (egui_term cannot depend on this module).
pub fn note_nanos(label: &str, nanos: u64) {
    if !enabled() {
        return;
    }
    let mut spans = SPANS.lock().unwrap();
    let entry = spans.entry(label.to_string()).or_insert((0, 0));
    entry.0 += nanos;
    entry.1 += 1;
}

fn drain_spans() -> Vec<(String, u64, u32)> {
    let mut spans = SPANS.lock().unwrap();
    let mut out: Vec<(String, u64, u32)> = spans
        .drain()
        .map(|(label, (nanos, calls))| (label, nanos, calls))
        .collect();
    out.sort_by_key(|(_, nanos, _)| std::cmp::Reverse(*nanos));
    out
}

/// One sample window covering a single drag/resize gesture (edge-triggered
/// by [`set_drag_active`]) plus a 300ms trailing hold so the window doesn't
/// flap between frames on brief input gaps.
struct DragWindow {
    started: Instant,
    frames: u32,
    frame_ms: Vec<f32>,
}

static WINDOW: Mutex<Option<DragWindow>> = Mutex::new(None);

/// Threshold above which a single frame is logged individually as a
/// `slow_frame` outlier while a drag window is active.
pub fn should_log_slow_frame(elapsed: Duration) -> bool {
    elapsed > Duration::from_millis(16)
}

/// Record this frame's total `ui()` elapsed time into the active drag
/// window, if one is open. No-op when profiling is disabled or no window
/// is active.
pub fn end_frame(total: Duration) {
    if !enabled() {
        return;
    }
    let mut window = WINDOW.lock().unwrap();
    let Some(w) = window.as_mut() else {
        return;
    };
    w.frames += 1;
    w.frame_ms.push(total.as_secs_f32() * 1000.0);

    if should_log_slow_frame(total) {
        let spans = drain_top_without_clearing(4);
        log::info!(
            target: "plexi::ui_profile",
            "[UIPROF] slow_frame ui={:.1}ms top: {}",
            total.as_secs_f32() * 1000.0,
            spans
                .iter()
                .map(|(label, nanos, _)| format!("{label}={:.1}ms", *nanos as f32 / 1_000_000.0))
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
}

/// Peek the top `n` spans by accumulated time without resetting the
/// accumulator — the drag window is still open and spans keep accumulating
/// until it closes.
fn drain_top_without_clearing(n: usize) -> Vec<(String, u64, u32)> {
    let spans = SPANS.lock().unwrap();
    let mut out: Vec<(String, u64, u32)> = spans
        .iter()
        .map(|(label, (nanos, calls))| (label.clone(), *nanos, *calls))
        .collect();
    out.sort_by_key(|(_, nanos, _)| std::cmp::Reverse(*nanos));
    out.truncate(n);
    out
}

/// Edge-triggered drag-state transition. Rising edge opens a new window and
/// clears accumulated spans; falling edge emits the summary log line and
/// clears the window. No-op when profiling is disabled.
pub fn set_drag_active(active: bool) {
    if !enabled() {
        return;
    }
    let mut window = WINDOW.lock().unwrap();
    match (window.is_some(), active) {
        (false, true) => {
            drop(SPANS.lock().unwrap().drain());
            *window = Some(DragWindow {
                started: Instant::now(),
                frames: 0,
                frame_ms: Vec::new(),
            });
        }
        (true, false) => {
            let w = window.take().unwrap();
            let dur_s = w.started.elapsed().as_secs_f32();
            let (p50, p95, max) = percentiles(&w.frame_ms);
            let spans = drain_spans();
            log::info!(
                target: "plexi::ui_profile",
                "{}",
                window_summary_line(w.frames, dur_s, p50, p95, max, &spans)
            );
        }
        _ => {}
    }
}

fn percentiles(frame_ms: &[f32]) -> (f32, f32, f32) {
    if frame_ms.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let mut sorted = frame_ms.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let at = |q: f32| -> f32 {
        let idx = ((sorted.len() as f32 - 1.0) * q).round() as usize;
        sorted[idx.min(sorted.len() - 1)]
    };
    (at(0.5), at(0.95), *sorted.last().unwrap())
}

/// Formats the `[UIPROF] drag_window` summary line closing a drag/resize
/// sample window, e.g.
/// `[UIPROF] drag_window frames=87 dur=1.43s fps=60.8 frame_ms p50=11.2 p95=31.4 max=48.9 spans: terminal=612.4ms/261 sidebar=61.0ms/87`.
pub fn window_summary_line(
    frames: u32,
    dur_s: f32,
    p50: f32,
    p95: f32,
    max: f32,
    spans: &[(String, u64, u32)],
) -> String {
    let fps = if dur_s > 0.0 {
        frames as f32 / dur_s
    } else {
        0.0
    };
    let mut sorted: Vec<&(String, u64, u32)> = spans.iter().collect();
    sorted.sort_by_key(|(_, nanos, _)| std::cmp::Reverse(*nanos));
    let mut line = format!(
        "[UIPROF] drag_window frames={frames} dur={dur_s:.2}s fps={fps:.1} frame_ms p50={p50:.1} p95={p95:.1} max={max:.1} spans:"
    );
    if sorted.is_empty() {
        line.push_str(" none");
    } else {
        for (label, nanos, calls) in sorted {
            line.push_str(&format!(
                " {label}={:.1}ms/{calls}",
                *nanos as f32 / 1_000_000.0
            ));
        }
    }
    line
}

/// Test-only: drop any open window and open a fresh one so a measurement
/// starts from zero spans and frames.
/// How long the window must hold its size before a resize gesture counts as
/// ended. Matches the drag-window hold above.
pub const RESIZE_GESTURE_HOLD: Duration = Duration::from_millis(300);

/// Always-on record of one window-resize gesture (stint 1224). Logs one info
/// line when the window starts changing size and one when it has held still
/// for [`RESIZE_GESTURE_HOLD`], so a slow resize leaves evidence in the
/// default log without `PLEXI_UI_PROFILE`.
///
/// A frame is a pass whose content rect changed. Each interval between two
/// consecutive resize frames is split three ways:
/// - `app`: Plexi's own `App::logic` + `App::ui` time;
/// - `render`: the rest of eframe's frame CPU (tessellation, paint encoding);
/// - `wait`: everything outside eframe's CPU figure — drawable acquire,
///   submit, present, surface reconfigure and event delivery.
#[derive(Default)]
pub struct ResizeGesture {
    open: Option<Gesture>,
    /// The previous pass, kept only when it was a resize frame.
    prev: Option<FrameMark>,
}

struct Gesture {
    started: Instant,
    last_resize: Instant,
    frames: u32,
    passes: u32,
    samples: Vec<FrameSample>,
}

#[derive(Clone, Copy)]
struct FrameMark {
    start: Instant,
    app_ms: f32,
}

#[derive(Clone, Copy)]
struct FrameSample {
    interval_ms: f32,
    app_ms: f32,
    /// eframe's CPU time for the frame, excluding its drawable and present
    /// waits (`eframe::Frame::info().cpu_usage`). `None` without a painter.
    eframe_cpu_ms: Option<f32>,
}

impl ResizeGesture {
    /// Call once at the start of every `App::logic` pass. `resized` is true
    /// when the window's content rect changed since the previous pass;
    /// `prev_eframe_cpu_s` is `Frame::info().cpu_usage`, which describes the
    /// previous frame. Returns whether a gesture is open after this pass.
    pub fn begin_pass(
        &mut self,
        now: Instant,
        resized: bool,
        prev_eframe_cpu_s: Option<f32>,
    ) -> bool {
        let prev = self.prev.take();
        if resized {
            let gesture = self.open.get_or_insert_with(|| {
                log::info!(target: "plexi::resize", "window resize: started");
                Gesture {
                    started: now,
                    last_resize: now,
                    frames: 0,
                    passes: 0,
                    samples: Vec::new(),
                }
            });
            if let Some(prev) = prev {
                gesture.samples.push(FrameSample {
                    interval_ms: ms(now.saturating_duration_since(prev.start)),
                    app_ms: prev.app_ms,
                    eframe_cpu_ms: prev_eframe_cpu_s.map(|s| s * 1000.0),
                });
            }
            gesture.frames += 1;
            gesture.passes += 1;
            gesture.last_resize = now;
            self.prev = Some(FrameMark {
                start: now,
                app_ms: 0.0,
            });
        } else if let Some(gesture) = self.open.as_mut() {
            gesture.passes += 1;
            if now.saturating_duration_since(gesture.last_resize) >= RESIZE_GESTURE_HOLD {
                let gesture = self.open.take().expect("open gesture");
                log::info!(target: "plexi::resize", "{}", gesture.summary());
            }
        }
        self.open.is_some()
    }

    /// Add `App::logic` or `App::ui` time to the current resize frame.
    pub fn note_app_time(&mut self, elapsed: Duration) {
        if let Some(prev) = self.prev.as_mut() {
            prev.app_ms += ms(elapsed);
        }
    }
}

impl Gesture {
    fn summary(&self) -> String {
        let dur_s = self
            .last_resize
            .saturating_duration_since(self.started)
            .as_secs_f32();
        let fps = if dur_s > 0.0 {
            self.frames.saturating_sub(1) as f32 / dur_s
        } else {
            0.0
        };
        let p95 = |values: Vec<f32>| percentiles(&values).1;
        let (p50, frame_p95, max) = percentiles(
            &self
                .samples
                .iter()
                .map(|s| s.interval_ms)
                .collect::<Vec<_>>(),
        );
        let app_p95 = p95(self.samples.iter().map(|s| s.app_ms).collect());
        let mut line = format!(
            "window resize: ended frames={} passes={} dur={dur_s:.2}s fps={fps:.1} frame_ms p50={p50:.1} p95={frame_p95:.1} max={max:.1} app_ms p95={app_p95:.1}",
            self.frames, self.passes
        );
        let cpu: Option<Vec<(f32, f32, f32)>> = self
            .samples
            .iter()
            .map(|s| s.eframe_cpu_ms.map(|cpu| (s.interval_ms, s.app_ms, cpu)))
            .collect();
        match cpu {
            Some(cpu) if !cpu.is_empty() => {
                let render = p95(cpu
                    .iter()
                    .map(|(_, app, cpu)| (cpu - app).max(0.0))
                    .collect());
                let wait = p95(cpu
                    .iter()
                    .map(|(interval, _, cpu)| (interval - cpu).max(0.0))
                    .collect());
                line.push_str(&format!(" render_ms p95={render:.1} wait_ms p95={wait:.1}"));
            }
            _ => line.push_str(" render_ms p95=n/a wait_ms p95=n/a"),
        }
        line
    }
}

fn ms(d: Duration) -> f32 {
    d.as_secs_f32() * 1000.0
}

#[cfg(test)]
pub(crate) fn reset_measurement() {
    CPU_FRAMES.lock().unwrap().clear();
    set_drag_active(false);
    set_drag_active(true);
}

/// Test-only: print the open window's summary plus render-thread CPU
/// percentiles (CPU excludes scheduler delay on a loaded builder).
#[cfg(test)]
pub(crate) fn print_measurement() {
    let cpu = CPU_FRAMES.lock().unwrap();
    let (p50, p95, max) = percentiles(&cpu);
    println!(
        "UI_THREAD_CPU frames={} p50_ms={p50:.2} p95_ms={p95:.2} max_ms={max:.2}",
        cpu.len()
    );
    let window = WINDOW.lock().unwrap();
    let w = window.as_ref().expect("an open measurement window");
    let (p50, p95, max) = percentiles(&w.frame_ms);
    println!(
        "{}",
        window_summary_line(
            w.frames,
            w.started.elapsed().as_secs_f32(),
            p50,
            p95,
            max,
            &drain_spans()
        )
    );
}

#[cfg(test)]
static CPU_FRAMES: Mutex<Vec<f32>> = Mutex::new(Vec::new());

/// Test-only render-thread CPU clock.
#[cfg(test)]
pub(crate) fn thread_cpu_time() -> Duration {
    let mut value = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `value` is a valid writable timespec; the clock is per-thread.
    let result = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut value) };
    assert_eq!(
        result,
        0,
        "thread CPU clock: {}",
        std::io::Error::last_os_error()
    );
    Duration::new(value.tv_sec as u64, value.tv_nsec as u32)
}

#[cfg(test)]
pub(crate) fn note_cpu_frame(total: Duration) {
    CPU_FRAMES
        .lock()
        .unwrap()
        .push(total.as_secs_f32() * 1000.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_path_never_touches_spans() {
        if enabled() {
            // Skip: PLEXI_UI_PROFILE=1 set in this process's env by another
            // test or CI config — the invariant under test doesn't apply.
            return;
        }
        note_nanos("terminal", 1_000_000);
        let spans = SPANS.lock().unwrap();
        assert!(
            spans.is_empty(),
            "SPANS must stay empty when PLEXI_UI_PROFILE is unset"
        );
    }

    #[test]
    fn should_log_slow_frame_threshold() {
        assert!(!should_log_slow_frame(Duration::from_millis(16)));
        assert!(should_log_slow_frame(Duration::from_millis(17)));
        assert!(should_log_slow_frame(Duration::from_millis(50)));
    }

    #[test]
    fn window_summary_line_format() {
        let spans = vec![
            ("terminal".to_string(), 612_400_000, 261),
            ("sidebar".to_string(), 61_000_000, 87),
        ];
        assert_eq!(
            window_summary_line(87, 1.43, 11.2, 31.4, 48.9, &spans),
            "[UIPROF] drag_window frames=87 dur=1.43s fps=60.8 frame_ms p50=11.2 p95=31.4 max=48.9 spans: terminal=612.4ms/261 sidebar=61.0ms/87"
        );
        assert_eq!(
            window_summary_line(0, 0.0, 0.0, 0.0, 0.0, &[]),
            "[UIPROF] drag_window frames=0 dur=0.00s fps=0.0 frame_ms p50=0.0 p95=0.0 max=0.0 spans: none"
        );
    }

    #[test]
    fn resize_gesture_splits_frame_time_and_closes_after_the_hold() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut g = ResizeGesture::default();
        assert!(!g.begin_pass(at(0), false, None));
        // Three resize frames 20ms apart: 4ms of app, 6ms of eframe CPU.
        for (i, start) in [0u64, 20, 40].into_iter().enumerate() {
            let cpu = (i > 0).then_some(0.006);
            assert!(g.begin_pass(at(start + 100), true, cpu));
            g.note_app_time(Duration::from_millis(1));
            g.note_app_time(Duration::from_millis(3));
        }
        // A pass with no size change inside the hold keeps it open.
        assert!(g.begin_pass(at(200), false, Some(0.006)));
        assert_eq!(
            g.open.as_ref().unwrap().summary(),
            "window resize: ended frames=3 passes=4 dur=0.04s fps=50.0 frame_ms p50=20.0 p95=20.0 max=20.0 app_ms p95=4.0 render_ms p95=2.0 wait_ms p95=14.0"
        );
        assert!(!g.begin_pass(at(140 + 300), false, None));
        assert!(g.open.is_none());
    }

    #[test]
    fn resize_gesture_without_eframe_cpu_reports_no_split() {
        let t0 = Instant::now();
        let mut g = ResizeGesture::default();
        g.begin_pass(t0, true, None);
        g.begin_pass(t0 + Duration::from_millis(16), true, None);
        assert!(g
            .open
            .as_ref()
            .unwrap()
            .summary()
            .ends_with("render_ms p95=n/a wait_ms p95=n/a"));
    }

    #[test]
    fn percentiles_basic() {
        let frame_ms: Vec<f32> = (1..=100).map(|n| n as f32).collect();
        let (p50, p95, max) = percentiles(&frame_ms);
        assert_eq!(p50, 51.0);
        assert_eq!(p95, 95.0);
        assert_eq!(max, 100.0);
        assert_eq!(percentiles(&[]), (0.0, 0.0, 0.0));
    }
}
