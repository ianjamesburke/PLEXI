//! Process-app render transaction diagnostics.
//!
//! Host repaint FPS alone does not prove that a PGAP app is receiving and
//! committing frames at a steady cadence. This tracks the Render -> FrameDone
//! transaction boundary per pane so animation regressions have a concrete log
//! signal.

use std::time::{Duration, Instant};

const FIRST_SUMMARY_WINDOW: Duration = Duration::from_secs(1);
const STEADY_SUMMARY_WINDOW: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub(crate) struct RenderDiagnostics {
    window_started_at: Instant,
    current_frame: Option<InFlightFrame>,
    last_sent_at: Option<Instant>,
    last_committed_at: Option<Instant>,
    summaries_logged: u32,
    sent: u32,
    committed: u32,
    total_send_gap: Duration,
    max_send_gap: Duration,
    slow_send_gaps: u32,
    total_rtt: Duration,
    max_rtt: Duration,
    total_commit_gap: Duration,
    max_commit_gap: Duration,
    slow_commit_gaps: u32,
}

#[derive(Debug)]
struct InFlightFrame {
    frame_id: u64,
    sent_at: Instant,
}

impl RenderDiagnostics {
    pub(crate) fn new() -> Self {
        Self {
            window_started_at: Instant::now(),
            current_frame: None,
            last_sent_at: None,
            last_committed_at: None,
            summaries_logged: 0,
            sent: 0,
            committed: 0,
            total_send_gap: Duration::ZERO,
            max_send_gap: Duration::ZERO,
            slow_send_gaps: 0,
            total_rtt: Duration::ZERO,
            max_rtt: Duration::ZERO,
            total_commit_gap: Duration::ZERO,
            max_commit_gap: Duration::ZERO,
            slow_commit_gaps: 0,
        }
    }

    pub(crate) fn record_render_sent(
        &mut self,
        frame_id: u64,
        target_interval: Option<Duration>,
        now: Instant,
    ) {
        self.sent = self.sent.saturating_add(1);
        if let Some(last_sent_at) = self.last_sent_at {
            let gap = now.saturating_duration_since(last_sent_at);
            self.total_send_gap += gap;
            self.max_send_gap = self.max_send_gap.max(gap);
            if target_interval.is_some_and(|interval| gap > interval.mul_f32(1.5)) {
                self.slow_send_gaps = self.slow_send_gaps.saturating_add(1);
            }
        }
        self.last_sent_at = Some(now);
        self.current_frame = Some(InFlightFrame {
            frame_id,
            sent_at: now,
        });
    }

    pub(crate) fn record_frame_committed(
        &mut self,
        app_id: &str,
        frame_id: u64,
        target_interval: Option<Duration>,
        now: Instant,
    ) {
        self.committed = self.committed.saturating_add(1);

        if let Some(in_flight) = self.current_frame.take() {
            if in_flight.frame_id == frame_id {
                let rtt = now.saturating_duration_since(in_flight.sent_at);
                self.total_rtt += rtt;
                self.max_rtt = self.max_rtt.max(rtt);
            }
        }

        if let Some(last_committed_at) = self.last_committed_at {
            let gap = now.saturating_duration_since(last_committed_at);
            self.total_commit_gap += gap;
            self.max_commit_gap = self.max_commit_gap.max(gap);
            if target_interval.is_some_and(|interval| gap > interval.mul_f32(1.5)) {
                self.slow_commit_gaps = self.slow_commit_gaps.saturating_add(1);
            }
        }
        self.last_committed_at = Some(now);

        if now.saturating_duration_since(self.window_started_at) >= self.current_window() {
            self.log_summary(app_id, target_interval, now, "interval");
            self.reset_window(now);
        }
    }

    pub(crate) fn flush_if_nonempty(
        &mut self,
        app_id: &str,
        target_interval: Option<Duration>,
        now: Instant,
        reason: &'static str,
    ) {
        if self.sent == 0 && self.committed == 0 {
            return;
        }
        self.log_summary(app_id, target_interval, now, reason);
        self.reset_window(now);
    }

    fn current_window(&self) -> Duration {
        if self.summaries_logged == 0 {
            FIRST_SUMMARY_WINDOW
        } else {
            STEADY_SUMMARY_WINDOW
        }
    }

    fn log_summary(
        &self,
        app_id: &str,
        target_interval: Option<Duration>,
        now: Instant,
        reason: &'static str,
    ) {
        let elapsed = now
            .saturating_duration_since(self.window_started_at)
            .as_secs_f64();
        let app_fps = if elapsed > 0.0 {
            self.committed as f64 / elapsed
        } else {
            0.0
        };
        let send_gap_count = self.sent.saturating_sub(1);
        let avg_send_gap_ms = avg_ms(self.total_send_gap, send_gap_count);
        let avg_rtt_ms = avg_ms(self.total_rtt, self.committed);
        let commit_gap_count = self.committed.saturating_sub(1);
        let avg_gap_ms = avg_ms(self.total_commit_gap, commit_gap_count);
        let target_ms = target_interval
            .map(|interval| format!("{:.1}", interval.as_secs_f64() * 1000.0))
            .unwrap_or_else(|| "none".to_string());

        log::info!(
            "ProcessApp[{app_id}]: render_diag reason={reason} sent={} committed={} window={elapsed:.1}s app_fps={app_fps:.1} target_ms={} avg_send_gap_ms={avg_send_gap_ms:.1} max_send_gap_ms={:.1} slow_send_gaps={} avg_rtt_ms={avg_rtt_ms:.1} max_rtt_ms={:.1} avg_commit_gap_ms={avg_gap_ms:.1} max_commit_gap_ms={:.1} slow_commit_gaps={}",
            self.sent,
            self.committed,
            target_ms,
            self.max_send_gap.as_secs_f64() * 1000.0,
            self.slow_send_gaps,
            self.max_rtt.as_secs_f64() * 1000.0,
            self.max_commit_gap.as_secs_f64() * 1000.0,
            self.slow_commit_gaps,
        );
    }

    fn reset_window(&mut self, now: Instant) {
        self.window_started_at = now;
        self.summaries_logged = self.summaries_logged.saturating_add(1);
        self.sent = 0;
        self.committed = 0;
        self.total_send_gap = Duration::ZERO;
        self.max_send_gap = Duration::ZERO;
        self.slow_send_gaps = 0;
        self.total_rtt = Duration::ZERO;
        self.max_rtt = Duration::ZERO;
        self.total_commit_gap = Duration::ZERO;
        self.max_commit_gap = Duration::ZERO;
        self.slow_commit_gaps = 0;
    }
}

fn avg_ms(total: Duration, count: u32) -> f64 {
    if count == 0 {
        0.0
    } else {
        total.as_secs_f64() * 1000.0 / f64::from(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_committed_frame_cadence() {
        let start = Instant::now();
        let mut diag = RenderDiagnostics::new();

        diag.record_render_sent(1, Some(Duration::from_millis(16)), start);
        diag.record_frame_committed("test", 1, Some(Duration::from_millis(16)), start);
        diag.record_render_sent(
            2,
            Some(Duration::from_millis(16)),
            start + Duration::from_millis(16),
        );
        diag.record_frame_committed(
            "test",
            2,
            Some(Duration::from_millis(16)),
            start + Duration::from_millis(50),
        );

        assert_eq!(diag.sent, 2);
        assert_eq!(diag.committed, 2);
        assert_eq!(diag.slow_commit_gaps, 1);
        assert_eq!(diag.max_commit_gap, Duration::from_millis(50));
    }
}
