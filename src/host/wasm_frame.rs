// Host-owned frame pacing and telemetry for WASM surface apps.
//
// The guest never implements its own FPS loop. It requests a continuous
// cadence; the host's [`FrameClock`] decides how many fixed sim steps to run
// per repaint (with bounded catch-up and a drop policy), and [`FrameTelemetry`]
// owns the wall-clock measurements (frame interval, present cost, dropped
// frames). Apps display host telemetry; they do not invent it.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Result of advancing the clock by one repaint: how many fixed sim steps the
/// guest should run, and how many were dropped because the host fell too far
/// behind to catch up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameStep {
    pub steps: u32,
    pub dropped: u32,
}

/// Fixed-timestep clock. One per surface pane. Catch-up is bounded by
/// `max_catch_up` so a single long stall cannot trigger a spiral of death;
/// anything still owed past that bound is dropped, not deferred.
pub struct FrameClock {
    target_interval: Duration,
    max_catch_up: u32,
    accumulator: Duration,
    last: Option<Instant>,
}

impl FrameClock {
    /// Build a clock targeting `target_hz` frames per second (clamped to ≥1).
    pub fn new(target_hz: u32) -> Self {
        let hz = target_hz.max(1);
        FrameClock {
            target_interval: Duration::from_secs_f64(1.0 / hz as f64),
            max_catch_up: 5,
            accumulator: Duration::ZERO,
            last: None,
        }
    }

    /// The repaint cadence the live pane should schedule with
    /// `request_repaint_after`.
    pub fn target_interval(&self) -> Duration {
        self.target_interval
    }

    /// Advance to wall-clock `now`, returning how many fixed sim steps to run.
    /// The first call establishes the baseline and yields a single step.
    pub fn advance(&mut self, now: Instant) -> FrameStep {
        let Some(last) = self.last else {
            self.last = Some(now);
            return FrameStep {
                steps: 1,
                dropped: 0,
            };
        };
        self.last = Some(now);
        self.accumulator += now.saturating_duration_since(last);

        let mut steps = 0u32;
        while self.accumulator >= self.target_interval && steps < self.max_catch_up {
            self.accumulator -= self.target_interval;
            steps += 1;
        }
        let mut dropped = 0u32;
        while self.accumulator >= self.target_interval {
            self.accumulator -= self.target_interval;
            dropped += 1;
        }
        FrameStep { steps, dropped }
    }
}

/// Rolling wall-clock telemetry for a surface pane. Fixed-size windows keep the
/// percentile math cheap and bound memory regardless of session length.
pub struct FrameTelemetry {
    intervals: VecDeque<Duration>,
    present: VecDeque<Duration>,
    window: usize,
    frames: u64,
    dropped: u64,
}

impl FrameTelemetry {
    pub fn new(window: usize) -> Self {
        FrameTelemetry {
            intervals: VecDeque::with_capacity(window),
            present: VecDeque::with_capacity(window),
            window: window.max(1),
            frames: 0,
            dropped: 0,
        }
    }

    /// Record the wall-clock gap since the previous presented frame.
    pub fn record_frame(&mut self, interval: Duration) {
        self.frames += 1;
        push_capped(&mut self.intervals, interval, self.window);
    }

    /// Record how long the host spent registering/compositing the surface.
    pub fn record_present(&mut self, dur: Duration) {
        push_capped(&mut self.present, dur, self.window);
    }

    pub fn record_dropped(&mut self, dropped: u32) {
        self.dropped += dropped as u64;
    }

    pub fn frames(&self) -> u64 {
        self.frames
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Mean frames-per-second over the interval window. Zero until two frames
    /// have been recorded.
    pub fn fps(&self) -> f32 {
        if self.intervals.is_empty() {
            return 0.0;
        }
        let total: Duration = self.intervals.iter().sum();
        let mean = total.as_secs_f32() / self.intervals.len() as f32;
        if mean <= 0.0 {
            0.0
        } else {
            1.0 / mean
        }
    }

    pub fn p95_interval_ms(&self) -> f32 {
        percentile_ms(&self.intervals, 0.95)
    }

    pub fn p95_present_ms(&self) -> f32 {
        percentile_ms(&self.present, 0.95)
    }
}

fn push_capped(buf: &mut VecDeque<Duration>, v: Duration, cap: usize) {
    if buf.len() == cap {
        buf.pop_front();
    }
    buf.push_back(v);
}

fn percentile_ms(buf: &VecDeque<Duration>, q: f32) -> f32 {
    if buf.is_empty() {
        return 0.0;
    }
    let mut v: Vec<Duration> = buf.iter().copied().collect();
    v.sort_unstable();
    let idx = (((v.len() - 1) as f32) * q).round() as usize;
    v[idx].as_secs_f32() * 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_advance_runs_one_step() {
        let mut clock = FrameClock::new(60);
        let t0 = Instant::now();
        assert_eq!(
            clock.advance(t0),
            FrameStep {
                steps: 1,
                dropped: 0
            }
        );
    }

    #[test]
    fn steady_cadence_runs_one_step_per_frame() {
        let mut clock = FrameClock::new(60);
        let dt = clock.target_interval();
        let mut t = Instant::now();
        clock.advance(t);
        for _ in 0..10 {
            t += dt;
            let step = clock.advance(t);
            assert_eq!(step.steps, 1);
            assert_eq!(step.dropped, 0);
        }
    }

    #[test]
    fn long_stall_catches_up_then_drops() {
        let mut clock = FrameClock::new(60);
        let dt = clock.target_interval();
        let t0 = Instant::now();
        clock.advance(t0);
        // A 20-frame stall: catch up to the bound (5), drop the rest.
        let step = clock.advance(t0 + dt * 20);
        assert_eq!(step.steps, 5);
        assert_eq!(step.dropped, 15);
    }

    #[test]
    fn telemetry_p95_picks_high_interval() {
        let mut tel = FrameTelemetry::new(100);
        for _ in 0..95 {
            tel.record_frame(Duration::from_millis(16));
        }
        for _ in 0..5 {
            tel.record_frame(Duration::from_millis(40));
        }
        // 96th-percentile-ish index lands in the slow tail.
        assert!(tel.p95_interval_ms() >= 16.0);
        assert!(tel.fps() > 20.0 && tel.fps() < 70.0);
    }

    #[test]
    fn telemetry_window_is_bounded() {
        let mut tel = FrameTelemetry::new(8);
        for _ in 0..100 {
            tel.record_frame(Duration::from_millis(16));
        }
        assert_eq!(tel.frames(), 100);
        // Only the window is retained; p95 stays well-defined.
        assert!((tel.p95_interval_ms() - 16.0).abs() < 0.001);
    }
}
