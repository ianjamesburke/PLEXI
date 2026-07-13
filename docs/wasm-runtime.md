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
- `file-read`, `file-write`, `http-fetch`, `ai-query`, `declare-event-streams`, and `emit-event`
- `clipboard-read`, `clipboard-write`, `notify`, and `spawn`
- `request-capability`

Effects that need a response return it through the matching `input-event` result variant. `clipboard-read`, `clipboard-write`, `notify`, and `spawn` return `clipboard-*-result`, `notify-result`, and `spawn-result` after the host action completes.

`notify` posts a context-scoped host notification. Its optional icon is descriptive metadata; it does not load an image or grant file access. `spawn` opens a manifest-registered app with the requested layout and argv. It does not create a process or bypass app registry policy.

### Capabilities

WASM components request session capabilities with `request-capability`. The host prompts the user for unknown requests, records remembered decisions by app and workspace, and returns `capability-granted` or `capability-denied` to the component.

Current runtime capability IDs include scoped filesystem and network forms plus `ai.query`, `state:read-write`, `pipe.open`, `gpu.render`, `audio.playback`, `audio:record`, `open-pane`, `spawn.app`, `clipboard.read`, `clipboard.write`, and `notify`. A protected effect without its grant returns an error result. The host logs every requested capability and every new effect execution.

Imported `host-state`, pipes, GPU, and audio interfaces are link-time gated. The component cannot call an interface the host did not link.

### Rendering, state, and host behavior

The host renders the WIT `ui-tree` through egui. Typed UI actions are delivered as `ui-action` and `ui-value-change` events. Surface nodes use a host-owned GPU texture. The state import provides persisted get, set, delete, and list-prefix operations scoped to the app. Typed pipes use host-managed handles.

The runtime is sandboxed. Components have isolated linear memory and only the host interfaces that Plexi links. Native Python apps remain a separate runtime and are not sandboxed by this component boundary.

## Verification

`src/host/wasm_app.rs` and `src/host/wasm_pane.rs` contain component and effect-loop tests. `src/app/permissions.rs` validates accepted raw-WASM capability IDs. The WIT file is compiled by the Rust bindgen invocation in `wasm_app.rs`; changing it breaks the host build until every generated variant is handled.

## Roadmap

The items below are not current runtime guarantees.

- Registry resolution, publisher signatures, payments, and hosted marketplace execution belong to the marketplace work described in `docs/app-framework-marketplace.md`.
- Cloud execution, remote binary RPC, mobile runtime work, and CPython compatibility remain planned runtime work. See `wasm-runtime-impl-plan.md` for the sequencing document.
- File listing/watch, WebSocket, CRDT merge and state sync, payment events, and additional audio/video effects are not declared by the current WIT contract. They require a versioned WIT change and a tracked implementation task before documentation can call them supported.
