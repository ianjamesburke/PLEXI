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

use super::wasm_app::{Effect, InputEvent, SystemStats, UiTree, WasmApp};

/// Source of system metrics for the `get-system-stats` effect. Production uses
/// a sysinfo-backed implementation; tests use a fake with fixed values.
pub trait SystemStatsSource: Send {
    fn read(&mut self) -> SystemStats;
}

struct Timer {
    id: u32,
    delay_ms: u32,
    repeat: bool,
    next_fire_ms: u64,
}

pub struct WasmPane {
    app: WasmApp,
    stats: Box<dyn SystemStatsSource>,
    queue: VecDeque<InputEvent>,
    timers: Vec<Timer>,
    wants_close: bool,
    pending_title: Option<String>,
    pending_status: Option<String>,
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
        self.drain(now_ms)
    }

    /// Enqueue an external input event for the next `tick`.
    pub fn push_input(&mut self, event: InputEvent) {
        self.queue.push_back(event);
    }

    /// Fire any due timers and drain the input queue. `now_ms` is monotonic
    /// elapsed milliseconds since the pane started.
    pub fn tick(&mut self, now_ms: u64) -> wasmtime::Result<()> {
        self.fire_timers(now_ms);
        self.drain(now_ms)
    }

    pub fn view(&mut self) -> wasmtime::Result<UiTree> {
        self.app.view()
    }

    pub fn wants_close(&self) -> bool {
        self.wants_close
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::wasm_app::{
        Grants, KeyEvent, Modifiers, StateSnapshot, StateStore, UiNodeData, WasmApp,
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
        let app = WasmApp::load(
            "sysmon-pane",
            &fixture(),
            &Grants::standard_app(),
            StateStore::ephemeral(),
        )
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
