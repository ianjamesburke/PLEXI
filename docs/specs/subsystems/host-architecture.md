# Host Architecture

**Status:** Active (v3.0)
**Last updated:** 2026-04-18

---

## 1. The Core Principle

The Plexi host is two separate things that happen to run in the same process:

1. **HostModel** — a pure state machine. Takes commands, produces effects. Zero dependency on egui or any rendering library. Can be constructed and exercised in a plain `cargo test`.
2. **Renderer** — reads HostModel state and paints the screen. In production: egui + wgpu. In CI and the agent dev loop: tiny-skia headless PNG output.

Nothing in HostModel knows about pixels, frames, or GPU. Nothing in the renderer knows about business logic. The protocol between them is HostModel's state — a snapshot of which panes exist, which is focused, what effects were produced.

---

## 2. HostModel — The State Machine

### 2.1 Commands

Commands are the only way to mutate host state. They arrive from:
- The renderer (keyboard input, window events)
- App processes (PGAP protocol events)
- The agent pane (Plexi IQ turn output)
- The test harness (directly, in tests)

```
OpenPane(OpenPaneRequest)       — create a new pane (Terminal, App, or Agent)
CloseFocusedPane                — close whichever pane has focus
FocusPane(PaneId)               — focus a specific pane by ID
Navigate(Direction)             — move focus: Up | Down | Left | Right
SplitHorizontal                 — split focused pane, new pane below
SplitVertical                   — split focused pane, new pane to the right
NewContext                      — create a new workspace context (tab)
SwitchContext(index)            — switch to context by index
SendKeyToFocusedApp { key }     — route a key event to the focused App pane
SimulatePathChanged { pane_id, cwd } — trigger pane group broadcast
CheckCapability { pane_id, capability } — check capability against permission set
```

### 2.2 Effects

Effects are HostModel's observable output. The renderer subscribes to them to update the screen. The test harness collects them to assert on behavior.

```
PaneOpened { pane_id, kind }
PaneClosed { pane_id }
FocusChanged { pane_id: Option<PaneId> }
SplitOpened { pane_id, kind, placement }
ContextCreated { index }
ContextSwitched { index }
AppKeyDispatched { pane_id, key }
PathBroadcasted { group, cwd, recipient_pane_ids }
CapabilityGranted { pane_id, capability }
CapabilityDenied { pane_id, capability }
CapabilityPromptRequired { pane_id, capability }
EventEmitted(HostEvent)         — mirrors the event bus for test assertions
```

### 2.3 State

HostModel holds:

```
contexts: Vec<HostContext>      — one per workspace tab
active_context: usize
next_pane_id: PaneId
```

Each `HostContext` holds:

```
panes: Vec<HostPane>
focused_pane: Option<PaneId>
groups: HashMap<String, Vec<PaneId>>      — pane group membership
permissions: HashMap<PaneId, HashSet<Capability>>  — decided capabilities
```

Each `HostPane` holds:

```
id: PaneId
kind: PaneRuntimeKind           — Terminal | App { app_id } | Agent
declared_capabilities: Vec<Capability>   — from manifest.toml
```

### 2.4 No egui

HostModel has zero compile-time dependency on egui, eframe, or wgpu. This is a hard rule enforced at the module level — no `use egui::` anywhere in `src/host/`.

---

## 3. HostServices — The Seam for Mocking

Every real system boundary (filesystem, network, secrets, event log, subprocess spawning) passes through `HostServices`. In production, these are real implementations. In tests, they are swapped for mocks.

```rust
pub struct HostServices {
    pub event_sink: Box<dyn EventSink>,
    pub fs: Box<dyn FsService>,
    pub secrets: Box<dyn SecretsService>,
    // future: net, spawn
}
```

Each service is a trait. Test impls:

- `VecEventSink` — stores emitted `HostEvent`s in a `Vec`, readable by tests
- `MockFsService` — returns injected file contents; used to mock git data, config files, anything
- `MockSecretsService` — returns injected secret values or denies

This is how you test features that touch real systems: inject mock data at the HostServices boundary. The app under test never knows the difference — it receives the same PGAP events it would get from a real system call.

---

## 4. The Renderer Layer

The renderer is a pure consumer of HostModel state. It never mutates model state directly — it translates user input into `HostCommand`s and submits them.

### 4.1 Production renderer (egui + wgpu)

Lives in `src/app/`. Implements `eframe::App`. On each frame:
1. Drains any pending HostModel effects
2. Draws the pane tree using egui_tiles
3. Translates keyboard input to `HostCommand`s
4. Submits commands to HostModel
5. Applies effects (pane open/close, focus change)

### 4.2 Headless renderer (tiny-skia)

Lives in `src/headless_renderer.rs`. Used by the test harness and the agent dev loop.

Input: `Vec<DrawCommand>` + viewport size
Output: `Vec<u8>` PNG

Renders the same draw primitives the egui renderer handles:
- `Rect { x, y, w, h, color, corner_radius? }`
- `Text { x, y, text, size, color, monospace? }`
- `Line { x0, y0, x1, y1, color, width }`
- `Image { x, y, w, h, data_base64 }`

Does not render `VideoPlayer` or `AudioMeter` (these need real hardware or mock devices). In headless mode those commands are silently skipped or replaced with a placeholder rect.

Activated by `PLEXI_RENDER=headless` env flag.

**Determinism requirement:** Apps must not use real-time sources (`time.time()`, `random`, etc.) in their render function. The PGAP `Render` event includes `frame_timestamp` — apps should use this for any time-dependent rendering so that a test harness producing frame N at a fixed timestamp always gets the same output.

---

## 5. Security Model

### 5.1 What PGAP isolation actually guarantees

Apps run as child processes with:
- Piped stdio only — no inherited file descriptors
- No shared memory with the host
- No direct access to the egui context or any host-internal state

The only communication channel is the PGAP JSON stream on stdin/stdout and typed pipes for binary data. A PGAP-conformant app cannot access anything the host doesn't explicitly send it.

### 5.2 What it does not guarantee

A non-conformant app (rogue Python app that ignores the protocol) can still:
- Call `open()` / `os.path` directly to read the filesystem
- Make network calls via `urllib` or `requests`
- Spawn subprocesses

These bypass the capability model entirely. The protocol cannot stop a malicious app at the process level.

### 5.3 The WASM path (v3.1+ for Rust, later for Python)

WebAssembly closes this gap. A WASM module can **only** do what the host explicitly grants via WASM imports — enforced at the CPU level by the WASM runtime (Wasmtime). No filesystem, no network, no subprocess, unless the host's WASM imports expose them.

The PGAP interface is designed to map cleanly to WASM component model exports:

| PGAP subprocess | WASM component equivalent |
|---|---|
| Init JSON on stdin | `init(app_id, workspace_root, capabilities) -> Ready` |
| Render JSON on stdin | `render(frame_id, w, h) -> Vec<DrawCommand>` |
| Key JSON on stdin | `on_key(key, modifiers)` |
| Shutdown JSON on stdin | `shutdown()` |

When the WASM toolchain matures for Rust (v3.1) and Python (later), the transport changes from subprocess+JSON to WASM function call. The protocol semantics are identical. Apps written for the subprocess model do not need to change their logic, only their build target.

**For v3.0:** Python subprocess. Honor-system security is acceptable for the curated app set shipping with Plexi and for apps distributed through a trusted channel. Community distribution of arbitrary apps requires WASM.

---

## 6. Multi-Agent Workflows

Multiple agent panes can run simultaneously. They coordinate via:

1. **spawn.app capability** — Agent A calls `Notify` or triggers a `RunGet` that causes the host to open a new `Pane::App` or `Pane::Agent`. Agent B gets its own pane with its own capability set.
2. **Typed pipes** — Agent A opens a `PipeOpen { pipe_id, mode: Json, direction: Duplex }`. The host routes `PipeSend` from A to B and back. Neither agent shares memory with the other.
3. **Per-agent capability sets** — B's capability set is independently declared in B's manifest and independently enforced by the host. A having `fs.write` does not give B `fs.write`.

The host mediates all cross-agent communication. Agents cannot communicate directly.

This maps to what Cloudflare Workers provides with Durable Objects + bindings — each agent is isolated, communication goes through the host, capability scope is per-agent. The Plexi version is local-first and synchronous rather than edge-distributed.

---

## 7. Files

| File | Purpose |
|---|---|
| `src/host/model.rs` | HostModel state machine |
| `src/host/command.rs` | HostCommand enum |
| `src/host/effect.rs` | HostEffect enum |
| `src/host/services.rs` | HostServices + trait definitions |
| `src/host/harness.rs` | HostHarness test driver |
| `src/headless_renderer.rs` | tiny-skia PNG renderer (no egui) |
| `src/app/mod.rs` | egui production renderer (eframe::App) |
