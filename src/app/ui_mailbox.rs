//! UiMailbox (stint 0507) — the one host-owned seam for "enqueue work for the
//! UI thread and guarantee a servicing frame".
//!
//! Every off-UI-thread producer (notify-socket IPC, event subscribe/publish,
//! host MCP, filesystem watchers, the updater, the Finder service) used to hold
//! a raw `mpsc::Sender` plus its own repaint knowledge — and each had to get
//! the egui wake math right independently. A producer that got it wrong (raw
//! `request_repaint()`, zero delay) could have its wake rejected by eframe's
//! stale-pass guard and strand queued work for minutes on an idle host. The
//! mailbox owns both halves: `send` enqueues *and* wakes, receivers advance a
//! serviced generation for free, so no call site carries repaint knowledge.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Wakes the UI thread so it produces at least one servicing frame. Object-safe
/// so tests can substitute a deterministic recorder for the egui adapter.
pub(crate) trait UiWake: Send + Sync {
    fn wake(&self, source: &'static str);
}

/// Wake the UI thread for an external request that arrived off the UI thread.
///
/// The repaint request is load-bearing: a fully idle Plexi produces zero
/// frames, and the pane-IPC channel is only drained during a frame. Without
/// the wake, a CLI request against an idle instance sits queued until the
/// user touches the window (#s13 perf batch regression).
///
/// The callback delay must stay nonzero after egui advances delayed repaint
/// requests by the predicted frame time. Add `predicted_dt` here so eframe
/// receives a one-shot delayed repaint instead of a `RepaintNow` event whose
/// pass-number freshness guard can reject it during startup/multipass layout.
/// One prompt pass is all an external arrival needs; handlers that change
/// visible state mark their own frames dirty. Keep every socket/MCP producer
/// on this seam: a raw `request_repaint()` is vulnerable to eframe's stale-pass
/// guard when the producer races a multipass frame.
/// This wake only works if the process itself is wakeable — see
/// `platform::app_nap::disable_app_nap` for the App Nap exemption that keeps
/// macOS from deferring these cross-thread wakeups on an idle host.
pub(crate) const IPC_WAKE_DELAY: Duration = Duration::from_millis(1);

/// Production `UiWake`: the egui repaint math documented on [`IPC_WAKE_DELAY`].
pub(crate) struct EguiWake(egui::Context);

impl EguiWake {
    pub(crate) fn new(ctx: egui::Context) -> Self {
        Self(ctx)
    }
}

impl UiWake for EguiWake {
    fn wake(&self, source: &'static str) {
        let predicted_frame_time =
            Duration::try_from_secs_f32(self.0.input(|input| input.predicted_dt))
                .unwrap_or_default();
        let requested_delay = predicted_frame_time.saturating_add(IPC_WAKE_DELAY);
        log::debug!(
            "ui_wake: source={source} predicted_dt={predicted_frame_time:?} requested_delay={requested_delay:?} has_requested_repaint={} pass_nr={}",
            self.0.has_requested_repaint(),
            self.0.cumulative_pass_nr(),
        );
        self.0.request_repaint_after(requested_delay);
    }
}

/// Deterministic test adapter — records every wake source instead of touching
/// an egui context.
#[cfg(test)]
pub(crate) struct RecordingWake {
    sources: std::sync::Mutex<Vec<&'static str>>,
}

#[cfg(test)]
impl RecordingWake {
    pub(crate) fn new() -> Self {
        Self {
            sources: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn sources(&self) -> Vec<&'static str> {
        self.sources.lock().unwrap().clone()
    }
}

#[cfg(test)]
impl UiWake for RecordingWake {
    fn wake(&self, source: &'static str) {
        self.sources.lock().unwrap().push(source);
    }
}

/// A send that already happened while an unserviced gap persists re-arms the
/// wake once the last wake is this stale — coalescing must never strand work.
const REARM_WAKE_MS: u64 = 1_000;
/// A gap unserviced this long at send time indicates the UI thread is not
/// draining; warn.
const STALL_WARN_GAP_MS: u64 = 5_000;
/// Minimum spacing between stall warnings per mailbox.
const STALL_WARN_INTERVAL_MS: u64 = 30_000;

/// Sentinel for "no timestamp recorded" in the gauge's atomic millis fields.
const NO_TS: u64 = u64::MAX;

/// Pure wake decision: wake on the 0→1 gap transition, and re-arm when a send
/// occurs while a gap persists and the last wake has gone stale.
fn should_wake(unserviced_gap_before_send: u64, ms_since_last_wake: Option<u64>) -> bool {
    unserviced_gap_before_send == 0 || ms_since_last_wake.is_none_or(|ms| ms >= REARM_WAKE_MS)
}

/// Pure stall-warn decision, clock-free for unit testing: warn when a send
/// finds a long-unserviced gap and the last warning is old enough.
fn should_warn_stall(
    unserviced_gap_before_send: u64,
    gap_age_ms: u64,
    ms_since_last_warn: Option<u64>,
) -> bool {
    unserviced_gap_before_send > 0
        && gap_age_ms >= STALL_WARN_GAP_MS
        && ms_since_last_warn.is_none_or(|ms| ms >= STALL_WARN_INTERVAL_MS)
}

/// Shared queued/serviced generations plus wake/stall timestamps. Timestamps
/// are millis since `anchor`; `NO_TS` means "never".
struct Gauge {
    anchor: Instant,
    queued: AtomicU64,
    serviced: AtomicU64,
    last_wake_ms: AtomicU64,
    gap_open_ms: AtomicU64,
    last_warn_ms: AtomicU64,
}

impl Gauge {
    fn now_ms(&self) -> u64 {
        self.anchor.elapsed().as_millis() as u64
    }

    fn load_ts(field: &AtomicU64) -> Option<u64> {
        match field.load(Ordering::Acquire) {
            NO_TS => None,
            ms => Some(ms),
        }
    }

    fn note_serviced(&self) {
        let serviced = self.serviced.fetch_add(1, Ordering::AcqRel) + 1;
        if serviced >= self.queued.load(Ordering::Acquire) {
            self.gap_open_ms.store(NO_TS, Ordering::Release);
        }
    }
}

/// Sending half: enqueue + guaranteed (coalesced) UI wake. Clone freely across
/// producer threads; clones share one channel and one gauge.
pub(crate) struct UiMailbox<T> {
    tx: Sender<T>,
    wake: Arc<dyn UiWake>,
    source: &'static str,
    gauge: Arc<Gauge>,
}

impl<T> Clone for UiMailbox<T> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            wake: Arc::clone(&self.wake),
            source: self.source,
            gauge: Arc::clone(&self.gauge),
        }
    }
}

impl<T> UiMailbox<T> {
    /// Create a mailbox and its receiving half.
    pub(crate) fn channel(
        wake: Arc<dyn UiWake>,
        source: &'static str,
    ) -> (UiMailbox<T>, MailboxReceiver<T>) {
        let (tx, rx) = mpsc::channel();
        let gauge = Arc::new(Gauge {
            anchor: Instant::now(),
            queued: AtomicU64::new(0),
            serviced: AtomicU64::new(0),
            last_wake_ms: AtomicU64::new(NO_TS),
            gap_open_ms: AtomicU64::new(NO_TS),
            last_warn_ms: AtomicU64::new(NO_TS),
        });
        log::info!("ui_mailbox: created source={source}");
        (
            UiMailbox {
                tx,
                wake,
                source,
                gauge: Arc::clone(&gauge),
            },
            MailboxReceiver { rx, gauge },
        )
    }

    /// A sender clone reporting a different wake source into the same channel
    /// and gauge — for a second transport feeding one drain (e.g. host MCP
    /// alongside the CLI socket).
    pub(crate) fn with_source(&self, source: &'static str) -> UiMailbox<T> {
        log::info!(
            "ui_mailbox: derived source={source} from source={}",
            self.source
        );
        UiMailbox {
            source,
            ..self.clone()
        }
    }

    /// Enqueue `msg` and wake the UI thread (coalesced — see [`should_wake`]).
    pub(crate) fn send(&self, msg: T) -> Result<(), mpsc::SendError<T>> {
        self.tx.send(msg)?;
        let g = &self.gauge;
        let now_ms = g.now_ms();
        let serviced = g.serviced.load(Ordering::Acquire);
        let queued_before = g.queued.fetch_add(1, Ordering::AcqRel);
        let gap_before = queued_before.saturating_sub(serviced);
        if gap_before == 0 {
            g.gap_open_ms.store(now_ms, Ordering::Release);
        }
        let ms_since_last_wake =
            Gauge::load_ts(&g.last_wake_ms).map(|ms| now_ms.saturating_sub(ms));
        if should_wake(gap_before, ms_since_last_wake) {
            g.last_wake_ms.store(now_ms, Ordering::Release);
            self.wake.wake(self.source);
        }
        if let Some(open_ms) = Gauge::load_ts(&g.gap_open_ms) {
            let gap_age_ms = now_ms.saturating_sub(open_ms);
            let ms_since_last_warn =
                Gauge::load_ts(&g.last_warn_ms).map(|ms| now_ms.saturating_sub(ms));
            if should_warn_stall(gap_before, gap_age_ms, ms_since_last_warn) {
                g.last_warn_ms.store(now_ms, Ordering::Release);
                log::warn!(
                    "ui_mailbox: source={} has {} unserviced message(s) for {gap_age_ms}ms — UI thread not draining",
                    self.source,
                    gap_before,
                );
            }
        }
        Ok(())
    }
}

/// Receiving half: each successful receive advances the serviced generation,
/// so drain sites get gap tracking with zero per-site bookkeeping.
pub(crate) struct MailboxReceiver<T> {
    rx: Receiver<T>,
    gauge: Arc<Gauge>,
}

impl<T> MailboxReceiver<T> {
    pub(crate) fn try_recv(&self) -> Result<T, mpsc::TryRecvError> {
        let msg = self.rx.try_recv()?;
        self.gauge.note_serviced();
        Ok(msg)
    }

    /// Blocking receive — used by test servers that service a mailbox from a
    /// dedicated thread; production drains are per-frame `try_recv` loops.
    #[cfg(test)]
    pub(crate) fn recv(&self) -> Result<T, mpsc::RecvError> {
        let msg = self.rx.recv()?;
        self.gauge.note_serviced();
        Ok(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One send on a fresh mailbox → exactly one wake, with the right source.
    #[test]
    fn idle_send_wakes_once_with_source() {
        let wake = Arc::new(RecordingWake::new());
        let (mailbox, rx) = UiMailbox::<u32>::channel(wake.clone(), "test_idle");
        mailbox.send(7).unwrap();
        assert_eq!(wake.sources(), vec!["test_idle"]);
        assert_eq!(rx.try_recv().unwrap(), 7);
    }

    /// A rapid unserviced burst coalesces to one wake; draining re-arms the
    /// next send.
    #[test]
    fn burst_coalesces_then_rearms_after_drain() {
        let wake = Arc::new(RecordingWake::new());
        let (mailbox, rx) = UiMailbox::<u32>::channel(wake.clone(), "test_burst");
        for i in 0..10 {
            mailbox.send(i).unwrap();
        }
        assert_eq!(
            wake.sources().len(),
            1,
            "10 rapid unserviced sends must coalesce to one wake"
        );
        for i in 0..10 {
            assert_eq!(rx.try_recv().unwrap(), i);
        }
        assert!(rx.try_recv().is_err(), "channel must be fully drained");
        mailbox.send(99).unwrap();
        assert_eq!(
            wake.sources().len(),
            2,
            "first send after a full drain must wake again"
        );
        assert_eq!(rx.try_recv().unwrap(), 99);
    }

    /// 4 threads × 25 sends through clones: all 100 received, at least one
    /// wake, serviced catches queued after the drain.
    #[test]
    fn concurrent_producers_deliver_all_and_settle_gauge() {
        let wake = Arc::new(RecordingWake::new());
        let (mailbox, rx) = UiMailbox::<u32>::channel(wake.clone(), "test_concurrent");
        let handles: Vec<_> = (0..4)
            .map(|t| {
                let mailbox = mailbox.clone();
                std::thread::spawn(move || {
                    for i in 0..25 {
                        mailbox.send(t * 25 + i).unwrap();
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let mut got = Vec::new();
        while let Ok(v) = rx.try_recv() {
            got.push(v);
        }
        got.sort_unstable();
        assert_eq!(got, (0..100).collect::<Vec<_>>());
        assert!(!wake.sources().is_empty(), "at least one wake must fire");
        let queued = rx.gauge.queued.load(Ordering::Acquire);
        let serviced = rx.gauge.serviced.load(Ordering::Acquire);
        assert_eq!(queued, 100);
        assert_eq!(serviced, queued, "serviced must catch queued after drain");
    }

    /// A persistent gap re-arms the wake once the last wake goes stale, so
    /// coalescing can never strand queued work.
    #[test]
    fn stale_wake_rearms_while_gap_persists() {
        assert!(should_wake(0, None), "0→1 transition always wakes");
        assert!(
            should_wake(3, None),
            "gap with no recorded wake must fail open"
        );
        assert!(
            !should_wake(3, Some(REARM_WAKE_MS - 1)),
            "fresh wake + persisting gap coalesces"
        );
        assert!(
            should_wake(3, Some(REARM_WAKE_MS)),
            "stale wake + persisting gap re-arms"
        );
    }

    #[test]
    fn stall_warn_decision() {
        assert!(!should_warn_stall(0, STALL_WARN_GAP_MS, None), "no gap");
        assert!(
            !should_warn_stall(2, STALL_WARN_GAP_MS - 1, None),
            "gap too young"
        );
        assert!(should_warn_stall(2, STALL_WARN_GAP_MS, None), "first warn");
        assert!(
            !should_warn_stall(2, STALL_WARN_GAP_MS, Some(STALL_WARN_INTERVAL_MS - 1)),
            "rate-limited"
        );
        assert!(
            should_warn_stall(2, STALL_WARN_GAP_MS, Some(STALL_WARN_INTERVAL_MS)),
            "rate limit elapsed"
        );
    }

    /// The blocking receive also advances the serviced generation.
    #[test]
    fn recv_advances_serviced() {
        let wake = Arc::new(RecordingWake::new());
        let (mailbox, rx) = UiMailbox::<u32>::channel(wake, "test_recv");
        mailbox.send(5).unwrap();
        assert_eq!(rx.recv().unwrap(), 5);
        assert_eq!(rx.gauge.serviced.load(Ordering::Acquire), 1);
        mailbox.send(6).unwrap();
        assert_eq!(
            rx.gauge.load_gap(),
            1,
            "gap must reopen on a post-drain send"
        );
    }
}

#[cfg(test)]
impl Gauge {
    fn load_gap(&self) -> u64 {
        self.queued
            .load(Ordering::Acquire)
            .saturating_sub(self.serviced.load(Ordering::Acquire))
    }
}
