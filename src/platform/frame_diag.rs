//! Frame repaint-cause diagnostics (#2019).
//!
//! Plexi has many independent `request_repaint()` / `request_repaint_after()`
//! paths. Each recurring/timed/background repaint site calls [`note`] with its
//! [`RepaintCause`] so the per-window summary in `PlexiApp::update` can report
//! the top repaint causes and the observed frame cadence under the
//! `plexi::frame_diag` log target. One-shot input-driven repaints are covered
//! by [`RepaintCause::UserInput`], noted once per frame that carries raw input
//! events.

use std::sync::atomic::{AtomicU32, Ordering};

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
