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
- **M3-M4: not started.** Plan below.

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
  1. `surface-node`: allocate wgpu texture, send `surface-ready`, expose `gpu` import
     (WebGPU-aligned: pipelines from WGSL, buffers, render/compute passes) on host wgpu device.
  2. RT audio: `audio-rt-control` import + `audio-rt-process` export called from the OS audio
     thread; u64 state threading; <10ms latency.
  3. Pipes: expose `TypedPipeRegistry` via the `pipes` WIT import end-to-end.
  4. Gates: G7 (bevy-pong surface), G11 (GPU render pass <2ms), G12 (audio non-silent <10ms),
     G13 (pipe roundtrip + overrun drop without crash).
- **M5 — one PR `wasm-rebuild` → alpha + `just install`** for end-to-end manual test.

## Done definition

`cargo test --bin plexi` green + in-scope gates (G1-G7, G11-G13) passing, ProcessApp/PGAP
untouched, then a single PR and a local install for manual e2e.
