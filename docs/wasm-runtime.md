# Plexi WASM runtime

Status: active

Stint: see `wasm-runtime-impl-plan.md` for the build sequence.

`wit/plexi.wit` is the runtime contract. This page describes what the current host ships. It does not define new wire variants.

## Current runtime reference

Plexi runs a WASM component in one `wasmtime::Store` per app pane. The component exports `lifecycle.init`, `lifecycle.update`, and `lifecycle.view`. The host renders the returned `ui-tree`, delivers input events, and executes returned effects.

The current worlds are `plexi-app`, `plexi-gpu-app`, `plexi-audio-app`, and `plexi-full-app`. Their exact imports, exports, records, variants, and field names live in [`wit/plexi.wit`](../wit/plexi.wit).

### Effects and result events

The host executes these WIT effects:

- `get-system-stats`, `set-timer`, `cancel-timer`, `set-title`, `set-status`, and `close-self`
- `file-read`, `file-write`, `http-fetch`, `ai-query`, `declare-event-streams`, `emit-event`, `subscribe-event-streams`, and `unsubscribe-event-streams`
- `declare-tools` and `tool-result`
- `clipboard-read`, `clipboard-write`, `notify`, and `spawn`
- `request-capability`

Effects that need a response return it through the matching `input-event` result variant. `clipboard-read`, `clipboard-write`, `notify`, and `spawn` return `clipboard-*-result`, `notify-result`, and `spawn-result` after the host action completes.

`notify` posts a context-scoped host notification. Its optional icon is descriptive metadata; it does not load an image or grant file access. `spawn` opens a manifest-registered app with the requested layout and argv. It does not create a process or bypass app registry policy.

Apps subscribe to declared streams by publisher app ID and stream name. Matching
events enter the subscriber's `update` loop as `app-event`, regardless of
whether the publisher or subscriber is a Rust WASM component or a Python app.
Subscriptions last for the pane session and are removed when the subscriber
unsubscribes or closes. Cross-app reads pass through the unified event-stream
permission broker before the host records the subscription.

`declare-tools` registers a running component's typed tools in the same app
connector registry as Python apps. Assistant calls arrive as `tool-call`; the
component answers with `tool-result`. The Assistant permission broker filters
tool visibility and gates calls before they reach the component. Read-only
tools auto-grant unless an explicit deny applies; mutating tools require the
normal app-connector grant. Every invocation is audit logged.

### Capabilities

WASM components request session capabilities with `request-capability`. The host prompts the user for unknown requests, records remembered decisions by app and workspace, and returns `capability-granted` or `capability-denied` to the component.

Current runtime capability IDs include scoped filesystem and network forms plus `ai.query`, `state:read-write`, `pipe.open`, `gpu.render`, `audio.playback`, `audio:record`, `open-pane`, `spawn.app`, `clipboard.read`, `clipboard.write`, and `notify`. A protected effect without its grant returns an error result. The host logs every requested capability and every new effect execution.

Imported `host-state`, pipes, GPU, and audio interfaces are link-time gated. The component cannot call an interface the host did not link.

### Rendering, state, and host behavior

The host renders the WIT `ui-tree` through egui. Typed UI actions are delivered as `ui-action` and `ui-value-change` events. Surface nodes use a host-owned GPU texture. The state import provides persisted get, set, delete, and list-prefix operations scoped to the app. Typed pipes use host-managed handles.

The runtime is sandboxed. Components have isolated linear memory and only the host interfaces that Plexi links. Python apps run through the CPython-in-WASM adapter (`src/host/wasm_python.rs`, stint 0285) and are sandboxed by this same component boundary — there is no remaining native-subprocess Python runtime (`src/process_app/` was deleted).

## Performance

The reference workload is `apps/balls` — a continuous-cadence canvas app that moves every drawing command every frame, which is the worst realistic case for the render transport. Measured on alpha (2026-07-24), it holds its declared 60 fps: `guest_fps` ≈ 59.9 with a ~13 ms guest round-trip and ~0.05 ms of host time per pane paint. The frame path is guest-bound, not host-bound.

**How to measure.** Never guess at frame cost. The CPython-WASM adapter logs a sampled `CPython-WASM perf` line per app to the channel log (see the Logging section of the root `CLAUDE.md`), carrying `paint_fps`, `guest_fps`, `avg_host_ms`, `avg_roundtrip_ms`, per-stage decode/render times, and `stdout_kib`. Native WASM surface panes are paced and measured separately by `FrameClock` / `FrameTelemetry` in `src/host/wasm_frame.rs`. Read those numbers before changing anything; the two paths are independent and only the Python one carries today's apps.

**Reading the numbers.** `guest_fps` is the app's real cadence — the only number that answers "is it smooth". `avg_host_ms` is what the pane costs the host per paint. `avg_roundtrip_ms` is the guest's total budget per frame and must stay under the declared interval (16.7 ms at 60 fps). `paint_fps` counts host repaints, which legitimately exceeds `guest_fps`: the scheduler's admission deadline and the decoder's frame-arrival wake are independent repaint sources.

**Peak-performance SDK usage**, with balls as the exemplar:

- **Declare a rate you can sustain, and pace from the host.** Call `SetSchedulerMode("continuous", fps=N)` once in `init()` and let the host own cadence — fixed-deadline admission, a headroom window, and a bounded in-flight pipeline. Never implement an FPS loop, a sleep, or a frame counter in the guest. A steady lower rate reads smoother than a jittery higher one; if `guest_fps` sits below the declared rate, lower the declaration rather than shipping the jitter.
- **Integrate against the event's own `elapsed`, not the nominal tick.** Take `dt` from `RenderFrame.elapsed` and clamp it (balls uses a max-`dt` cap) so a stall or a tab-back cannot tunnel objects through walls. Physics must be dt-correct, not tick-count-correct.
- **Keep `view()` pure and cheap.** `view()` runs every frame and should only read simulation state into nodes. Do the simulation in `update()`; never mutate state, allocate caches, or do I/O inside `view()`.
- **Spend the frame budget on the command list, because it is the transport.** Every canvas command is JSON-encoded and written to stdout each frame, so the command count — not the pixel area — sets the cost. Balls draws a shadow, a body, and a highlight per ball, and its per-frame payload scales linearly with ball count. Bound the object count (balls caps at a fixed maximum) rather than letting the payload grow without limit.
- **Prefer a stable tree shape.** The runtime emits a positional `tree_delta` instead of a full frame while the tree keeps the same root and shape. Structural churn every frame — a changing node count, a rebuilt root — forces a full re-serialize. Note that a delta is only a win when part of the frame is static: for a canvas where every command changes, the delta is the same size as a full frame or slightly larger.

Both games under `apps/` declare 60 fps. Stint 0446 locked breakout to a lower rate as an explicit interim measure until 0438 (diffed tree updates) landed; 0438 is done, so 60 is the correct current declaration, not a reverted lock.

## Security Model

Every app pane — Rust WASM component or Python app — runs inside its own `wasmtime::Store` with isolated linear memory. A component can only reach the host interfaces Plexi links for its world (`plexi-app`, `plexi-gpu-app`, `plexi-audio-app`, `plexi-full-app`); this is link-time gating, not a runtime policy check. Python apps run through the CPython-in-WASM adapter (`src/host/wasm_python.rs`, stint 0285) inside the same component boundary — there is no separate native-subprocess Python runtime.

Beyond the link-time boundary, protected effects (`file-read`, `file-write`, `http-fetch`, `ai-query`, `pipe.open`, `gpu.render`, `audio.playback`, `audio:record`, `open-pane`, `spawn.app`, `clipboard.read`, `clipboard.write`, `notify`, and scoped filesystem/network forms) require an explicit capability grant. A component requests one with `request-capability`; the host prompts the user on first request, remembers the decision per app and workspace, and returns `capability-granted` or `capability-denied` on every subsequent request. A protected effect invoked without its grant returns an error result instead of executing. Every requested capability and every executed effect is logged.

**Trust labels.** `trust_label()` (`src/app/package.rs`) classifies a package before install:

| Label | When |
|---|---|
| `FirstPartyCore` | App id is in the bundled core pack |
| `SandboxedWasm` | `[app] type = "wasm"` entry |
| `PythonUnreviewed` / `ReviewedNative` | `.py` entry, outside the core pack — reviewed if `marketplace_reviewed` is set by the install flow (server attestation only; never in-package self-attestation) |

`marketplace_reviewed` gates only the trust label, not a sandbox choice. `FirstPartyCore`, `SandboxedWasm`, and Python entries all launch through the same wasmtime component boundary and capability grant flow described above — `PythonUnreviewed`/`ReviewedNative` for Python is a review-provenance label, not a weaker sandbox. A non-`.py`, non-WASM entry has no launch path at all: `ManifestType` only defines `App` (routed to the CPython-in-WASM adapter, which requires a `.py`/`.pyc` entry) and `Wasm`, so such a package is rejected at validate time (`PackageError::UnlaunchableEntry`) rather than classified with a trust label (stints 0411, #2412 removed the former `NativeUnreviewed` label and `PackageRuntime::Native`).

## Verification

`src/host/wasm_app.rs` and `src/host/wasm_pane.rs` contain component and effect-loop tests. `src/app/permissions.rs` validates accepted raw-WASM capability IDs. The WIT file is compiled by the Rust bindgen invocation in `wasm_app.rs`; changing it breaks the host build until every generated variant is handled.

## Roadmap

The items below are not current runtime guarantees.

- Stable frame pacing, shared Python/native telemetry, latest-frame presentation, and real-time canvas transport belong to [`realtime-app-runtime.md`](realtime-app-runtime.md).
- Registry resolution, publisher signatures, payments, and hosted marketplace execution belong to the marketplace work described in `docs/app-framework-marketplace.md`.
- Cloud execution, remote binary RPC, mobile runtime work, and CPython compatibility remain planned runtime work. See `wasm-runtime-impl-plan.md` for the sequencing document.
- File listing/watch, WebSocket, CRDT merge and state sync, payment events, and additional audio/video effects are not declared by the current WIT contract. They require a versioned WIT change and a tracked implementation task before documentation can call them supported.
