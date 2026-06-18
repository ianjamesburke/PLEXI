# Plexi WASM Runtime — Implementation Plan (host integration)

Companion to `wasm-runtime.md` (the architecture spec). That doc is the destination;
this is the build sequence and the concrete integration decisions for landing it on
the `wasm-rebuild` branch. Scope for this effort: **gates G1-G7 + G11-G13**. G8 (Python
compat), G9 (cloud), G10 (402 payment) are explicitly deferred to follow-on missions.

## Status

- **M1 — Toolchain + WIT build + pure-fn tests (G1, G2): DONE.**
  - `wit/plexi.wit` made valid (see commit `ac3e8172`): fixed `load-avg-1m`, multi-payload
    `pipe-error`, `list`/`f32` keyword collisions, `in_` underscore, and dropped
    self-referential fully-qualified world imports.
  - POC crates build to valid `wasm32-wasip2` components via `cargo component` 0.21.1 +
    `wit-bindgen` 0.41. `wasm-tools validate` exits 0 for all three.
  - Single-sourced platform WIT: each POC `wit/world.wit` is a symlink to `wit/plexi.wit`.
  - sysmon has 4 pure-function tests (`cargo test`, no host) — commit `df3e74fc`.
- **M2 — Host wasmtime integration (G3, G5): DONE.**
  - `src/host/wasm_app.rs`: `wasmtime` 43 + `wasmtime-wasi` 43; `bindgen!` the
    `plexi-app` world; host imports `host-log` (→ `log::app::<id>`), `host-state`
    (→ file-backed `StateStore`), `pipes` (stub until M4). Baseline WASI 0.2
    (clocks+random, no env/fs/net) linked so Rust-std components instantiate.
  - Capability linker: `host-log` always linked; `host-state`/`pipes` linked
    only when granted (`Grants`). Ungranted import → instantiation fails.
  - `WasmApp::{load,init,update,view,snapshot}` — synchronous Elm effect loop.
  - G3 `g3_effect_roundtrip` + G5 `g5_state_persists_across_reload` pass against
    a committed 124 KB release fixture `tests/wasm-fixtures/sysmon.wasm`
    (regenerate via `just wasm-fixtures`). Full `cargo test --bin plexi`: 1243+2
    pass. New module symbols are test-only until M3 wires `AppRuntime::Wasm`.
- **M3 — Rendering + run primitive (G4, G6): DONE.**
  - DONE `src/host/wasm_render.rs` (commit `9a205577`): arena `UiTree` -> egui
    renderer for every `ui-node-data` variant (GPU surface is a labelled
    placeholder until M4); depth-capped; interactions collected into
    `RenderResult`. Headless test renders the real sysmon tree without panic.
  - DONE `src/host/wasm_pane.rs` (commit `975c1a7d`): `WasmPane`, the
    synchronous effect-loop driver over `WasmApp` — queued input, ms-timers off
    frame time, `get-system-stats` via a pluggable `SystemStatsSource`,
    close/title/status surfacing, queue converges per `tick`. 4 driver tests
    pass against the sysmon fixture (init stats, poll refresh, `q` close, `=`
    retime to 5000ms).
  - DONE live integration (commits `bd56ddfb` wiring+G4, `6d051875` G6):
    1. `SysinfoStats` (added `sysinfo`) — production `SystemStatsSource`
       (cpu/mem/uptime/load; disk/net bps are 0 pending interval-delta tracking).
    2. `AppRuntime::Wasm(Box<LiveWasmPane>)` variant + all `pane.rs` arms; the
       five external exhaustive matches (`focus.rs`, `app/mod.rs` x3,
       `notification_image.rs`) gained a `Wasm` arm mirroring `Builtin`.
    3. `LiveWasmPane` (in `wasm_pane.rs`): the live adapter — owns the monotonic
       clock, lazy `init` on first frame, egui key translation (printable-vs-named
       split mirroring `process_app`), per-frame `tick`+`view`+`render_ui_tree`,
       timer-scheduled repaints (`request_repaint_after` off `next_deadline_ms`),
       and fatal-error capture (a bad app shows an error in place, never crashes
       the host).
    4. `open_wasm_app_pane` (`pane_ops/create.rs`): ephemeral spawn path, one
       `wasmtime::Store` per pane.
    5. Run primitive: `.wasm` paths route through the existing `app open <path>`
       -> `spawn_pane{path}` socket flow; `launch_app_by_path_with_layout`
       detects the `.wasm` extension and calls `open_wasm_app_pane`. No new
       top-level command (`plexi run` is the project-command runner; `plexi app
       open ./x.wasm` is the launch surface). State is ephemeral; `--persist` is
       deferred to M4.
    6. Gates: **G4** `tests/scenes/wasm-sysmon.toml` (live render of the sysmon
       fixture, asserts running + content + screenshot). **G6**
       `ui_tests::wasm_path_launch_opens_wasm_pane` (the shared launch entry
       produces an `AppRuntime::Wasm` pane). Both green; full suite 1249 pass.
  - Transient warnings (non-test build only): `StateStore::persistent` (G5
    infra), `LiveWasmPane::{is_running,last_render_text}` (G4 scene inspectors).
    Real code, test-reachable only; clears when `--persist` and a host-side wasm
    pane inspector land in M4. No `#[allow]` used.
- **M4: IN PROGRESS.** Pipes (G13 host + guest round-trip) and audio (G12) wired
  via world generalization; only the gpu world (G7/G11) remains. Plan below.

## Key facts discovered (load-bearing for the build)

- **Binary vs lib target.** `src/lib.rs` is a *minimal stub* lib used only by `gen_schema`/
  `gen_cli_docs`. The real host is the **binary target `src/main.rs`** (29 `mod`s incl.
  `process_app`, `host`, `render`, `testing`). New runtime modules + tests go in the binary
  target. Host tests run via `cargo test --bin plexi`.
- **App runtime enum:** `AppRuntime` in `src/host/pane.rs` (`Process(Box<ProcessApp>)` /
  `Builtin(Box<dyn App>)`). Add a third variant `Wasm(Box<WasmApp>)`; fan out its methods
  (`ui`, `handle_key`, `take_pending_commands`, ...) like the existing arms. **ProcessApp/PGAP
  stay untouched** — WasmApp lands alongside v1.
- **Host deps present:** egui 0.31 + eframe(wgpu) + `wgpu = "24"` (metal) + `egui-wgpu` 0.31
  (→ GPU/surface path for M4), `crossbeam-queue` 0.3 (→ pipes ArrayQueue, already used by
  `src/host/typed_pipes.rs`). **No tokio** — effects execute synchronously, not async.
- **Component output path:** cargo-component emits the component at
  `target/wasm32-wasip1/debug/<app>.wasm` (builds a wasip1 core module + adapts to a wasip2
  component). The gate docs say `wasm32-wasip2/` — tests must use the actual `wasip1` path.

## Integration decisions (reversible engineering calls, already made)

- **Host bindings:** `wasmtime::component::bindgen!` against `wit/plexi.wit`, per world.
  Start with `plexi-app` (sysmon); add `plexi-gpu-app` / `plexi-audio-app` in M4.
- **Runtime crate:** `wasmtime` (component-model + cranelift, sync). One `Store` per pane.
- **State store (G5):** minimal host-owned `StateStore` — `HashMap<String, Vec<u8>>` with
  get/set/delete/list-prefix/snapshot, persisted to a per-namespace file on write. No `sled`
  dep (CRDT `cas()` is a later concern; the primitive store is enough for G5). Persistent
  namespace = profile dir; ephemeral run = temp namespace deleted on pane close.
- **Effect loop:** Elm-style. `WasmApp::update(event) -> Vec<Effect>`; host executes effects
  and feeds results back as the next `input-event`. `GetSystemStats` uses the existing host
  stats source; `SetTimer`/`CancelTimer` via the host scheduler; `SetTitle`/`CloseSelf`
  become `AppCommand`s. Synchronous; long effects (http-fetch) run on a worker thread and
  post their result event back.

## Build sequence (each step ends green + committed)

- **M2 — Host wasmtime integration (G3, G5)**
  1. Add `wasmtime` dep; `mod wasm_app;` in `src/main.rs`.
  2. `bindgen!` the `plexi-app` world; implement host imports: `host-log` (→ `log::`),
     `host-state` (→ `StateStore`), `pipes` (→ `TypedPipeRegistry`, stub ok until M4).
  3. `WasmApp` struct: Store + instance + grants + StateStore + pending events. API:
     `load(path, grants)`, `init(snapshot, size) -> Vec<Effect>`, `update(event) -> Vec<Effect>`,
     `view() -> UiTree`.
  4. **Capability linker:** only link imports whose capability is granted (link-time gating).
  5. G3 test: load sysmon → `TimerFired` returns `GetSystemStats` → deliver
     `SystemStatsResult` → `view()` shows CPU%. No subprocess.
  6. G5 test: `=`×3 raises poll interval to 5000ms via `host-state`; reload from snapshot →
     startup timer is 5000ms.
- **M3 — Rendering + run primitive (G4, G6)**
  1. Scene graph: retained `UiTree` per pane; map `ui-node-data` variants → egui widgets
     (reuse `src/widgets.rs`/`src/style.rs`). Structural diff by `key`; repaint changed only.
  2. Wire `AppRuntime::Wasm` into pane render + key dispatch.
  3. `plexi run ./x.wasm` CLI (ephemeral temp namespace, deleted on close).
  4. G4 scene/screenshot test; G6 CLI + scene (open → populate → `q` closes, no namespace).
- **M4 — Surface/GPU + RT audio + pipes (G7, G11, G12, G13)**
  - DONE (commit `c165eba6`) **pipes host side**: `HostCtx` owns a per-app
    `TypedPipeRegistry` + handle map; `open`/`send-binary`/`send-json`/`close`/
    `is-connected` implemented; binary sends push the lock-free ring and report
    overrun as `Err` (no block/panic); registry `Drop` closes pipes on Store
    drop. Two host-side tests (`g13_pipes_open_send_overrun_close`,
    `g13_json_pipe_validation`). The full guest round-trip is below (needs the
    audio world).
  - DONE (commit `2ec7db69`) **world generalization + audio (G12)**: one
    `bindgen!` on `plexi-full-app` generates all shared types; the loader
    instantiates raw and builds per-interface export Guests (`lifecycle` always,
    `audio-rt-process` probed), so standard and audio apps share one path with no
    type duplication and no all-or-nothing world struct. `audio-rt-control`
    import backed by an `AudioStreams` registry; `WasmApp::audio_process_output`
    pulls the guest's RT export. **G12** (`g12_audio_process_output_produces_sound`):
    audio-synth is silent stopped, non-silent after `space`. Live RT output:
    `media::audio::start_output_stream` opens a cpal output device draining a
    lock-free `ArrayQueue<f32>`; the UI thread tops the ring each tick, the RT
    callback only pops (no Store/lock on the audio thread). Ephemeral run derives
    grants from the component's imports (`load_ephemeral_run`); gpu world fails
    fast. `audio-synth.wasm` fixture + recipe. Live device output is the manual
    leg (`just install`).
  - DONE (commit `cfd59e31`) **G13 guest round-trip**: the real audio-synth
    component, driven through wasmtime, opens its `waveform-out` binary pipe in
    init and pushes a decimated 256-byte preview from `process-output` each RT
    buffer; a peer connects to the unix socket as a client and receives the
    frames intact (u32-BE length-prefixed). Proves the full guest -> lock-free
    ring -> drain thread -> socket path. Test-only `binary_socket_path` /
    `WasmApp::pipe_socket_path` accessors back the listener harness.
    (`g13_guest_roundtrip_waveform_pipe`).
  - REMAINING:
    1. Bind `plexi-gpu-app`: `surface-node` lifecycle + `surface-ready`; the
       `gpu` import (WebGPU-aligned — WGSL pipelines, buffers, render/compute
       passes) on the host wgpu device shared with `egui-wgpu`. **G7**
       (pixel-buffer surface scene) then **G11** (GPU render pass <2ms). The
       loader already links `gpu` per-grant — only the host `gpu::Host` impl and
       surface wiring remain.
    2. Gates: G7, G11.
- **M5 — one PR `wasm-rebuild` → alpha + `just install`** for end-to-end manual test.

## Done definition

`cargo test --bin plexi` green + in-scope gates (G1-G7, G11-G13) passing, ProcessApp/PGAP
untouched, then a single PR and a local install for manual e2e.
