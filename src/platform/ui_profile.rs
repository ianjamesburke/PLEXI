//! Per-frame `App::ui` cost profiling, attributed by pane kind (stint 0731).
//!
//! Stint 0716's `[FREEZE]` / `logic_drain:` instrumentation lives in
//! `App::logic` and is structurally blind to drag-resize stutter, which
//! happens in `App::ui`. This module measures where `ui()` frame time goes
//! during an active drag or window resize, gated behind `PLEXI_UI_PROFILE=1`
//! so it costs nothing on the default hot path.

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
    fn percentiles_basic() {
        let frame_ms: Vec<f32> = (1..=100).map(|n| n as f32).collect();
        let (p50, p95, max) = percentiles(&frame_ms);
        assert_eq!(p50, 51.0);
        assert_eq!(p95, 95.0);
        assert_eq!(max, 100.0);
        assert_eq!(percentiles(&[]), (0.0, 0.0, 0.0));
    }
}
