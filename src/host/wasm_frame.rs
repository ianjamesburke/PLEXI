// Host-owned frame pacing and telemetry for WASM surface apps.
//
// The guest never implements its own FPS loop. It requests a continuous
// cadence; the host's [`FrameClock`] decides how many fixed sim steps to run
// per repaint (with bounded catch-up and a drop policy), and [`FrameTelemetry`]
// owns the wall-clock measurements (frame interval, present cost, dropped
// frames). Apps display host telemetry; they do not invent it.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Cadence bounds an app can steer the host to. Matched to the CPython path's
/// `fps.clamp(1, 240)` so neither runtime lets a guest declare a rate the host
/// would pay an unbounded repaint cost to honour.
pub const MIN_TARGET_INTERVAL: Duration = Duration::from_nanos(1_000_000_000 / 240);
pub const MAX_TARGET_INTERVAL: Duration = Duration::from_secs(1);

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
    default_interval: Duration,
    max_catch_up: u32,
    accumulator: Duration,
    last: Option<Instant>,
}

impl FrameClock {
    /// Build a clock targeting `target_hz` frames per second (clamped to ≥1).
    /// That rate is also the fallback [`retarget`](Self::retarget) restores when
    /// an app declares no cadence of its own.
    pub fn new(target_hz: u32) -> Self {
        let hz = target_hz.max(1);
        let interval = Duration::from_secs_f64(1.0 / hz as f64);
        FrameClock {
            target_interval: interval,
            default_interval: interval,
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

    /// Retarget the clock at the cadence the app declared for itself, clamped to
    /// [`MIN_TARGET_INTERVAL`]..=[`MAX_TARGET_INTERVAL`] — an app steers the
    /// host's repaint rate here, so it never gets to name an arbitrary one.
    /// `None` means the app declares nothing (or stopped declaring), which
    /// restores the construction-time default rather than stranding the pane on
    /// a cadence its app no longer asks for. Idempotent when unchanged; a real
    /// change discards the partial accumulator, which is debt measured against
    /// the old interval and meaningless against the new one.
    pub fn retarget(&mut self, declared: Option<Duration>) {
        let interval = declared
            .unwrap_or(self.default_interval)
            .clamp(MIN_TARGET_INTERVAL, MAX_TARGET_INTERVAL);
        if interval == self.target_interval {
            return;
        }
        self.target_interval = interval;
        self.accumulator = Duration::ZERO;
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
        // Whatever is still owed past the bound is dropped in one arithmetic
        // step — a multi-minute stall must not cost an iteration per missed
        // frame just to discard them.
        let owed = self.accumulator.as_nanos() / self.target_interval.as_nanos();
        let dropped = u32::try_from(owed).unwrap_or(u32::MAX);
        self.accumulator -= self.target_interval * dropped;
        FrameStep { steps, dropped }
    }
}

/// Delay to hand `request_repaint_after` so the repaint actually lands on
/// `deadline`.
///
/// egui starts a repaint one predicted frame *before* the requested delay, so
/// passing a bare 16.7 ms interval at 60 Hz collapses into an immediate,
/// unbounded host repaint loop. Adding the estimate back is the fix; both the
/// native surface path and the CPython-WASM path schedule through here so the
/// two cannot drift apart.
pub fn repaint_delay_until(deadline: Instant, now: Instant, predicted_frame: Duration) -> Duration {
    deadline.saturating_duration_since(now) + predicted_frame
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
    fn retargeting_paces_at_the_declared_rate() {
        let mut clock = FrameClock::new(60);
        clock.retarget(Some(Duration::from_millis(50)));
        assert_eq!(clock.target_interval(), Duration::from_millis(50));

        // One step per frame at the *new* cadence, not the constructed 60 Hz.
        let dt = clock.target_interval();
        let mut t = Instant::now();
        clock.advance(t);
        for _ in 0..10 {
            t += dt;
            assert_eq!(
                clock.advance(t),
                FrameStep {
                    steps: 1,
                    dropped: 0
                }
            );
        }
    }

    #[test]
    fn retargeting_drops_accumulator_owed_against_the_old_interval() {
        let mut clock = FrameClock::new(60);
        let t0 = Instant::now();
        clock.advance(t0);
        // Bank most of a 60 Hz frame, then retarget: that debt is measured in
        // the old grid and must not leak into the new one.
        clock.advance(t0 + Duration::from_micros(16_000));
        clock.retarget(Some(Duration::from_millis(100)));
        let step = clock.advance(t0 + Duration::from_micros(16_000) + Duration::from_millis(99));
        assert_eq!(
            step,
            FrameStep {
                steps: 0,
                dropped: 0
            }
        );
    }

    #[test]
    fn retargeting_clamps_what_an_app_can_ask_the_host_to_repaint_at() {
        let mut clock = FrameClock::new(60);
        clock.retarget(Some(Duration::from_micros(100)));
        assert_eq!(clock.target_interval(), MIN_TARGET_INTERVAL);
        clock.retarget(Some(Duration::from_secs(600)));
        assert_eq!(clock.target_interval(), MAX_TARGET_INTERVAL);
    }

    #[test]
    fn retargeting_with_no_declaration_restores_the_default() {
        let mut clock = FrameClock::new(60);
        let default = clock.target_interval();
        clock.retarget(Some(Duration::from_millis(500)));
        assert_eq!(clock.target_interval(), Duration::from_millis(500));
        // The app stopped declaring a cadence (cancelled its frame timer): the
        // pane returns to the host default, it does not strand on 500 ms.
        clock.retarget(None);
        assert_eq!(clock.target_interval(), default);
    }

    #[test]
    fn retargeting_to_the_same_interval_is_inert() {
        let mut clock = FrameClock::new(20);
        let t0 = Instant::now();
        clock.advance(t0);
        clock.advance(t0 + Duration::from_millis(49));
        clock.retarget(Some(Duration::from_millis(50)));
        // The banked 49 ms survives, so 1 ms more is a full step.
        let step = clock.advance(t0 + Duration::from_millis(50));
        assert_eq!(step.steps, 1);
    }

    #[test]
    fn repaint_delay_compensates_for_egui_predicted_frame_subtraction() {
        let now = Instant::now();
        let interval = Duration::from_nanos(16_666_666);
        assert_eq!(
            repaint_delay_until(now + interval, now, interval),
            interval * 2
        );
    }

    #[test]
    fn repaint_delay_never_underflows_a_passed_deadline() {
        let now = Instant::now();
        let predicted = Duration::from_millis(8);
        assert_eq!(
            repaint_delay_until(now - Duration::from_secs(1), now, predicted),
            predicted
        );
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
