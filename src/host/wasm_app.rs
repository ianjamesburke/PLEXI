// WasmApp — wasmtime-backed runtime for Plexi v2 WASM component apps.
//
// One `Store` per pane. The component exports the `lifecycle` interface
// (init/update/view) and imports host capabilities (host-log, host-state,
// pipes). Effects returned by init/update are executed by the host, which
// feeds results back as the next `input-event` — a synchronous Elm-style loop,
// no async runtime.
//
// Rendering (UiTree -> egui) and pane-tree wiring land in M3; this module is
// the runtime core validated by the G3 (effect roundtrip) and G5 (state
// persistence) gates against the sysmon POC, with no subprocess involved.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

// The crate itself is named `plexi`; bindgen also generates a top-level `plexi`
// module from the `plexi:platform` package. Nest the generated code so the two
// never collide at a use-path.
mod bindings {
    wasmtime::component::bindgen!({
        path: "wit/plexi.wit",
        world: "plexi-app",
    });
}

use bindings::plexi::platform::{host_log, host_state, pipes, types};
use bindings::PlexiApp;

pub use types::{
    Alignment, BadgeColor, ButtonStyle, Color, Effect, IndexedNode, InputEvent, StateSnapshot,
    SystemStats, UiNodeData, UiTree,
};

// ─── Capability grants ─────────────────────────────────────────────────────
//
// host-log is always linked (logging is unconditionally safe). Every other
// host interface is linked only when granted. An app whose component imports an
// ungranted interface fails to instantiate — link-time gating, fail-fast.

#[derive(Clone, Debug, Default)]
pub struct Grants {
    pub state: bool,
    pub pipes: bool,
    pub gpu: bool,
    pub audio: bool,
}

impl Grants {
    /// Baseline grants for a standard `plexi-app`: state + pipes, no GPU/audio.
    pub fn standard_app() -> Self {
        Grants { state: true, pipes: true, gpu: false, audio: false }
    }
}

// ─── State store (G5) ──────────────────────────────────────────────────────
//
// Host-owned key/value store backing the `host-state` import. Persistent
// stores write the full map to a per-namespace JSON file on every mutation;
// ephemeral stores (e.g. `plexi run`) keep the map in memory only.

pub struct StateStore {
    data: HashMap<String, Vec<u8>>,
    path: Option<PathBuf>,
}

impl StateStore {
    pub fn ephemeral() -> Self {
        StateStore { data: HashMap::new(), path: None }
    }

    /// Open (or create) a persistent store at `path`. Existing contents are
    /// loaded; a missing file starts empty.
    pub fn persistent(path: PathBuf) -> std::io::Result<Self> {
        let data = if path.exists() {
            let bytes = std::fs::read(&path)?;
            serde_json::from_slice(&bytes).map_err(std::io::Error::other)?
        } else {
            HashMap::new()
        };
        Ok(StateStore { data, path: Some(path) })
    }

    fn persist(&self) -> Result<(), String> {
        let Some(path) = &self.path else { return Ok(()) };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let bytes = serde_json::to_vec(&self.data).map_err(|e| e.to_string())?;
        std::fs::write(path, bytes).map_err(|e| e.to_string())
    }

    pub fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.data.get(key).cloned()
    }

    pub fn set(&mut self, key: String, value: Vec<u8>) -> Result<(), String> {
        self.data.insert(key, value);
        self.persist()
    }

    pub fn delete(&mut self, key: &str) -> Result<(), String> {
        self.data.remove(key);
        self.persist()
    }

    pub fn list_prefix(&self, prefix: &str) -> Vec<String> {
        self.data.keys().filter(|k| k.starts_with(prefix)).cloned().collect()
    }

    pub fn snapshot(&self) -> StateSnapshot {
        StateSnapshot {
            entries: self.data.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        }
    }
}

// ─── Host context (store data) ─────────────────────────────────────────────

pub struct HostCtx {
    app_id: String,
    state: StateStore,
    // Baseline WASI 0.2: clocks + random only. Rust-compiled components import
    // wasi:cli/io/clocks/random from std even when unused; a default ctx grants
    // no env, filesystem, or network access — the real platform capabilities
    // are the Plexi host-* interfaces, gated separately.
    wasi: WasiCtx,
    table: ResourceTable,
}

impl WasiView for HostCtx {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.wasi, table: &mut self.table }
    }
}

impl host_log::Host for HostCtx {
    fn debug(&mut self, msg: String) {
        log::debug!("app::{}: {}", self.app_id, msg);
    }
    fn info(&mut self, msg: String) {
        log::info!("app::{}: {}", self.app_id, msg);
    }
    fn warn(&mut self, msg: String) {
        log::warn!("app::{}: {}", self.app_id, msg);
    }
    fn error(&mut self, msg: String) {
        log::error!("app::{}: {}", self.app_id, msg);
    }
}

impl host_state::Host for HostCtx {
    fn get(&mut self, key: String) -> Option<Vec<u8>> {
        self.state.get(&key)
    }
    fn set(&mut self, key: String, value: Vec<u8>) -> Result<(), String> {
        self.state.set(key, value)
    }
    fn delete(&mut self, key: String) -> Result<(), String> {
        self.state.delete(&key)
    }
    fn list_prefix(&mut self, prefix: String) -> Vec<String> {
        self.state.list_prefix(&prefix)
    }
    fn snapshot(&mut self) -> StateSnapshot {
        self.state.snapshot()
    }
}

impl pipes::Host for HostCtx {
    fn open(
        &mut self,
        _id: String,
        _kind: pipes::PipeType,
        _direction: pipes::PipeDirection,
    ) -> Result<u32, String> {
        Err("pipes are not wired until M4".to_string())
    }
    fn send_binary(&mut self, _handle: u32, _payload: Vec<u8>) -> Result<(), String> {
        Err("pipes are not wired until M4".to_string())
    }
    fn send_json(&mut self, _handle: u32, _json: String) -> Result<(), String> {
        Err("pipes are not wired until M4".to_string())
    }
    fn close(&mut self, _handle: u32) -> Result<(), String> {
        Err("pipes are not wired until M4".to_string())
    }
    fn is_connected(&mut self, _handle: u32) -> bool {
        false
    }
}

// ─── WasmApp ───────────────────────────────────────────────────────────────

pub struct WasmApp {
    store: Store<HostCtx>,
    bindings: PlexiApp,
}

impl WasmApp {
    /// Load a component from `path`, link only granted capabilities, and
    /// instantiate it with `state` as its backing store.
    pub fn load(
        app_id: impl Into<String>,
        path: &Path,
        grants: &Grants,
        state: StateStore,
    ) -> wasmtime::Result<Self> {
        let app_id = app_id.into();

        let mut config = Config::new();
        config.wasm_component_model(true);
        let engine = Engine::new(&config)?;

        let component = Component::from_file(&engine, path)?;

        let mut linker = Linker::<HostCtx>::new(&engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
        host_log::add_to_linker::<_, HasSelf<HostCtx>>(&mut linker, |c| c)?;
        if grants.state {
            host_state::add_to_linker::<_, HasSelf<HostCtx>>(&mut linker, |c| c)?;
        }
        if grants.pipes {
            pipes::add_to_linker::<_, HasSelf<HostCtx>>(&mut linker, |c| c)?;
        }

        let ctx = HostCtx {
            app_id: app_id.clone(),
            state,
            wasi: WasiCtxBuilder::new().build(),
            table: ResourceTable::new(),
        };
        let mut store = Store::new(&engine, ctx);
        let bindings = PlexiApp::instantiate(&mut store, &component, &linker)?;

        log::info!("app::{}: wasm component instantiated ({})", app_id, path.display());
        Ok(WasmApp { store, bindings })
    }

    pub fn init(&mut self, snapshot: &StateSnapshot, size: (f32, f32)) -> wasmtime::Result<Vec<Effect>> {
        self.bindings
            .plexi_platform_lifecycle()
            .call_init(&mut self.store, snapshot, size)
    }

    pub fn update(&mut self, event: &InputEvent) -> wasmtime::Result<Vec<Effect>> {
        self.bindings
            .plexi_platform_lifecycle()
            .call_update(&mut self.store, event)
    }

    pub fn view(&mut self) -> wasmtime::Result<UiTree> {
        self.bindings.plexi_platform_lifecycle().call_view(&mut self.store)
    }

    /// Current backing-store snapshot (host-side; does not call the guest).
    pub fn snapshot(&self) -> StateSnapshot {
        self.store.data().state.snapshot()
    }
}

// ─── Gate tests (G3 effect roundtrip, G5 state persistence) ────────────────
//
// These load the committed sysmon component fixture and drive the real
// wasmtime runtime — no subprocess, no rendering. Regenerate the fixture with
// `just wasm-fixtures` after changing the sysmon POC.
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wasm-fixtures/sysmon.wasm")
    }

    fn empty_snapshot() -> StateSnapshot {
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

    fn key(k: &str) -> InputEvent {
        InputEvent::Key(types::KeyEvent {
            key: k.to_string(),
            modifiers: types::Modifiers { ctrl: false, shift: false, alt: false, meta: false },
            pressed: true,
        })
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

    // G3: init -> timer-fired returns get-system-stats -> deliver stats -> view
    // shows the CPU percentage. The whole loop runs through the wasmtime host
    // with no subprocess.
    #[test]
    fn g3_effect_roundtrip() -> wasmtime::Result<()> {
        let mut app = WasmApp::load(
            "sysmon-g3",
            &fixture(),
            &Grants::standard_app(),
            StateStore::ephemeral(),
        )?;

        let startup = app.init(&empty_snapshot(), (400.0, 300.0))?;
        assert!(
            startup.iter().any(|e| matches!(e, Effect::GetSystemStats)),
            "init should request system stats"
        );

        let effects = app.update(&InputEvent::TimerFired(1))?;
        assert!(
            effects.iter().any(|e| matches!(e, Effect::GetSystemStats)),
            "timer-fired should request fresh system stats"
        );

        let after = app.update(&InputEvent::SystemStatsResult(mock_stats(42.0)))?;
        assert!(after.is_empty(), "stats-result produces no further effects");

        let tree = app.view()?;
        assert!(
            tree_text(&tree).contains("42.0%"),
            "view should render the delivered CPU percentage"
        );
        Ok(())
    }

    // G5: '=' x3 raises the poll interval to 5000ms, persisted through
    // host-state to disk. Reloading the store and re-initing yields a startup
    // timer at the persisted 5000ms interval.
    #[test]
    fn g5_state_persists_across_reload() -> wasmtime::Result<()> {
        let path = std::env::temp_dir()
            .join(format!("plexi-wasm-g5-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);

        {
            let store = StateStore::persistent(path.clone())?;
            let mut app = WasmApp::load(
                "sysmon-g5",
                &fixture(),
                &Grants::standard_app(),
                store,
            )?;
            app.init(&empty_snapshot(), (400.0, 300.0))?;
            // default 2000ms + 3 x 1000ms = 5000ms
            for _ in 0..3 {
                app.update(&key("="))?;
            }
        }

        let store = StateStore::persistent(path.clone())?;
        let snapshot = store.snapshot();
        let mut reloaded = WasmApp::load(
            "sysmon-g5-reload",
            &fixture(),
            &Grants::standard_app(),
            store,
        )?;
        let startup = reloaded.init(&snapshot, (400.0, 300.0))?;
        assert!(
            startup
                .iter()
                .any(|e| matches!(e, Effect::SetTimer(t) if t.delay_ms == 5000)),
            "reloaded app should start its poll timer at the persisted 5000ms interval"
        );

        let _ = std::fs::remove_file(&path);
        Ok(())
    }
}
