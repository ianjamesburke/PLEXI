//! Frame repaint-cause diagnostics (#2019).
//!
//! Plexi has many independent `request_repaint()` / `request_repaint_after()`
//! paths. Each recurring/timed/background repaint site calls [`note`] with its
//! [`RepaintCause`] so the per-window summary in `PlexiApp::update` can report
//! the top repaint causes and the observed frame cadence under the
//! `plexi::frame_diag` log target. One-shot input-driven repaints are covered
//! by [`RepaintCause::UserInput`], noted once per frame that carries raw input
//! events.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{LazyLock, Mutex};

/// Why a repaint was requested. Every variant maps to a stable snake_case
/// label via [`RepaintCause::label`] for grep-able log output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepaintCause {
    /// ProcessApp 100ms idle poll for async responses (per visible pane).
    AppIdlePoll,
    /// ProcessApp click forwarding (immediate repaint + awaiting-frame echo).
    AppClick,
    /// App-requested `ScheduleRender { after_ms }`.
    AppScheduleRender,
    /// Pointer-tracking apps at ~60fps while the pointer moves; also terminal
    /// drag-selection auto-scroll.
    PointerTracking,
    /// Focused terminal cursor blink.
    TerminalCursorBlink,
    /// Terminal search pill query-cursor blink.
    TerminalSearchBlink,
    /// PTY output event from a terminal backend thread.
    TerminalPtyOutput,
    /// Skeleton loader animation (~20fps).
    SkeletonLoader,
    /// Pane swap animation.
    PaneSwapAnim,
    /// Directional edge pulse after focus move.
    EdgePulse,
    /// CRT feature effect (~60fps while enabled).
    CrtEffect,
    /// Image cache load completion (background thread).
    ImageCacheCompletion,
    /// 100ms poll while a file drag hovers the window.
    FileDragPoll,
    /// 100ms widget pulse/poll animations (copy feedback, quick-note loading).
    WidgetPulse,
    /// Quit/delete confirm overlay 100ms timeout poll.
    QuitConfirm,
    /// Frame carried raw input events (covers one-shot event-driven repaints).
    UserInput,
}

/// All variants, in declaration order. Indexes into [`COUNTERS`].
const ALL_CAUSES: [RepaintCause; 16] = [
    RepaintCause::AppIdlePoll,
    RepaintCause::AppClick,
    RepaintCause::AppScheduleRender,
    RepaintCause::PointerTracking,
    RepaintCause::TerminalCursorBlink,
    RepaintCause::TerminalSearchBlink,
    RepaintCause::TerminalPtyOutput,
    RepaintCause::SkeletonLoader,
    RepaintCause::PaneSwapAnim,
    RepaintCause::EdgePulse,
    RepaintCause::CrtEffect,
    RepaintCause::ImageCacheCompletion,
    RepaintCause::FileDragPoll,
    RepaintCause::WidgetPulse,
    RepaintCause::QuitConfirm,
    RepaintCause::UserInput,
];

static COUNTERS: [AtomicU32; ALL_CAUSES.len()] = [const { AtomicU32::new(0) }; ALL_CAUSES.len()];

impl RepaintCause {
    /// Stable snake_case label for log output.
    pub fn label(self) -> &'static str {
        match self {
            RepaintCause::AppIdlePoll => "app_idle_poll",
            RepaintCause::AppClick => "app_click",
            RepaintCause::AppScheduleRender => "app_schedule_render",
            RepaintCause::PointerTracking => "pointer_tracking",
            RepaintCause::TerminalCursorBlink => "terminal_cursor_blink",
            RepaintCause::TerminalSearchBlink => "terminal_search_blink",
            RepaintCause::TerminalPtyOutput => "terminal_pty_output",
            RepaintCause::SkeletonLoader => "skeleton_loader",
            RepaintCause::PaneSwapAnim => "pane_swap_anim",
            RepaintCause::EdgePulse => "edge_pulse",
            RepaintCause::CrtEffect => "crt_effect",
            RepaintCause::ImageCacheCompletion => "image_cache_completion",
            RepaintCause::FileDragPoll => "file_drag_poll",
            RepaintCause::WidgetPulse => "widget_pulse",
            RepaintCause::QuitConfirm => "quit_confirm",
            RepaintCause::UserInput => "user_input",
        }
    }
}

/// Record one repaint request for `cause`. Callable from any thread
/// (image cache and PTY readers note from background threads).
pub fn note(cause: RepaintCause) {
    COUNTERS[cause as usize].fetch_add(1, Ordering::Relaxed);
}

/// Ground-truth repaint causes from egui (#2209): per sample window, count of
/// frames each `file:line` repaint-request site appeared in. Cross-checks the
/// self-reported [`note`] counters above — egui sees every
/// `request_repaint()` caller, including sites Plexi never instrumented.
static EGUI_CAUSES: LazyLock<Mutex<HashMap<String, u32>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Accumulate one frame's `ctx.repaint_causes()` (the PREVIOUS pass's
/// requests). Causes are dedup'd by `file:line` within the pass — the same
/// site can request a repaint several times per pass, but "how many frames
/// did this site wake" is the useful idle-CPU signal, so we count per-frame
/// presence, not raw request volume.
pub fn note_egui_causes(causes: &[egui::RepaintCause]) {
    if causes.is_empty() {
        return;
    }
    let mut seen: HashSet<String> = HashSet::new();
    let mut map = EGUI_CAUSES.lock().unwrap();
    for cause in causes {
        let key = format!("{}:{}", cause.file, cause.line);
        if seen.insert(key.clone()) {
            *map.entry(key).or_insert(0) += 1;
        }
    }
}

/// Returns all egui cause counts sorted descending (ties alphabetical by
/// `file:line` for stable output) and clears the accumulator.
pub fn egui_snapshot_and_reset() -> Vec<(String, u32)> {
    let mut map = EGUI_CAUSES.lock().unwrap();
    let mut counts: Vec<(String, u32)> = map.drain().collect();
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    counts
}

/// Formats the egui ground-truth line logged under the same window as
/// [`summary_line`], truncated to the top `top_n` sites, e.g.
/// `egui_causes: crates/egui/src/context.rs:1234=601 src/app/mod.rs:99=12 (+3 more)`.
pub fn egui_summary_line(counts: &[(String, u32)], top_n: usize) -> String {
    let mut line = String::from("egui_causes:");
    if counts.is_empty() {
        line.push_str(" none");
        return line;
    }
    for (site, n) in counts.iter().take(top_n) {
        line.push_str(&format!(" {site}={n}"));
    }
    if counts.len() > top_n {
        line.push_str(&format!(" (+{} more)", counts.len() - top_n));
    }
    line
}

/// Returns all nonzero counts sorted descending and zeroes every counter.
/// Ties keep variant declaration order (stable sort).
pub fn snapshot_and_reset() -> Vec<(RepaintCause, u32)> {
    let mut counts: Vec<(RepaintCause, u32)> = ALL_CAUSES
        .iter()
        .map(|&cause| (cause, COUNTERS[cause as usize].swap(0, Ordering::Relaxed)))
        .filter(|&(_, n)| n > 0)
        .collect();
    counts.sort_by(|a, b| b.1.cmp(&a.1));
    counts
}

/// Formats one summary line, e.g.
/// `frames=102 window=10.0s fps=10.2 causes: app_idle_poll=95 user_input=12`.
pub fn summary_line(frames: u32, elapsed_secs: f32, counts: &[(RepaintCause, u32)]) -> String {
    let fps = if elapsed_secs > 0.0 {
        frames as f32 / elapsed_secs
    } else {
        0.0
    };
    let mut line = format!("frames={frames} window={elapsed_secs:.1}s fps={fps:.1} causes:");
    if counts.is_empty() {
        line.push_str(" none");
    } else {
        for (cause, n) in counts {
            line.push_str(&format!(" {}={n}", cause.label()));
        }
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::Mutex;

    /// Counters are process-global and harness tests drive real frames in
    /// parallel, so: (1) frame_diag tests serialize against each other, and
    /// (2) assertions only touch causes no harness frame ever notes
    /// (EdgePulse, CrtEffect, SkeletonLoader).
    static SERIAL: Mutex<()> = Mutex::new(());

    fn count_of(counts: &[(RepaintCause, u32)], cause: RepaintCause) -> Option<u32> {
        counts.iter().find(|(c, _)| *c == cause).map(|&(_, n)| n)
    }

    #[test]
    fn note_increments_and_snapshot_resets() {
        let _guard = SERIAL.lock().unwrap();
        snapshot_and_reset();

        note(RepaintCause::EdgePulse);
        note(RepaintCause::EdgePulse);
        note(RepaintCause::EdgePulse);

        let counts = snapshot_and_reset();
        assert_eq!(count_of(&counts, RepaintCause::EdgePulse), Some(3));

        // Counter was zeroed by the snapshot.
        let counts = snapshot_and_reset();
        assert_eq!(count_of(&counts, RepaintCause::EdgePulse), None);
    }

    #[test]
    fn snapshot_sorted_descending() {
        let _guard = SERIAL.lock().unwrap();
        snapshot_and_reset();

        for _ in 0..5 {
            note(RepaintCause::SkeletonLoader);
        }
        for _ in 0..2 {
            note(RepaintCause::EdgePulse);
        }
        note(RepaintCause::CrtEffect);

        let counts = snapshot_and_reset();
        for pair in counts.windows(2) {
            assert!(pair[0].1 >= pair[1].1, "not descending: {counts:?}");
        }
        let pos = |cause| counts.iter().position(|(c, _)| *c == cause).unwrap();
        assert!(pos(RepaintCause::SkeletonLoader) < pos(RepaintCause::EdgePulse));
        assert!(pos(RepaintCause::EdgePulse) < pos(RepaintCause::CrtEffect));
        assert_eq!(count_of(&counts, RepaintCause::SkeletonLoader), Some(5));
        assert_eq!(count_of(&counts, RepaintCause::EdgePulse), Some(2));
        assert_eq!(count_of(&counts, RepaintCause::CrtEffect), Some(1));
    }

    #[test]
    fn summary_line_format() {
        let counts = vec![
            (RepaintCause::AppIdlePoll, 95),
            (RepaintCause::UserInput, 12),
        ];
        assert_eq!(
            summary_line(102, 10.0, &counts),
            "frames=102 window=10.0s fps=10.2 causes: app_idle_poll=95 user_input=12"
        );
        assert_eq!(
            summary_line(0, 10.0, &[]),
            "frames=0 window=10.0s fps=0.0 causes: none"
        );
        // Zero-length window must not divide by zero.
        assert_eq!(
            summary_line(5, 0.0, &[]),
            "frames=5 window=0.0s fps=0.0 causes: none"
        );
    }

    fn egui_cause(file: &'static str, line: u32) -> egui::RepaintCause {
        egui::RepaintCause {
            file,
            line,
            reason: std::borrow::Cow::Borrowed("test"),
        }
    }

    /// Harness tests run real frames in parallel and feed real sites into the
    /// global egui accumulator; assertions only look at synthetic
    /// `tests/fake_*.rs` keys no real frame can produce.
    fn fake_only(counts: &[(String, u32)]) -> Vec<(String, u32)> {
        counts
            .iter()
            .filter(|(k, _)| k.starts_with("tests/fake_"))
            .cloned()
            .collect()
    }

    #[test]
    fn egui_causes_dedup_within_pass_and_accumulate_across_passes() {
        let _guard = SERIAL.lock().unwrap();
        egui_snapshot_and_reset();

        // Pass 1: the same site requests twice — counts once for the frame.
        note_egui_causes(&[
            egui_cause("tests/fake_a.rs", 10),
            egui_cause("tests/fake_a.rs", 10),
            egui_cause("tests/fake_b.rs", 20),
        ]);
        // Pass 2: only fake_a.rs wakes again.
        note_egui_causes(&[egui_cause("tests/fake_a.rs", 10)]);
        // Empty pass is a no-op.
        note_egui_causes(&[]);

        let counts = fake_only(&egui_snapshot_and_reset());
        assert_eq!(
            counts,
            vec![
                ("tests/fake_a.rs:10".to_string(), 2),
                ("tests/fake_b.rs:20".to_string(), 1),
            ]
        );

        // Snapshot cleared the accumulator.
        assert!(fake_only(&egui_snapshot_and_reset()).is_empty());
    }

    #[test]
    fn egui_snapshot_sorted_descending_then_alphabetical() {
        let _guard = SERIAL.lock().unwrap();
        egui_snapshot_and_reset();

        for _ in 0..3 {
            note_egui_causes(&[egui_cause("tests/fake_z.rs", 1)]);
        }
        note_egui_causes(&[
            egui_cause("tests/fake_m.rs", 5),
            egui_cause("tests/fake_a.rs", 9),
        ]);

        let counts = fake_only(&egui_snapshot_and_reset());
        assert_eq!(
            counts,
            vec![
                ("tests/fake_z.rs:1".to_string(), 3),
                ("tests/fake_a.rs:9".to_string(), 1),
                ("tests/fake_m.rs:5".to_string(), 1),
            ]
        );
    }

    #[test]
    fn egui_summary_line_format() {
        assert_eq!(egui_summary_line(&[], 8), "egui_causes: none");

        let counts = vec![
            ("src/a.rs:10".to_string(), 95),
            ("src/b.rs:20".to_string(), 12),
        ];
        assert_eq!(
            egui_summary_line(&counts, 8),
            "egui_causes: src/a.rs:10=95 src/b.rs:20=12"
        );

        // Truncation appends the overflow count.
        let many: Vec<(String, u32)> = (0..10)
            .map(|i| (format!("src/f.rs:{i}"), 10 - i))
            .collect();
        let line = egui_summary_line(&many, 8);
        assert!(line.ends_with(" (+2 more)"), "got: {line}");
        assert!(line.contains("src/f.rs:0=10"));
        assert!(!line.contains("src/f.rs:8="));
    }

    #[test]
    fn labels_unique_and_complete() {
        let labels: HashSet<&'static str> = ALL_CAUSES.iter().map(|c| c.label()).collect();
        assert_eq!(labels.len(), ALL_CAUSES.len());
        for label in labels {
            assert!(!label.is_empty());
            assert!(
                label.chars().all(|ch| ch.is_ascii_lowercase() || ch == '_'),
                "label not snake_case: {label}"
            );
        }
    }
}
