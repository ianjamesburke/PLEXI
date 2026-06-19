// Sysmon — System Monitor POC for the Plexi v2 WASM runtime.
//
// Proof of concept for: init/update/view lifecycle, effect system (timer +
// system-stats), state persistence (poll interval), typed UINode tree, and
// structural diffing (only metrics rows change, header is stable).
//
// Does not compile against a real host yet — requires the v2 wasmtime host to
// implement plexi:platform/host-state and plexi:platform/host-log.
// Build target: wasm32-wasip2 via `cargo component build`.

wit_bindgen::generate!({
    world: "plexi-app",
    path: "wit/world.wit",
});

use exports::plexi::platform::lifecycle::Guest;
use plexi::platform::types::{
    Alignment, Color, ColumnNode, Effect, IndexedNode, InputEvent, ProgressBarNode, RowNode,
    StateSnapshot, SystemStats, TextNode, TimerEffect, UiNodeData, UiTree,
};
use plexi::platform::{host_log, host_state};

// ── Constants ─────────────────────────────────────────────────────────────────

const TIMER_POLL: u32 = 1;
const DEFAULT_POLL_MS: u32 = 2000;
const POLL_INTERVAL_KEY: &str = "poll_interval_ms";

// ── App State ─────────────────────────────────────────────────────────────────

struct Sysmon {
    stats: Option<SystemStats>,
    poll_interval_ms: u32,
    // Node ID counter — incremented per tree build.
    // The host diffs by key string, so IDs just need to be consistent within one view().
    next_id: u32,
}

impl Sysmon {
    fn new(state: &StateSnapshot) -> Self {
        let poll_interval_ms = state
            .entries
            .iter()
            .find(|(k, _)| k == POLL_INTERVAL_KEY)
            .and_then(|(_, v)| {
                let arr: [u8; 4] = v.as_slice().try_into().ok()?;
                Some(u32::from_le_bytes(arr))
            })
            .unwrap_or(DEFAULT_POLL_MS);

        Sysmon { stats: None, poll_interval_ms, next_id: 0 }
    }

    fn node(&mut self, key: &str, data: UiNodeData) -> IndexedNode {
        let id = self.next_id;
        self.next_id += 1;
        IndexedNode { id, key: key.to_string(), data }
    }

    // ── View ─────────────────────────────────────────────────────────────────

    fn build_tree(&mut self) -> UiTree {
        self.next_id = 0;
        let mut nodes: Vec<IndexedNode> = Vec::new();

        // Header
        let header = self.node("header", UiNodeData::Text(TextNode {
            text: "System Monitor".to_string(),
            size: Some(20.0),
            bold: true,
            color: None,
            truncate: false,
            align: Alignment::Start,
        }));
        let header_id = header.id;
        nodes.push(header);

        match &self.stats.clone() {
            None => {
                let loading = self.node("loading", UiNodeData::Text(TextNode {
                    text: "Collecting stats…".to_string(),
                    size: Some(13.0),
                    bold: false,
                    color: Some(Color { r: 148, g: 148, b: 160, a: 255 }),
                    truncate: false,
                    align: Alignment::Start,
                }));
                let loading_id = loading.id;
                nodes.push(loading);

                let col = self.node("root", UiNodeData::Column(ColumnNode {
                    children: vec![header_id, loading_id],
                    gap: 12.0,
                    align: Alignment::Start,
                    grow: true,
                }));
                let root_id = col.id;
                nodes.push(col);
                UiTree { root: root_id, nodes }
            }

            Some(stats) => {
                let stats = stats.clone();

                // CPU row
                let cpu_bar = self.node("cpu-bar", UiNodeData::ProgressBar(ProgressBarNode {
                    value: stats.cpu_usage_pct,
                    max: 100.0,
                    color: Some(Color { r: 137, g: 180, b: 250, a: 255 }), // blue
                    label: None,
                }));
                let cpu_label = self.node("cpu-label", UiNodeData::Text(TextNode {
                    text: format!("CPU  {:.1}%", stats.cpu_usage_pct),
                    size: Some(13.0),
                    bold: false,
                    color: None,
                    truncate: false,
                    align: Alignment::End,
                }));
                let cpu_row = self.node("cpu-row", UiNodeData::Row(RowNode {
                    children: vec![cpu_bar.id, cpu_label.id],
                    gap: 8.0,
                    align: Alignment::Center,
                    grow: true,
                }));
                nodes.extend([cpu_bar, cpu_label]);
                let cpu_row_id = cpu_row.id;
                nodes.push(cpu_row);

                // Memory row
                let mem_pct = stats.memory_used_bytes as f32 / stats.memory_total_bytes.max(1) as f32 * 100.0;
                let mem_bar = self.node("mem-bar", UiNodeData::ProgressBar(ProgressBarNode {
                    value: mem_pct,
                    max: 100.0,
                    color: Some(Color { r: 166, g: 227, b: 161, a: 255 }), // green
                    label: None,
                }));
                let mem_label = self.node("mem-label", UiNodeData::Text(TextNode {
                    text: format!(
                        "Mem  {:.1} / {:.1} GB",
                        stats.memory_used_bytes as f32 / 1_073_741_824.0,
                        stats.memory_total_bytes as f32 / 1_073_741_824.0,
                    ),
                    size: Some(13.0),
                    bold: false,
                    color: None,
                    truncate: false,
                    align: Alignment::End,
                }));
                let mem_row = self.node("mem-row", UiNodeData::Row(RowNode {
                    children: vec![mem_bar.id, mem_label.id],
                    gap: 8.0,
                    align: Alignment::Center,
                    grow: true,
                }));
                nodes.extend([mem_bar, mem_label]);
                let mem_row_id = mem_row.id;
                nodes.push(mem_row);

                // Network row
                let net = self.node("net", UiNodeData::Text(TextNode {
                    text: format!(
                        "Net  ↑ {}/s  ↓ {}/s",
                        fmt_bytes(stats.net_tx_bps),
                        fmt_bytes(stats.net_rx_bps),
                    ),
                    size: Some(12.0),
                    bold: false,
                    color: Some(Color { r: 148, g: 148, b: 160, a: 255 }),
                    truncate: false,
                    align: Alignment::Start,
                }));
                let net_id = net.id;
                nodes.push(net);

                // Uptime row
                let uptime = self.node("uptime", UiNodeData::Text(TextNode {
                    text: format!("Up   {}", fmt_uptime(stats.uptime_secs)),
                    size: Some(12.0),
                    bold: false,
                    color: Some(Color { r: 148, g: 148, b: 160, a: 255 }),
                    truncate: false,
                    align: Alignment::Start,
                }));
                let uptime_id = uptime.id;
                nodes.push(uptime);

                let divider = self.node("divider", UiNodeData::Divider);
                let divider_id = divider.id;
                nodes.push(divider);

                let hint = self.node("hint", UiNodeData::Text(TextNode {
                    text: format!("Polling every {}s  ·  q to close", self.poll_interval_ms / 1000),
                    size: Some(11.0),
                    bold: false,
                    color: Some(Color { r: 108, g: 112, b: 134, a: 255 }),
                    truncate: false,
                    align: Alignment::Start,
                }));
                let hint_id = hint.id;
                nodes.push(hint);

                let col = self.node("root", UiNodeData::Column(ColumnNode {
                    children: vec![header_id, cpu_row_id, mem_row_id, net_id, uptime_id, divider_id, hint_id],
                    gap: 10.0,
                    align: Alignment::Start,
                    grow: true,
                }));
                let root_id = col.id;
                nodes.push(col);
                UiTree { root: root_id, nodes }
            }
        }
    }
}

// ── Lifecycle implementation ──────────────────────────────────────────────────

// Pure state transitions — no host imports on these paths (except the +/- arms,
// which persist through host-state). Kept free functions on Sysmon so they can
// be unit-tested directly without a host (G2).
impl Sysmon {
    fn startup_effects(&self) -> Vec<Effect> {
        vec![
            Effect::GetSystemStats,
            Effect::SetTimer(TimerEffect { id: TIMER_POLL, delay_ms: self.poll_interval_ms, repeat: true }),
        ]
    }

    fn handle(&mut self, event: InputEvent) -> Vec<Effect> {
        match event {
            InputEvent::SystemStatsResult(stats) => {
                self.stats = Some(stats);
                vec![]
            }

            InputEvent::TimerFired(TIMER_POLL) => vec![Effect::GetSystemStats],

            InputEvent::Key(key) if key.key == "q" || key.key == "escape" => {
                vec![Effect::CloseSelf]
            }

            InputEvent::Key(key) if key.key == "+" || key.key == "=" => {
                self.poll_interval_ms = (self.poll_interval_ms + 1000).min(30_000);
                self.persist_interval();
                self.retimer_effects()
            }

            InputEvent::Key(key) if key.key == "-" => {
                self.poll_interval_ms = self.poll_interval_ms.saturating_sub(1000).max(1000);
                self.persist_interval();
                self.retimer_effects()
            }

            _ => vec![],
        }
    }

    fn retimer_effects(&self) -> Vec<Effect> {
        vec![
            Effect::CancelTimer(TIMER_POLL),
            Effect::SetTimer(TimerEffect { id: TIMER_POLL, delay_ms: self.poll_interval_ms, repeat: true }),
            Effect::GetSystemStats,
        ]
    }

    fn persist_interval(&self) {
        let _ = host_state::set(POLL_INTERVAL_KEY, &self.poll_interval_ms.to_le_bytes().to_vec());
    }
}

struct Component;

static mut APP: Option<Sysmon> = None;

fn app() -> &'static mut Sysmon {
    // Safety: single-threaded WASM, no concurrent access.
    unsafe { (*core::ptr::addr_of_mut!(APP)).as_mut().unwrap() }
}

impl Guest for Component {
    fn init(state: StateSnapshot, _size: (f32, f32), _args: Vec<String>) -> Vec<Effect> {
        let sysmon = Sysmon::new(&state);
        host_log::info(&format!("sysmon: init poll_interval_ms={}", sysmon.poll_interval_ms));
        let effects = sysmon.startup_effects();
        unsafe { APP = Some(sysmon); }
        effects
    }

    fn update(event: InputEvent) -> Vec<Effect> {
        app().handle(event)
    }

    fn view() -> UiTree {
        app().build_tree()
    }
}

export!(Component);

// ── Helpers ───────────────────────────────────────────────────────────────────

fn fmt_bytes(bps: u64) -> String {
    match bps {
        b if b >= 1_073_741_824 => format!("{:.1} GB", b as f64 / 1_073_741_824.0),
        b if b >= 1_048_576 => format!("{:.1} MB", b as f64 / 1_048_576.0),
        b if b >= 1024 => format!("{:.0} KB", b as f64 / 1024.0),
        b => format!("{} B", b),
    }
}

fn fmt_uptime(secs: u64) -> String {
    let d = secs / 86400;
    let h = (secs % 86400) / 3600;
    let m = (secs % 3600) / 60;
    if d > 0 { format!("{}d {}h {}m", d, h, m) }
    else if h > 0 { format!("{}h {}m", h, m) }
    else { format!("{}m", m) }
}

// ── G2: pure-function unit tests ──────────────────────────────────────────────
// init/update/view logic is testable without a host, a process, or network.
#[cfg(test)]
mod tests {
    use super::*;

    fn empty() -> StateSnapshot {
        StateSnapshot { entries: vec![] }
    }

    fn mock_stats(cpu: f32) -> SystemStats {
        SystemStats {
            cpu_usage_pct: cpu,
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

    fn tree_text(tree: &UiTree) -> String {
        tree.nodes
            .iter()
            .filter_map(|n| match &n.data {
                UiNodeData::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn default_poll_interval_is_2s() {
        assert_eq!(Sysmon::new(&empty()).poll_interval_ms, DEFAULT_POLL_MS);
    }

    #[test]
    fn timer_fired_requests_stats() {
        let mut app = Sysmon::new(&empty());
        let effects = app.handle(InputEvent::TimerFired(TIMER_POLL));
        assert!(effects.iter().any(|e| matches!(e, Effect::GetSystemStats)));
    }

    #[test]
    fn stats_result_updates_view() {
        let mut app = Sysmon::new(&empty());
        app.handle(InputEvent::SystemStatsResult(mock_stats(42.0)));
        let tree = app.build_tree();
        assert!(tree_text(&tree).contains("42.0%"), "view should show CPU 42.0%");
    }

    #[test]
    fn startup_sets_timer_at_persisted_interval() {
        let snap = StateSnapshot {
            entries: vec![(POLL_INTERVAL_KEY.to_string(), 5000u32.to_le_bytes().to_vec())],
        };
        let effects = Sysmon::new(&snap).startup_effects();
        assert!(effects
            .iter()
            .any(|e| matches!(e, Effect::SetTimer(t) if t.delay_ms == 5000)));
    }
}
