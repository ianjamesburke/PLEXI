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
        world: "plexi-full-app",
    });
}

use bindings::exports::plexi::platform::audio_rt_process::{
    Guest as AudioProcessGuest, GuestIndices as AudioProcessIndices,
};
use bindings::exports::plexi::platform::lifecycle::{
    Guest as LifecycleGuest, GuestIndices as LifecycleIndices,
};
use bindings::plexi::platform::{audio_rt_control, host_log, host_state, pipes, types};

pub use types::{
    Alignment, BadgeColor, ButtonStyle, Color, Effect, IndexedNode, InputEvent, KeyEvent,
    Modifiers, StateSnapshot, SystemStats, UiNodeData, UiTree,
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

/// A pipe the guest opened, keyed by the `pipe-handle` it was given. Holds the
/// registry id and kind so sends route to the right backing (binary ring vs.
/// JSON validation).
struct PipeBinding {
    id: String,
    binary: bool,
}

pub struct HostCtx {
    app_id: String,
    state: StateStore,
    // Per-app typed-pipe registry backing the `pipes` import (G13). Binary
    // pipes get a unix socket + drain thread; the registry's Drop closes them
    // when the pane (and its Store) is dropped.
    pipes: crate::host::typed_pipes::TypedPipeRegistry,
    pipe_handles: HashMap<u32, PipeBinding>,
    next_pipe_handle: u32,
    // Open audio streams (G12). Backs the `audio-rt-control` import; sample
    // pulls happen through the guest's `process-output` export, not here.
    audio_streams: AudioStreams,
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

impl HostCtx {
    fn pipe_id(&self, handle: u32) -> Result<&PipeBinding, String> {
        self.pipe_handles
            .get(&handle)
            .ok_or_else(|| format!("unknown pipe handle {handle}"))
    }
}

impl pipes::Host for HostCtx {
    fn open(
        &mut self,
        id: String,
        kind: pipes::PipeType,
        direction: pipes::PipeDirection,
    ) -> Result<u32, String> {
        use crate::host::typed_pipes::PipeDirection as Dir;
        let dir = match direction {
            pipes::PipeDirection::In => Dir::In,
            pipes::PipeDirection::Out => Dir::Out,
            pipes::PipeDirection::Duplex => Dir::Duplex,
        };
        let binary = matches!(kind, pipes::PipeType::Binary);
        if binary {
            self.pipes
                .open_binary(id.clone(), dir)
                .map_err(|e| e.to_string())?;
        } else {
            self.pipes
                .open_json(id.clone(), dir)
                .map_err(|e| e.to_string())?;
        }
        let handle = self.next_pipe_handle;
        self.next_pipe_handle += 1;
        self.pipe_handles.insert(handle, PipeBinding { id: id.clone(), binary });
        log::info!("app::{}: opened {} pipe '{id}' -> handle {handle}", self.app_id, if binary { "binary" } else { "json" });
        Ok(handle)
    }

    fn send_binary(&mut self, handle: u32, payload: Vec<u8>) -> Result<(), String> {
        let binding = self.pipe_id(handle)?;
        if !binding.binary {
            return Err(format!("pipe handle {handle} is a json pipe"));
        }
        let ring = self
            .pipes
            .binary_ring(&binding.id)
            .ok_or_else(|| format!("pipe '{}' has no binary ring", binding.id))?;
        // Lock-free push; a full ring means the peer is behind — drop the frame
        // (RT-safe, best-effort) and report overrun to the guest.
        ring.push(payload)
            .map_err(|_| "pipe ring full (overrun)".to_string())
    }

    fn send_json(&mut self, handle: u32, json: String) -> Result<(), String> {
        let (id, binary) = {
            let b = self.pipe_id(handle)?;
            (b.id.clone(), b.binary)
        };
        if binary {
            return Err(format!("pipe handle {handle} is a binary pipe"));
        }
        let value = serde_json::from_str::<serde_json::Value>(&json)
            .map_err(|e| format!("invalid json: {e}"))?;
        self.pipes.send_json(&id, value).map_err(|e| e.to_string())
    }

    fn close(&mut self, handle: u32) -> Result<(), String> {
        let binding = self
            .pipe_handles
            .remove(&handle)
            .ok_or_else(|| format!("unknown pipe handle {handle}"))?;
        self.pipes.close(&binding.id);
        Ok(())
    }

    fn is_connected(&mut self, handle: u32) -> bool {
        match self.pipe_handles.get(&handle) {
            Some(b) if b.binary => !self.pipes.drain_failed(&b.id),
            Some(b) => self.pipes.has_reader(&b.id),
            None => false,
        }
    }
}

// ─── Audio streams (G12) ────────────────────────────────────────────────────
//
// Backs the `audio-rt-control` import. Opening a stream registers its config;
// the host's RT thread (live path) and the synchronous G12 gate both pull
// samples by calling the guest's exported `process-output` via
// `WasmApp::audio_process_output` — not through this registry.

#[derive(Default)]
struct AudioStreams {
    streams: HashMap<u32, audio_rt_control::AudioConfig>,
    next: u32,
}

impl audio_rt_control::Host for HostCtx {
    fn open_output(&mut self, config: audio_rt_control::AudioConfig) -> Result<u32, String> {
        let handle = self.audio_streams.next;
        self.audio_streams.next += 1;
        log::info!(
            "app::{}: audio open_output stream {handle} ({} Hz, {} ch, {} frames)",
            self.app_id, config.sample_rate, config.channels, config.buffer_frames
        );
        self.audio_streams.streams.insert(handle, config);
        Ok(handle)
    }

    fn open_input(&mut self, config: audio_rt_control::AudioConfig) -> Result<u32, String> {
        let handle = self.audio_streams.next;
        self.audio_streams.next += 1;
        log::info!("app::{}: audio open_input stream {handle}", self.app_id);
        self.audio_streams.streams.insert(handle, config);
        Ok(handle)
    }

    fn stream_config(
        &mut self,
        handle: u32,
    ) -> Result<audio_rt_control::AudioConfig, String> {
        self.audio_streams
            .streams
            .get(&handle)
            .cloned()
            .ok_or_else(|| format!("unknown audio stream {handle}"))
    }

    fn pause(&mut self, handle: u32) -> Result<(), String> {
        if self.audio_streams.streams.contains_key(&handle) {
            Ok(())
        } else {
            Err(format!("unknown audio stream {handle}"))
        }
    }

    fn resume(&mut self, handle: u32) -> Result<(), String> {
        if self.audio_streams.streams.contains_key(&handle) {
            Ok(())
        } else {
            Err(format!("unknown audio stream {handle}"))
        }
    }

    fn close(&mut self, handle: u32) -> Result<(), String> {
        self.audio_streams.streams.remove(&handle);
        log::info!("app::{}: audio stream {handle} closed", self.app_id);
        Ok(())
    }
}

// ─── WasmApp ───────────────────────────────────────────────────────────────

pub struct WasmApp {
    store: Store<HostCtx>,
    lifecycle: LifecycleGuest,
    // Some only when the component exports `audio-rt-process` (audio/full worlds).
    audio: Option<AudioProcessGuest>,
}

/// Derive the capability grants a component needs from the interfaces it
/// imports. Used by the ephemeral `run` path, where there is no manifest or
/// grant prompt yet: a locally-run component is trusted to receive exactly the
/// host interfaces it declares. `load` (explicit grants) remains the
/// capability-enforcing entry used by gates and installed apps.
fn grants_from_component(engine: &Engine, component: &Component) -> Grants {
    let mut grants = Grants::default();
    for (name, _) in component.component_type().imports(engine) {
        if name.contains("host-state") {
            grants.state = true;
        } else if name.contains("/pipes") {
            grants.pipes = true;
        } else if name.contains("audio-rt-control") {
            grants.audio = true;
        } else if name.contains("/gpu") {
            grants.gpu = true;
        }
    }
    grants
}

impl WasmApp {
    fn engine_and_component(path: &Path) -> wasmtime::Result<(Engine, Component)> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        let engine = Engine::new(&config)?;
        let component = Component::from_file(&engine, path)?;
        Ok((engine, component))
    }

    /// Load for an ephemeral `plexi run`: grants are derived from the
    /// component's declared imports rather than passed in. The WASM sandbox
    /// still contains the app; this only decides which host interfaces are
    /// linked before the capability-prompt UI exists.
    pub fn load_ephemeral_run(
        app_id: impl Into<String>,
        path: &Path,
        state: StateStore,
    ) -> wasmtime::Result<Self> {
        let app_id = app_id.into();
        let (engine, component) = Self::engine_and_component(path)?;
        let grants = grants_from_component(&engine, &component);
        log::info!("app::{app_id}: ephemeral run grants {grants:?}");
        Self::instantiate(app_id, &engine, &component, &grants, state)
    }

    fn instantiate(
        app_id: String,
        engine: &Engine,
        component: &Component,
        grants: &Grants,
        state: StateStore,
    ) -> wasmtime::Result<Self> {
        let mut linker = Linker::<HostCtx>::new(engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
        host_log::add_to_linker::<_, HasSelf<HostCtx>>(&mut linker, |c| c)?;
        if grants.state {
            host_state::add_to_linker::<_, HasSelf<HostCtx>>(&mut linker, |c| c)?;
        }
        if grants.pipes {
            pipes::add_to_linker::<_, HasSelf<HostCtx>>(&mut linker, |c| c)?;
        }
        if grants.gpu {
            // The gpu host interface lands with G7/G11. Fail fast with a clear
            // message rather than an opaque unsatisfied-import error below.
            return Err(wasmtime::Error::msg(format!(
                "app::{app_id}: gpu capability not yet supported (G7/G11)"
            )));
        }
        if grants.audio {
            audio_rt_control::add_to_linker::<_, HasSelf<HostCtx>>(&mut linker, |c| c)?;
        }

        let ctx = HostCtx {
            app_id: app_id.clone(),
            state,
            pipes: crate::host::typed_pipes::TypedPipeRegistry::new(
                crate::config::config_dir().join("pipes"),
            ),
            pipe_handles: HashMap::new(),
            next_pipe_handle: 0,
            audio_streams: AudioStreams::default(),
            wasi: WasiCtxBuilder::new().build(),
            table: ResourceTable::new(),
        };

        // Instantiate raw, then build per-interface export accessors. The world
        // struct's instantiate would demand every export of `plexi-full-app`;
        // standard apps only export `lifecycle`, so build that unconditionally
        // and probe for the optional `audio-rt-process` export.
        let pre = linker.instantiate_pre(component)?;
        let lifecycle_idx = LifecycleIndices::new(&pre)?;
        let audio_idx = AudioProcessIndices::new(&pre).ok();

        let mut store = Store::new(engine, ctx);
        let instance = pre.instantiate(&mut store)?;
        let lifecycle = lifecycle_idx.load(&mut store, &instance)?;
        let audio = match audio_idx {
            Some(idx) => Some(idx.load(&mut store, &instance)?),
            None => None,
        };

        log::info!(
            "app::{}: wasm component instantiated (audio={})",
            app_id,
            audio.is_some()
        );
        Ok(WasmApp { store, lifecycle, audio })
    }

    pub fn init(&mut self, snapshot: &StateSnapshot, size: (f32, f32)) -> wasmtime::Result<Vec<Effect>> {
        self.lifecycle.call_init(&mut self.store, snapshot, size)
    }

    pub fn update(&mut self, event: &InputEvent) -> wasmtime::Result<Vec<Effect>> {
        self.lifecycle.call_update(&mut self.store, event)
    }

    pub fn view(&mut self) -> wasmtime::Result<UiTree> {
        self.lifecycle.call_view(&mut self.store)
    }

    /// Pull one buffer of output samples from the guest's RT `process-output`
    /// export. Returns interleaved f32 samples (len = buffer_frames * channels)
    /// and the threaded `u64` state slot. Errors on a non-audio component.
    pub fn audio_process_output(
        &mut self,
        handle: u32,
        buffer_frames: u32,
        channels: u32,
        sample_rate: u32,
        state: u64,
    ) -> wasmtime::Result<(Vec<f32>, u64)> {
        let audio = self
            .audio
            .as_ref()
            .ok_or_else(|| wasmtime::Error::msg("audio_process_output on a non-audio app"))?;
        audio.call_process_output(
            &mut self.store,
            handle,
            buffer_frames,
            channels,
            sample_rate,
            state,
        )
    }

    /// Config of the first audio output stream the guest opened via
    /// `audio-rt-control`, as `(stream_handle, sample_rate, channels,
    /// buffer_frames)`. `None` until the guest opens one (or for non-audio
    /// apps). The live pane uses this to start the host output stream.
    pub fn output_stream_config(&self) -> Option<(u32, u32, u32, u32)> {
        let streams = &self.store.data().audio_streams;
        streams
            .streams
            .iter()
            .min_by_key(|(handle, _)| **handle)
            .map(|(handle, cfg)| (*handle, cfg.sample_rate, cfg.channels, cfg.buffer_frames))
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

    fn audio_fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wasm-fixtures/audio-synth.wasm")
    }

    fn empty_snapshot() -> StateSnapshot {
        StateSnapshot { entries: vec![] }
    }

    fn host_ctx_with_pipes() -> HostCtx {
        // Unix socket paths must be < ~104 bytes (SUN_LEN); the macOS
        // /var/folders temp dir + a UUID socket name overflows it, so use a
        // short /tmp path for the bind to succeed.
        let dir = PathBuf::from("/tmp").join(format!("plpipe{}", std::process::id()));
        HostCtx {
            app_id: "pipes-test".to_string(),
            state: StateStore::ephemeral(),
            pipes: crate::host::typed_pipes::TypedPipeRegistry::new(dir),
            pipe_handles: std::collections::HashMap::new(),
            next_pipe_handle: 0,
            audio_streams: AudioStreams::default(),
            wasi: WasiCtxBuilder::new().build(),
            table: ResourceTable::new(),
        }
    }

    // G13 (host side): the pipes import is backed by TypedPipeRegistry. A binary
    // pipe opens, accepts frames into the lock-free ring, rejects json-on-binary,
    // and reports overrun (full ring) as an error instead of panicking/blocking.
    #[test]
    fn g13_pipes_open_send_overrun_close() {
        use pipes::{Host, PipeDirection, PipeType};
        let mut ctx = host_ctx_with_pipes();

        let h = ctx
            .open("waveform-out".to_string(), PipeType::Binary, PipeDirection::Out)
            .expect("open binary");
        assert!(ctx.is_connected(h), "freshly opened pipe is healthy");
        ctx.send_binary(h, vec![1, 2, 3, 4]).expect("first frame fits");

        // No peer is draining the ring, so it fills at capacity; further pushes
        // must return an overrun error — never panic, never block.
        let mut overran = false;
        for _ in 0..1024 {
            if ctx.send_binary(h, vec![0u8; 64]).is_err() {
                overran = true;
                break;
            }
        }
        assert!(overran, "a full ring must surface overrun as Err");

        // json send on a binary pipe is rejected.
        assert!(ctx.send_json(h, "{}".to_string()).is_err());
        ctx.close(h).expect("close");
        assert!(!ctx.is_connected(h), "closed handle is not connected");
        assert!(ctx.send_binary(h, vec![0]).is_err(), "send after close errors");
    }

    // A json pipe round-trips through validation: valid json is accepted,
    // malformed json and binary-on-json are rejected.
    #[test]
    fn g13_json_pipe_validation() {
        use pipes::{Host, PipeDirection, PipeType};
        let mut ctx = host_ctx_with_pipes();
        let h = ctx
            .open("score".to_string(), PipeType::Json, PipeDirection::Out)
            .expect("open json");
        ctx.send_json(h, r#"{"score":3}"#.to_string()).expect("valid json");
        assert!(ctx.send_json(h, "{not json".to_string()).is_err());
        assert!(ctx.send_binary(h, vec![0]).is_err(), "binary on json pipe errors");
        ctx.close(h).expect("close");
    }

    // G12 (host side): a plexi-audio-app component links audio-rt-control and
    // exports audio-rt-process. After the guest starts playing, the host pulls
    // a buffer via process-output and gets non-silent interleaved samples. The
    // whole loop runs synchronously through wasmtime — no OS audio thread, which
    // is exactly the spec's automated pass condition (RT-thread priority and
    // <10ms latency are measured separately by the live path).
    #[test]
    fn g12_audio_process_output_produces_sound() -> wasmtime::Result<()> {
        // Grants are derived from the component's imports; audio-synth imports
        // audio-rt-control, so the audio capability is linked.
        let mut app = WasmApp::load_ephemeral_run(
            "audio-synth-g12",
            &audio_fixture(),
            StateStore::ephemeral(),
        )?;
        assert!(app.audio.is_some(), "audio-synth must export audio-rt-process");

        app.init(&empty_snapshot(), (400.0, 300.0))?;

        // Before play, the envelope is at zero — output is silent.
        let (silent, _) = app.audio_process_output(0, 512, 2, 48_000, 0)?;
        assert_eq!(silent.len(), 512 * 2, "interleaved buffer = frames * channels");
        assert!(
            silent.iter().all(|&s| s.abs() <= 0.01),
            "stopped synth must be silent"
        );

        // Space toggles play; pump several buffers so the amplitude ramp opens.
        app.update(&key("space"))?;
        let mut state = 0u64;
        let mut peak = 0.0f32;
        for _ in 0..32 {
            let (samples, next) = app.audio_process_output(0, 512, 2, 48_000, state)?;
            state = next;
            peak = peak.max(samples.iter().fold(0.0, |m, &s| m.max(s.abs())));
        }
        assert!(peak > 0.01, "playing synth must produce audible samples (peak={peak})");
        Ok(())
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
        let mut app = WasmApp::load_ephemeral_run(
            "sysmon-g3",
            &fixture(),
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
            let mut app = WasmApp::load_ephemeral_run("sysmon-g5", &fixture(), store)?;
            app.init(&empty_snapshot(), (400.0, 300.0))?;
            // default 2000ms + 3 x 1000ms = 5000ms
            for _ in 0..3 {
                app.update(&key("="))?;
            }
        }

        let store = StateStore::persistent(path.clone())?;
        let snapshot = store.snapshot();
        let mut reloaded = WasmApp::load_ephemeral_run("sysmon-g5-reload", &fixture(), store)?;
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
