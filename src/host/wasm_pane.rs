// WasmPane — the pane-level driver around a `WasmApp`.
//
// Implements the synchronous Elm effect loop: external input (keys, mouse,
// resize, focus) is queued, `tick` fires due timers and drains the queue, and
// each `update` effect is executed against host services — system stats from a
// pluggable `SystemStatsSource`, ms timers tracked here off frame time, and
// title/status/close surfaced for the pane chrome. Effects that spawn results
// (get-system-stats, timer-fired) push follow-up input events back onto the
// same queue, so the loop converges within one tick. No async runtime.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_queue::ArrayQueue;

use crate::app::app_trait::KeyDisposition;
use crate::media::audio::{start_output_stream, OutputSession};
use crate::ui::theme::Colors;

use super::wasm_app::{
    Effect, InputEvent, KeyEvent, Modifiers, StateSnapshot, SystemStats, UiTree, WasmApp,
};
use super::wasm_render::render_ui_tree;

/// Source of system metrics for the `get-system-stats` effect. Production uses
/// a sysinfo-backed implementation; tests use a fake with fixed values.
pub trait SystemStatsSource: Send {
    fn read(&mut self) -> SystemStats;
}

/// Production [`SystemStatsSource`] backed by `sysinfo`. CPU usage needs two
/// refreshes spaced by a real interval to be meaningful, so the first read
/// after construction reports 0% and subsequent reads (driven by the app's
/// poll timer, ≥1s apart) are accurate. Disk and network byte-rates require
/// interval deltas the host does not yet track and are reported as 0.
pub struct SysinfoStats {
    sys: sysinfo::System,
}

impl SysinfoStats {
    pub fn new() -> Self {
        let mut sys = sysinfo::System::new();
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        SysinfoStats { sys }
    }
}

impl SystemStatsSource for SysinfoStats {
    fn read(&mut self) -> SystemStats {
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();
        let load = sysinfo::System::load_average();
        SystemStats {
            cpu_usage_pct: self.sys.global_cpu_usage(),
            memory_used_bytes: self.sys.used_memory(),
            memory_total_bytes: self.sys.total_memory(),
            disk_read_bps: 0,
            disk_write_bps: 0,
            net_rx_bps: 0,
            net_tx_bps: 0,
            uptime_secs: sysinfo::System::uptime(),
            load_avg_one_min: load.one as f32,
        }
    }
}

struct Timer {
    id: u32,
    delay_ms: u32,
    repeat: bool,
    next_fire_ms: u64,
}

/// Live RT-audio output for an audio-world app. The host UI thread tops up
/// `ring` from the guest's `process-output`; the cpal callback owned by
/// `_session` drains it on the audio thread. Negotiated `sample_rate`/`channels`
/// come from the device and may differ from what the guest requested — the
/// guest is driven at the negotiated values so pitch stays correct.
struct AudioOut {
    ring: Arc<ArrayQueue<f32>>,
    _session: OutputSession,
    handle: u32,
    sample_rate: u32,
    channels: u32,
    buffer_frames: u32,
    state: u64,
}

pub struct WasmPane {
    app: WasmApp,
    stats: Box<dyn SystemStatsSource>,
    queue: VecDeque<InputEvent>,
    timers: Vec<Timer>,
    wants_close: bool,
    pending_title: Option<String>,
    pending_status: Option<String>,
    audio: Option<AudioOut>,
}

impl WasmPane {
    pub fn new(app: WasmApp, stats: Box<dyn SystemStatsSource>) -> Self {
        WasmPane {
            app,
            stats,
            queue: VecDeque::new(),
            timers: Vec::new(),
            wants_close: false,
            pending_title: None,
            pending_status: None,
            audio: None,
        }
    }

    /// Run the guest's `init`, execute its startup effects, and converge the
    /// resulting input queue (e.g. the first stats request).
    pub fn init(
        &mut self,
        snapshot: &super::wasm_app::StateSnapshot,
        size: (f32, f32),
        now_ms: u64,
    ) -> wasmtime::Result<()> {
        let effects = self.app.init(snapshot, size)?;
        for e in effects {
            self.exec(e, now_ms);
        }
        self.drain(now_ms)?;
        // The guest opens its audio stream during init; start the host output
        // and prime the ring once it has.
        self.start_audio();
        self.pump_audio()
    }

    /// Enqueue an external input event for the next `tick`.
    pub fn push_input(&mut self, event: InputEvent) {
        self.queue.push_back(event);
    }

    /// Fire any due timers and drain the input queue. `now_ms` is monotonic
    /// elapsed milliseconds since the pane started.
    pub fn tick(&mut self, now_ms: u64) -> wasmtime::Result<()> {
        self.fire_timers(now_ms);
        self.drain(now_ms)?;
        self.pump_audio()
    }

    /// True once a live audio output stream is running. The live pane uses this
    /// to keep requesting repaints so the ring stays topped up.
    pub fn has_audio(&self) -> bool {
        self.audio.is_some()
    }

    /// Start the host output stream if the guest opened one and it isn't running
    /// yet. A device failure is logged and leaves the pane silent — never fatal.
    fn start_audio(&mut self) {
        if self.audio.is_some() {
            return;
        }
        let Some((handle, rate, channels, buffer_frames)) = self.app.output_stream_config() else {
            return;
        };
        // Ring holds ~8 buffers so a stalled UI thread doesn't underrun audibly.
        let capacity = (buffer_frames.max(1) * channels.max(1) * 8) as usize;
        let ring = Arc::new(ArrayQueue::new(capacity));
        match start_output_stream(rate, channels as u16, Arc::clone(&ring)) {
            Ok(session) => {
                let sample_rate = session.sample_rate;
                let channels = session.channels;
                log::info!(
                    "wasm audio: output stream started (stream {handle}, {sample_rate} Hz, {channels} ch)"
                );
                self.audio = Some(AudioOut {
                    ring,
                    _session: session,
                    handle,
                    sample_rate,
                    channels,
                    buffer_frames,
                    state: 0,
                });
            }
            Err(e) => log::warn!("wasm audio: output stream failed, pane stays silent: {e}"),
        }
    }

    /// Top up the audio ring from the guest's `process-output` while it has room
    /// for at least one more buffer. Runs on the UI thread (which owns the
    /// Store); the cpal callback only pops, so the RT thread never re-enters
    /// wasmtime.
    fn pump_audio(&mut self) -> wasmtime::Result<()> {
        let Some(audio) = self.audio.as_mut() else {
            return Ok(());
        };
        let frame_samples = (audio.buffer_frames * audio.channels) as usize;
        while audio.ring.capacity() - audio.ring.len() >= frame_samples {
            let (samples, next) = self.app.audio_process_output(
                audio.handle,
                audio.buffer_frames,
                audio.channels,
                audio.sample_rate,
                audio.state,
            )?;
            audio.state = next;
            for s in samples {
                if audio.ring.push(s).is_err() {
                    break;
                }
            }
        }
        Ok(())
    }

    pub fn view(&mut self) -> wasmtime::Result<UiTree> {
        self.app.view()
    }

    pub fn wants_close(&self) -> bool {
        self.wants_close
    }

    /// Earliest pending timer deadline (monotonic ms), if any. The live render
    /// loop uses this to schedule the next repaint so timers fire on time
    /// without busy-polling.
    pub fn next_deadline_ms(&self) -> Option<u64> {
        self.timers.iter().map(|t| t.next_fire_ms).min()
    }

    pub fn take_title(&mut self) -> Option<String> {
        self.pending_title.take()
    }

    pub fn take_status(&mut self) -> Option<String> {
        self.pending_status.take()
    }

    fn fire_timers(&mut self, now_ms: u64) {
        let mut fired: Vec<u32> = Vec::new();
        self.timers.retain_mut(|t| {
            if now_ms >= t.next_fire_ms {
                fired.push(t.id);
                if t.repeat {
                    t.next_fire_ms = now_ms + t.delay_ms as u64;
                    true
                } else {
                    false
                }
            } else {
                true
            }
        });
        for id in fired {
            self.queue.push_back(InputEvent::TimerFired(id));
        }
    }

    fn drain(&mut self, now_ms: u64) -> wasmtime::Result<()> {
        while let Some(event) = self.queue.pop_front() {
            let effects = self.app.update(&event)?;
            for e in effects {
                self.exec(e, now_ms);
            }
        }
        Ok(())
    }

    fn exec(&mut self, effect: Effect, now_ms: u64) {
        match effect {
            Effect::GetSystemStats => {
                let stats = self.stats.read();
                self.queue.push_back(InputEvent::SystemStatsResult(stats));
            }
            Effect::SetTimer(t) => {
                let next_fire_ms = now_ms + t.delay_ms as u64;
                if let Some(existing) = self.timers.iter_mut().find(|x| x.id == t.id) {
                    existing.delay_ms = t.delay_ms;
                    existing.repeat = t.repeat;
                    existing.next_fire_ms = next_fire_ms;
                } else {
                    self.timers.push(Timer {
                        id: t.id,
                        delay_ms: t.delay_ms,
                        repeat: t.repeat,
                        next_fire_ms,
                    });
                }
            }
            Effect::CancelTimer(id) => {
                self.timers.retain(|t| t.id != id);
            }
            Effect::SetTitle(title) => {
                self.pending_title = Some(title);
            }
            Effect::SetStatus(status) => {
                self.pending_status = Some(status);
            }
            Effect::CloseSelf => {
                self.wants_close = true;
            }
            Effect::RequestCapability(cap) => {
                log::info!("wasm app requested capability (not yet grantable): {cap}");
            }
            // File / HTTP effects run on a worker thread in a later milestone;
            // the POCs in scope do not exercise them.
            Effect::FileRead(_) | Effect::FileWrite(_) | Effect::HttpFetch(_) => {
                log::warn!("wasm app issued an I/O effect not yet supported by the host");
            }
        }
    }
}

// ─── Live adapter ───────────────────────────────────────────────────────────
//
// `LiveWasmPane` bridges the time-injected, headless [`WasmPane`] to the host's
// live egui render loop. It owns the monotonic clock (so `WasmPane` stays pure
// and testable), runs `init` lazily on the first frame once the pane size is
// known, translates egui key input into guest `InputEvent`s using the same
// printable-vs-named key split as `process_app`, and renders the guest's view
// tree each frame. A fatal guest/runtime error is captured and shown in place
// rather than propagated — a misbehaving WASM app must never crash the host.

pub struct LiveWasmPane {
    inner: WasmPane,
    started: Instant,
    spawn_name: String,
    title: Option<String>,
    pending_init: Option<StateSnapshot>,
    error: Option<String>,
    /// Concatenated text of the last rendered view tree. Lets the headless
    /// scene runner assert on rendered content without re-entering the guest.
    last_text: String,
}

impl LiveWasmPane {
    /// Build a live pane. `init` is deferred to the first `ui` call, when the
    /// egui region size is known. `snapshot` is the persisted state to restore.
    pub fn new(inner: WasmPane, spawn_name: impl Into<String>, snapshot: StateSnapshot) -> Self {
        LiveWasmPane {
            inner,
            started: Instant::now(),
            spawn_name: spawn_name.into(),
            title: None,
            pending_init: Some(snapshot),
            error: None,
            last_text: String::new(),
        }
    }

    fn now_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    /// True while the app is alive (no fatal error, has not asked to close).
    pub fn is_running(&self) -> bool {
        self.error.is_none() && !self.inner.wants_close()
    }

    /// Text content of the most recently rendered view, for scene assertions.
    pub fn last_render_text(&self) -> &str {
        &self.last_text
    }

    fn fail(&mut self, ctx: &str, e: impl std::fmt::Display) {
        let msg = format!("{ctx}: {e}");
        log::error!("wasm pane error — {msg}");
        self.error = Some(msg);
    }

    pub fn wants_close(&self) -> bool {
        self.inner.wants_close()
    }

    pub fn display_name(&self) -> String {
        self.title.clone().unwrap_or_else(|| self.spawn_name.clone())
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, colors: &Colors) {
        if let Some(err) = &self.error {
            ui.colored_label(colors.danger, err);
            return;
        }

        let size = ui.available_size();
        let now = self.now_ms();

        let stepped = if let Some(snapshot) = self.pending_init.take() {
            self.inner.init(&snapshot, (size.x, size.y), now)
        } else {
            self.inner.tick(now)
        };
        if let Err(e) = stepped {
            self.fail("step", e);
            return;
        }

        if let Some(t) = self.inner.take_title() {
            self.title = Some(t);
        }
        let _ = self.inner.take_status();

        let tree = match self.inner.view() {
            Ok(t) => t,
            Err(e) => {
                self.fail("view", e);
                return;
            }
        };
        self.last_text = collect_tree_text(&tree);
        let result = render_ui_tree(ui, &tree, colors);
        if !result.actions.is_empty() || !result.value_changes.is_empty() {
            // The current WIT `input-event` has no generic action/value case;
            // the in-scope POCs drive interaction via keys. Surface rather than
            // silently drop so a future contract addition is obvious.
            log::debug!(
                "wasm pane produced {} action(s) / {} value change(s) with no input-event path yet",
                result.actions.len(),
                result.value_changes.len()
            );
        }

        // An audio app must keep topping up its sample ring, so repaint at
        // audio cadence regardless of timers. Otherwise schedule the next
        // repaint for the earliest pending timer so polling apps refresh on
        // time without busy-looping.
        if self.inner.has_audio() {
            ui.ctx().request_repaint_after(Duration::from_millis(15));
        } else if let Some(deadline) = self.inner.next_deadline_ms() {
            ui.ctx()
                .request_repaint_after(Duration::from_millis(deadline.saturating_sub(now)));
        }
    }

    /// Translate egui key input into guest `InputEvent::Key`s. Mirrors the
    /// printable-vs-named split in `process_app::mod` so that letters, digits,
    /// and punctuation arrive as their OS-resolved character (via `Event::Text`)
    /// and named keys (escape, enter, arrows) arrive lowercased. Cmd-modified
    /// chords are reserved for host shortcuts and never reach the app.
    pub fn handle_key(&mut self, input: &egui::InputState) -> KeyDisposition {
        let mut consumed = false;
        for event in &input.events {
            match event {
                egui::Event::Key { key, pressed: true, modifiers, .. } => {
                    if (!printable_key(*key) || modifiers.ctrl) && !modifiers.command {
                        self.inner.push_input(InputEvent::Key(KeyEvent {
                            key: format!("{key:?}").to_lowercase(),
                            modifiers: Modifiers {
                                ctrl: modifiers.ctrl,
                                shift: modifiers.shift,
                                alt: modifiers.alt,
                                meta: modifiers.command,
                            },
                            pressed: true,
                        }));
                    }
                    consumed = true;
                }
                egui::Event::Text(text) => {
                    for ch in text.chars() {
                        if ch.is_control() {
                            continue;
                        }
                        self.inner.push_input(InputEvent::Key(KeyEvent {
                            key: ch.to_string(),
                            modifiers: Modifiers { ctrl: false, shift: false, alt: false, meta: false },
                            pressed: true,
                        }));
                    }
                    consumed = true;
                }
                _ => {}
            }
        }
        if consumed {
            KeyDisposition::Consumed
        } else {
            KeyDisposition::Passthrough
        }
    }
}

/// Join every text node in a view tree into a single string (newline-joined),
/// preserving arena order. Used for headless content assertions.
fn collect_tree_text(tree: &UiTree) -> String {
    use super::wasm_app::UiNodeData;
    tree.nodes
        .iter()
        .filter_map(|n| match &n.data {
            UiNodeData::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether an egui key produces an `Event::Text` at the OS level (letters,
/// digits, punctuation). For these the `Event::Text` arm delivers the
/// shift-resolved character, so the `Event::Key` arm is suppressed to avoid a
/// double delivery. Canonical source: `process_app::mod` (same set).
fn printable_key(key: egui::Key) -> bool {
    use egui::Key::*;
    matches!(
        key,
        A | B | C | D | E | F | G | H | I | J | K | L | M | N | O | P | Q | R | S | T | U | V | W
            | X | Y | Z
            | Num0 | Num1 | Num2 | Num3 | Num4 | Num5 | Num6 | Num7 | Num8 | Num9
            | Minus | Equals | OpenBracket | CloseBracket | Backslash | Semicolon | Quote
            | Backtick | Comma | Period | Slash | Plus
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::wasm_app::{
        KeyEvent, Modifiers, StateSnapshot, StateStore, UiNodeData, WasmApp,
    };
    use std::path::PathBuf;

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wasm-fixtures/sysmon.wasm")
    }

    struct FakeStats {
        cpu: f32,
    }
    impl SystemStatsSource for FakeStats {
        fn read(&mut self) -> SystemStats {
            SystemStats {
                cpu_usage_pct: self.cpu,
                memory_used_bytes: 8u64 << 30,
                memory_total_bytes: 16u64 << 30,
                disk_read_bps: 0,
                disk_write_bps: 0,
                net_rx_bps: 0,
                net_tx_bps: 0,
                uptime_secs: 0,
                load_avg_one_min: 0.0,
            }
        }
    }

    fn pane(cpu: f32) -> WasmPane {
        let app = WasmApp::load_ephemeral_run("sysmon-pane", &fixture(), StateStore::ephemeral())
            .expect("load");
        WasmPane::new(app, Box::new(FakeStats { cpu }))
    }

    fn key(k: &str) -> InputEvent {
        InputEvent::Key(KeyEvent {
            key: k.to_string(),
            modifiers: Modifiers { ctrl: false, shift: false, alt: false, meta: false },
            pressed: true,
        })
    }

    fn cpu_text(tree: &UiTree) -> String {
        tree.nodes
            .iter()
            .filter_map(|n| match &n.data {
                UiNodeData::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    // init runs startup effects: the first get-system-stats resolves through
    // the stats source and the view shows the CPU percentage.
    #[test]
    fn init_resolves_first_stats() -> wasmtime::Result<()> {
        let mut p = pane(42.0);
        p.init(&StateSnapshot { entries: vec![] }, (400.0, 300.0), 0)?;
        assert!(cpu_text(&p.view()?).contains("42.0%"));
        Ok(())
    }

    // The repeating poll timer (id 1, 2000ms) fires once elapsed time passes
    // its deadline, requesting fresh stats that reflect the updated source.
    #[test]
    fn poll_timer_refreshes_stats() -> wasmtime::Result<()> {
        let mut p = pane(10.0);
        p.init(&StateSnapshot { entries: vec![] }, (400.0, 300.0), 0)?;
        assert!(cpu_text(&p.view()?).contains("10.0%"));

        p.stats = Box::new(FakeStats { cpu: 88.0 });
        p.tick(2_500)?; // past the 2000ms poll deadline
        assert!(cpu_text(&p.view()?).contains("88.0%"));
        Ok(())
    }

    // 'q' maps to a close-self effect, surfaced to the pane as wants_close.
    #[test]
    fn q_closes() -> wasmtime::Result<()> {
        let mut p = pane(5.0);
        p.init(&StateSnapshot { entries: vec![] }, (400.0, 300.0), 0)?;
        assert!(!p.wants_close());
        p.push_input(key("q"));
        p.tick(10)?;
        assert!(p.wants_close());
        Ok(())
    }

    // '=' x3 raises the poll interval to 5000ms; the next fire only occurs at
    // the new deadline, not the old 2000ms one.
    #[test]
    fn equals_raises_poll_interval() -> wasmtime::Result<()> {
        let mut p = pane(7.0);
        p.init(&StateSnapshot { entries: vec![] }, (400.0, 300.0), 0)?;
        for _ in 0..3 {
            p.push_input(key("="));
        }
        p.tick(100)?;
        // a tick at 3000ms would have fired the old 2000ms timer; with a 5000ms
        // interval there is exactly one pending timer at next_fire 100+5000.
        assert_eq!(p.timers.len(), 1);
        assert_eq!(p.timers[0].delay_ms, 5000);
        assert_eq!(p.timers[0].next_fire_ms, 5100);
        Ok(())
    }
}
