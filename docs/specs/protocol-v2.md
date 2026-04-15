# Plexi Protocol v2 — The Agent-Native Release

**Status:** Draft
**Last updated:** 2026-04-15
**Owner:** plexi-core
**Target:** Plexi 2.0 — 3 months from doc date

---

## TL;DR

Plexi v1's protocol is solid for single-app rendering and parent↔child composition (see `app-infrastructure.md`, `typed-pipes.md` Phase 0). It is not sufficient for the agent-native vision in `VISION.md` because it lacks four load-bearing primitives:

1. **Structured spawn intent** — no in-protocol way to say *why* an app was opened (which file, which prompt, which caller).
2. **A host-side event bus** — no way for any app to observe what other apps are doing, which blocks every cross-cutting feature: agent flow visualizer, trust learning, replay testing, attention queue.
3. **A Run primitive** — no host-side concept of a stateful multi-step task that can be blocked on user input. Notifications are fire-and-forget; agent jobs have nowhere to live between turns.
4. **Rich notifications** — action payloads are underspecified; notifications cannot wrap or resume a Run.

Everything else Plexi needs for v2 — Plexi IQ Stage 1, typed pipes Phase 1, capability enforcement, directory-scoped sandboxing — is already spec'd or partially implemented. This doc is an **index + gap-fill**, not a replacement. It references existing specs by file, defines the two genuinely new primitives (`OpenIntent`, `Run`) inline, resolves four contradictions between existing specs, and publishes the ship order.

The explicit design constraint: **the SDK barely changes.** The host gets smarter; apps stay dumb. That matches the "one install, three interfaces" non-negotiable in `VISION.md` and the zero-dependency SDK rule from DEV_LOG.

---

## 1. Scope and Non-Goals

### In scope for Plexi 2.0

- Init `OpenIntent` payload (new, this doc)
- Host event bus / `.plexi` event log (#91, this doc formalizes)
- `Run` primitive (new, this doc)
- Rich notification action payloads (#218/#219/#221, this doc closes spec gap)
- Plexi IQ Stage 1 in-host orchestrator (#210/#212)
- Typed pipes Phase 1 (`typed-pipes.md`)
- Capability enforcement pass (`app_permissions.rs` → runtime prompt)
- **Input layering contract** — host-owned priority stack for keyboard routing (§7.5, resolves #240/#236 class of alpha-bugs)
- Protocol version negotiation (new, trivial)

### Explicitly deferred to v2.1+

- PGAP intelligence gateway (#213, `intelligence-protocol.md`) — v2 keeps `claude -p --resume` subprocess
- Trust/risk float learning (`agent-orchestration.md` §4) — v2 uses binary Yes once/Yes always/No prompts
- Agent replay testing (`agent-replay-testing.md`)
- WASM/PWA deployment (`wasm-pwa-deployment.md`)
- SpacetimeDB sync (`sync-architecture.md`)
- Chat primitive (`chat-primitive.md`)
- Core text editor primitive (`core-text-editor-primitive.md`)
- Advanced UI SDK egui widgets (`core-advanced-ui-sdk.md`, #132)
- Spatial canvas Option B/C (`spatial-canvas.md`) — v2 stays on Option A as background
- Spawn `SpawnLifecycle::Prompt` — stays stubbed as `Orphan`

### Non-goals — will never be in protocol v2

- A generic RPC layer between arbitrary apps. Request/Reply stays scoped to `app_api.rs` (host-mediated capability calls). App↔app data flows through typed pipes.
- A chat/message bus primitive. Typed pipes are the only composition primitive.
- Host-side workflow orchestration. Runs are dumb containers; orchestration lives in Plexi IQ.

---

## 2. The Existing Protocol Surface (v1)

This is the ground truth as of 2026-04-14. Anything not listed here does not exist.

| Subsystem | File | Purpose |
|---|---|---|
| Wire types | `src/app_protocol.rs` | `PlexiEvent`, `DrawCommand`, spawn/pipe/notification primitives |
| Subprocess host | `src/process_app.rs` | Spawns app binaries, JSON-over-stdio |
| Pane dispatch | `src/pane_ops.rs` | Routes events to panes, validates spawn/pipe |
| Manifest | `src/app_registry.rs` | Loads `manifest.toml` from global + `.plexi/apps/` |
| Permissions | `src/app_permissions.rs` | Trust level + FsPermission; declarative |
| Host API | `src/app_api.rs` | Structured `ApiRequest`/`ApiResponse` over dedicated channel |
| Notifications | `src/notification_log.rs`, `src/notification_palette.rs`, `src/notify_socket.rs` | JSONL log, Cmd+Shift+N UI, Unix socket ingestion |
| Palette | `src/command_palette.rs` | Cmd+P, lists panes + registry apps |
| Agent mode | `src/agent_mode.rs` | Per-pane Ctrl+/ overlay; not yet on protocol |
| Python SDK | `sdk/python/plexi_sdk.py` | Stdlib-only, vendored per app |
| Rust SDK | `sdk/rust/src/lib.rs` | Mirrors Python, lags by ~6 months |
| Manifest schema | `schemas/plexi-manifest-schema.json` | JSON Schema validation |

### Protocol events (v1, from `src/app_protocol.rs`)

`PlexiEvent`: `Init`, `Render`, `Resize`, `Key`, `Click`, `MouseDown`, `MouseUp`, `MouseMove`, `Scroll`, `Command`, `Drop`, `GetState`, `SetState`, `PipeData`, `Shutdown`.

`DrawCommand`: `Rect`, `Text`, `Line`, `List`, `Image`, `VideoThumbnail`, `FileGrid`, `RunInTerminal`, `Cd`, `Log`, `State`, `CostReport`, `Notification`, `DropTarget`, `SetCursor`, `MouseTracking`, `SpawnApp`, `PipeWrite`, `PipeSubscribe`, `FrameDone`.

v2 adds: `OpenIntent` field on `Init`, `RunCreate`/`RunUpdate`/`RunComplete` draw commands, `RunEvent` PlexiEvent, `EventSubscribe` draw command, `Event` PlexiEvent, and a `protocol_version` handshake on `Init`. Everything else is unchanged.

---

## 3. New Primitive: `OpenIntent`

### Problem

When the palette, CLI, or another app launches an app, the subprocess receives only width/height/dpi on `Init`. Intent — which file, which prompt, which parent, which payload — rides on argv and env vars via ad-hoc conventions. This blocks:

- File explorer as a regular protocol-driven app (cannot return a selection)
- `plexi launch text-editor foo.md` having standard "open this file" semantics
- Agent delegation carrying a structured prompt
- Palette → app handoff with typed context
- Notification action → app resume with a Run id

### Design

Add one optional field to `PlexiEvent::Init`:

```rust
pub struct Init {
    pub width: f32,
    pub height: f32,
    pub pixels_per_point: f32,
    pub protocol_version: u32,        // new in v2, default 2
    pub open_intent: Option<OpenIntent>, // new in v2
}

pub struct OpenIntent {
    pub kind: OpenKind,
    pub caller: Option<Caller>,
    pub payload: Option<serde_json::Value>,
    pub run_id: Option<String>,
}

pub enum OpenKind {
    File { path: PathBuf, range: Option<TextRange> },
    Url { url: String },
    Prompt { text: String, model_hint: Option<String> },
    Resume { snapshot_key: String },
    Bare, // palette open with no intent
}

pub struct Caller {
    pub app_id: String,
    pub pane_id: Option<String>,
    pub source: CallerSource, // Palette, Cli, Spawn, Notification, AgentMode, ApiCall
}
```

### Rules

1. `OpenIntent` is advisory. An app may ignore it. A text editor receiving `Bare` opens a blank document. A text editor receiving `File { path }` opens that file.
2. Only the host constructs `OpenIntent`. Apps requesting a spawn pass an `OpenIntent` to `SpawnApp`; the host validates and forwards. The host stamps `caller` automatically — apps cannot lie about who they are.
3. `payload` is a free-form JSON escape hatch for intent kinds that haven't earned a first-class variant yet. Apps wanting to use it must document their payload shape in their manifest under `[app.open_intent]`.
4. `run_id` lets a notification or spawn resume a Run. When present, the app is expected to emit a `RunUpdate` after initializing.
5. `protocol_version` lets the host and app negotiate. v1 apps receiving an Init with `protocol_version = 2` and new fields will ignore unknown fields (JSON forward-compat). v2 apps receiving `protocol_version = 1` must fall back gracefully or refuse to start with a clear error.

### SDK changes

- Python: `init.open_intent` is accessible on the `@app.on_init` handler. No new methods required — it's a field read.
- Python: `emit.spawn_app(..., open_intent=OpenIntent.file("foo.md"))` — convenience constructors.
- Rust SDK: same shape.

### Resolving the palette/CLI/agent mode paths

- **Palette** (`src/command_palette.rs`) — when a user selects an app, construct `OpenIntent::Bare` unless the palette was entered with a query that resolves to a file, in which case `OpenIntent::File`.
- **CLI** (`src/cli.rs`) — `plexi launch text-editor foo.md` becomes `OpenKind::File`. `plexi launch agent "write a script"` becomes `OpenKind::Prompt`.
- **Agent mode** (`src/agent_mode.rs`) — when the agent decides to spawn an app, it emits a `SpawnApp` through Plexi IQ with `OpenKind::Prompt` or `OpenKind::File` as appropriate. The caller is stamped `AgentMode`.
- **Notification resume** — see §5.

---

## 4. New Primitive: Host Event Bus

### Problem

Nothing can observe what other apps are doing. `PlexiApp.spawn_relationships` lives in memory (`src/app.rs:49`). Notifications are logged to JSONL but nothing else is. This blocks the agent-flow visualizer, Plexi IQ's workspace awareness, replay testing, attention queue, trust learning.

This is issue #91 (`.plexi scope infrastructure: event log, pane ancestry, unified permissions`). This section formalizes what #91 asks for.

### Design

A single append-only JSONL event log at `~/.plexi-alpha/events.jsonl` (and per-directory at `.plexi/events.jsonl` when inside a scoped workspace). Events are small, structured, and never block the main loop. Writes go through a bounded channel to a background writer thread.

```rust
pub struct Event {
    pub id: u64,                  // monotonic per process
    pub ts: i64,                  // unix ms
    pub scope: Scope,             // Global | Workspace(path)
    pub kind: EventKind,
    pub caller: Option<Caller>,
}

pub enum EventKind {
    AppSpawned { parent: Option<PaneId>, child: PaneId, app_id: String, open_intent: Option<OpenIntent> },
    AppClosed { pane: PaneId, app_id: String, reason: CloseReason },
    PipeWrite { from: PaneId, channel: String, bytes: u32 },
    NotificationEmitted { id: String, app_id: String, urgency: Urgency, run_id: Option<String> },
    NotificationActioned { id: String, action: String },
    ApiCall { app_id: String, method: String, ok: bool },
    AgentTurn { agent_id: String, run_id: Option<String>, tokens_in: u32, tokens_out: u32, model: String },
    RunCreated { run_id: String, head_task: String, initiator: Caller },
    RunUpdated { run_id: String, status: RunStatus, head_task: String },
    RunCompleted { run_id: String, outcome: RunOutcome },
    PermissionPrompted { app_id: String, capability: String, decision: Decision },
    CostReport { app_id: String, usd: f64, model: String },
}
```

### Subscription

Apps subscribe via a new draw command and receive events via a new event:

```
DrawCommand::EventSubscribe { kinds: Vec<EventKindTag>, scope: SubscribeScope }
PlexiEvent::EventData { event: Event }
```

`SubscribeScope` is `Workspace` (current directory only), `Pane` (this pane's children), or `Group` (linked pane group). **No `Global` subscription without the `observes` capability** (see §7). This is the gate that prevents every app from seeing every event.

### Rules

1. The event bus is **append-only**. No edits, no deletes. Compaction is a background chore (v2.1).
2. Writes are **non-blocking**. If the channel is full (bounded at 4096), the event is dropped and a counter increments. Dropping an event is preferable to slowing render.
3. Events are **scoped**. A workspace event log in `.plexi/events.jsonl` only contains events from apps running inside that workspace. Apps outside cannot see them. This is how directory scope becomes observable without leaking.
4. Events carry **caller provenance**. The host stamps caller fields — apps cannot forge them.
5. The log is **the source of truth** for anything that wants to reconstruct what happened. Spawn relationships, run state, and notification history can all be rebuilt from events. This means the agent-flow visualizer is a pure consumer: no special APIs, no privileged access, just subscribe to `AppSpawned` events and render the graph.

### What this unlocks (free, once it exists)

- Agent flow visualizer app (subscribe to `AppSpawned` + `AgentTurn` + `PipeWrite`)
- Attention queue (#74) — filter `NotificationEmitted` by unread
- Trust learning (deferred to v2.1 but the data is now logged)
- Replay testing (`agent-replay-testing.md`) — events are the replay log
- Cost dashboard — aggregate `CostReport` and `AgentTurn`
- Plexi IQ workspace awareness — IQ subscribes to everything in its scope

---

## 5. New Primitive: `Run`

### Problem

Agent jobs, video renders, multi-step edits, and "I'll be back in 5 minutes" tasks have nowhere to live. Notifications are fire-and-forget. Pane state is ephemeral. A "job" waiting for the user to manually edit a video in the editor app has no host representation — close the pane, lose the state.

### Design

A `Run` is a minimal host-side object representing a multi-step task. It is intentionally dumb: Plexi IQ and apps own the semantics; Plexi just tracks state and makes it addressable.

```rust
pub struct Run {
    pub id: String,              // ulid
    pub created_at: i64,
    pub updated_at: i64,
    pub status: RunStatus,
    pub head_task: String,       // what is this run *currently* doing, in one line
    pub initiator: Caller,       // who started it
    pub scope: Scope,            // Global | Workspace(path)
    pub notification_id: Option<String>,
    pub parent_run_id: Option<String>, // for sub-runs
    pub payload: serde_json::Value,    // free-form; apps own shape
}

pub enum RunStatus {
    Pending,
    Running,
    BlockedOnUser { prompt: String, resume_intent: OpenIntent },
    BlockedOnChild { child_run_id: String },
    Complete,
    Failed { error: String },
    Cancelled,
}
```

Runs are stored in `~/.plexi-alpha/runs.jsonl` (append-only; current state is reconstructed from the log). This mirrors the event bus design — there's no separate database, and the event log is the single source of truth.

### Draw commands

```
DrawCommand::RunCreate { head_task, payload, parent_run_id?, notification? }
  → host returns run_id via PlexiEvent::RunCreated
DrawCommand::RunUpdate { run_id, status, head_task?, payload? }
DrawCommand::RunComplete { run_id, outcome }
```

### Rules

1. **Runs are not workflows.** Plexi does not execute them, retry them, or route them. Plexi stores them. Plexi IQ and apps own the semantics.
2. **Any app can create a Run.** It's cheap. Overuse is fine — the event bus doesn't care.
3. **`BlockedOnUser` is the critical status.** It means the run is paused waiting for a human action (typically an `OpenIntent` to resume with). The notification palette surfaces these prominently. When a user clicks the resume action, the host spawns the target app with the embedded `OpenIntent`, and the app receives `run_id` in its Init so it can fetch the Run's payload via `ApiRequest::RunGet`.
4. **Runs can nest.** A video production Run spawns a "write script" child Run. The parent is `BlockedOnChild { child_run_id }` until the child completes.
5. **Runs live in the event log.** `RunCreated`, `RunUpdated`, `RunCompleted` are event kinds. The Run itself is a projection of its events — the canonical store is the JSONL log. In v2.0 the host maintains a simple in-memory index for fast lookup.

### The video editor scenario, end-to-end

1. User tells agent mode: "cut the first 3 seconds off the intro of this video."
2. Plexi IQ (#210) decomposes into script → render → user-review. Emits `RunCreate { head_task: "trim intro" }` → `run_id = r_01...`
3. IQ spawns parallax with `OpenIntent::Prompt { text: "trim 3s", ... }, run_id: r_01`. Parallax renders, emits `RunUpdate { status: Running }`, does the work, emits `RunUpdate { status: BlockedOnUser { prompt: "Review cut", resume_intent: OpenIntent::File { path: "out.mp4" } } }`, emits a `Notification` with `run_id: r_01` and action `resume_run`.
4. Notification palette now shows `⏳ Review cut` under the run. Clicking it spawns the video player with `OpenIntent::File { path: "out.mp4" }, run_id: r_01`.
5. User watches, closes player with a keystroke that emits `DrawCommand::RunUpdate { run_id: r_01, status: Complete, payload: { approved: true } }`.
6. Parallax, which has been subscribed to `RunUpdated` events for `r_01`, sees the completion and emits its own `RunComplete`.
7. IQ, subscribed to the parent run, emits the final notification to the user: "Done."

None of this requires a workflow engine. It's three primitives composing.

---

## 6. Rich Notifications

This section tightens the action payload shape defined in `src/app_protocol.rs:281` and closes the spec gap that #218/#219/#221 all point at.

### Current state

`Notification` exists with `action_type: Option<String>` and `action_payload: Option<Value>`. Only `focus` has a defined payload. `confirm`, `text_input`, `dismiss` are named but unshaped. External ingestion works via `notify_socket.rs`.

### v2 additions

A closed set of action types, each with a typed payload:

```rust
pub enum NotificationAction {
    Focus { pane_id: String, fullscreen: bool },
    Confirm { confirm_text: String, cancel_text: String, on_confirm: Box<NotificationAction> },
    TextInput { prompt: String, placeholder: Option<String>, on_submit: Box<NotificationAction> },
    Dismiss,
    ResumeRun { run_id: String },
    OpenIntent { open_intent: OpenIntent }, // opens an app with this intent
    RunCommand { app_id: String, command: String, args: Vec<String> },
    ExternalUrl { url: String },
}
```

`run_id: Option<String>` is added to `Notification` itself so notifications can be grouped, filtered, and surfaced as run status.

### Rules

1. Notifications with a `run_id` render as a run card in the palette: head_task, elapsed time, status pill, action button.
2. `ResumeRun` and `OpenIntent` actions are the canonical way to wake up a blocked Run from the user's side.
3. External notifications via `notify_socket.rs` get the same action surface. This means a CLI daemon can push a "build done" notification that clicks into an app.
4. Notifications without a `run_id` stay as v1 fire-and-forget events. This is the common case and stays cheap.

### No new SDK surface required beyond action constructors. All of this is host-side.

---

## 7. Capability Enforcement Pass

`src/app_permissions.rs` has the types (`AppPermissions`, `FsPermission`, trust levels). `src/app_api.rs` has per-request checks for structured API calls. What's missing is:

1. **Runtime prompts.** When an app makes a capability call it didn't declare in its manifest, or tries to escalate, Plexi prompts the user: "Yes once / Yes always / No". Persisted to `~/.plexi-alpha/permissions.json` per (app_id, capability, scope).
2. **New capabilities for v2:**
   - `observes = ["spawn", "notifications", "runs", "agent_turns"]` — gates event bus subscriptions by kind. Default: none.
   - `create_runs: bool` — gates `RunCreate`. Default: true (Runs are cheap and non-destructive).
   - `open_intent_kinds = ["file", "prompt", ...]` — gates which `OpenKind` variants an app can emit on spawn. Default: file, url.
3. **Directory scope is structural, not declarative.** An app running inside a workspace cannot construct an `OpenIntent::File` with a path outside that workspace. The host validates paths against scope at the ApiRequest layer and at SpawnApp time. This is how `VISION.md:44` becomes "by construction, not by convention" — the host refuses the call, the app never gets to try.

Trust scores and the float-based trust system (`agent-orchestration.md` §4) stay deferred. v2 uses binary prompts. The data (`PermissionPrompted` events) is logged, so v2.1 can train trust scores from it without a migration.

---

## 7.5. Input Layering Contract

### Problem

v1 has no centralized keyboard routing. Overlays, panes, modals, and apps all call `ui.input_mut(|i| i.consume_key(...))` independently, and the outcome of any keypress is determined by widget rendering order plus ad-hoc guards. This is how the command palette can be "open" but still leak Enter/arrows to the underlying app (alpha-bug #240), how quick-note cursor activation on pane navigation breaks (alpha-bug #236), and how egui TextEdit can eat app-level shortcuts like Cmd+S before the intended handler runs (DEV_LOG lesson, recorded 2026-03). The pattern is: every new overlay added to Plexi re-discovers the same bug class and invents its own workaround.

v2 makes the routing explicit and centralized.

### Design

A host-owned **input layer stack** with named, priority-ordered layers. Each layer implements a small trait and registers with the stack on activation.

```rust
pub trait InputLayer {
    /// Human-readable identifier (for event bus logging and debugging).
    fn name(&self) -> &'static str;

    /// Called once per frame, before any egui widget renders. Returns
    /// `Consumed` to claim the event (downstream layers and widgets will
    /// never see it), or `Passthrough` to defer.
    fn handle(&mut self, ev: &InputEvent, ctx: &mut InputContext) -> InputDecision;
}

pub enum InputDecision { Consumed, Passthrough }

pub struct InputLayerStack {
    layers: Vec<Box<dyn InputLayer>>, // top of stack = highest priority
}
```

The default layer priority (top to bottom):

1. `Overlay::CommandPalette` — when active
2. `Overlay::QuickNote` — when active
3. `Overlay::NotificationPalette` — when active
4. `Overlay::AgentMode` — per-pane, when active
5. `Pane::Focused` — the currently focused pane's input owner (external app, terminal, built-in)
6. `Pane::Unfocused` — background panes (input is mostly dropped here; sole exception is a narrow allowlist for hot-path shortcuts)

Each frame:
1. The host drains egui's input event queue exactly once.
2. For every event, the stack is walked top-down.
3. The first layer returning `Consumed` wins; the event is removed from the stream.
4. If every layer passes, the event falls through to the lowest layer (effectively discarded at the host boundary).

No egui widget rendering occurs until after the stack has been walked. This means `TextEdit` widgets inside overlay/pane components never see events that a higher-priority layer claimed — the "TextEdit eats Cmd+S before my handler fires" class of bug becomes impossible by construction.

### Rules

1. **All input routing goes through the stack.** No widget-level `ui.input_mut(|i| i.consume_key(...))` calls in overlay code. The few that remain in v1 code paths (`command_palette.rs`, `quick_note_app.rs`, `notification_palette.rs`, `agent_mode.rs`, `keys.rs`) migrate to `InputLayer` implementations as part of v2 Month 2.
2. **Layers are pushed on activation, popped on dismissal.** The stack owns layer lifetime; activation is a push, dismissal is a pop. Layers never self-remove mid-frame.
3. **Overlays at the same priority level are mutually exclusive.** Only one top-level overlay can be active. Opening a second one dismisses the first. This keeps the priority order unambiguous.
4. **`Pane::Focused` is the default layer for external apps.** An app's subprocess receives a `Key` event on stdin only if every higher layer on the stack declined it. Apps cannot observe events the host consumed; the wire protocol `Key` event is the surface, the layer stack is the gate.
5. **The stack is observable.** `EventKind::InputLayerChanged { layer: String, active: bool }` fires on the event bus (§4) whenever a layer pushes or pops. This lets the agent-flow visualizer, replay testing, and debugging tools see what was in focus when. It also means `alpha-bug #240` becomes testable: "open the palette, send a keystroke, assert the top layer is `Overlay::CommandPalette` and the pane's app never received the event."
6. **Capability enforcement composes with layering.** A pane's `observes` capability (§7) still gates event bus subscriptions. Input layering is distinct — it governs which layer gets to handle a raw egui input event, not which pane gets to observe structured host events.

### What this fixes

| Bug / issue | Today | Under the layer stack |
|---|---|---|
| **#240** Command palette leaks input | TextEdit widget or lower panes consume before palette can guard | `Overlay::CommandPalette` is top of stack when active; pane layers never see the event |
| **#236** Quick note cursor doesn't activate on Cmd+H/J/K/L pane nav | Pane focus moves but inner TextEdit doesn't re-claim egui focus | Navigation operation rebuilds `Pane::Focused` layer with the destination's input owner; next frame's events route to the new pane |
| **`consume_key` modifier exactness** (CLAUDE.md lesson) | `consume_key(NONE, Enter)` matches Enter with Shift held — app-level checks get confused | Layers receive raw events and decide for themselves; no more "does consume_key consider modifiers" surprises |
| **TextEdit eats Cmd+S** (CLAUDE.md lesson) | Widget render consumes before app handler | App-level shortcuts live in a pane-level `InputLayer` that runs before widgets render |

### SDK surface

**None.** This is host-internal. External apps see the same `Key` events on their stdin they always have — they just get fewer spurious ones, and the ones they do get are guaranteed to be events no higher-priority layer wanted.

The only change visible to app authors is in documentation: the SDK conventions doc (see issue #241 / `sdk/python/README.md`) will note that apps can assume overlays have first dibs on input, so apps should not try to "guard" against overlay-active states in their own key handlers. That code smell becomes unnecessary.

### Implementation location

New file: `src/input_layer.rs` — the stack, the trait, the default priority order, the bus event emission.

Refactored consumers:
- `src/keys.rs` — becomes a thin draining function that walks the stack per frame. No direct `consume_key` calls.
- `src/command_palette.rs` — implements `InputLayer`. Arrow/Enter/Escape/Tab/search-text all flow through its `handle()`.
- `src/quick_note_app.rs` — implements `InputLayer` for the quick-note overlay path. Focus activation on pane nav is fixed by the Pane::Focused layer re-pushing its owner on navigate.
- `src/notification_palette.rs` — implements `InputLayer`.
- `src/agent_mode.rs` — implements `InputLayer` per-pane. Ctrl+/ activation pushes; deactivation pops.
- `src/pane_ops.rs` — pane focus changes drive Pane::Focused layer rebuilds.

### Testing story

The event bus makes this testable without UI automation. A host integration test can:
1. Push an `Overlay::CommandPalette` layer.
2. Inject a synthetic `Key { key: "Enter", .. }` into the input queue.
3. Drain the stack.
4. Assert the CommandPalette layer's handler consumed the event.
5. Assert the focused pane's external-app subprocess received zero `Key` events on its stdin.
6. Assert exactly one `InputLayerChanged` event was emitted on the bus.

This is the first time input routing becomes deterministically testable in Plexi. v1's testing story for input was "start the app and mash keys" — v2 retires that.

### Relationship to the rest of v2

- **§3 `OpenIntent`** — unchanged. OpenIntent is launch-time; layer stack is per-frame input.
- **§4 event bus** — layer changes emit events on the bus.
- **§5 `Run`** — unchanged.
- **§6 rich notifications** — palette input handling goes through `Overlay::NotificationPalette` instead of ad-hoc key consumers.
- **§7 capability enforcement** — complementary. Capabilities gate what an app can observe/call; layering gates what a layer can handle.
- **§8 typed pipes** — unchanged.
- **§9 Plexi IQ** — IQ's agent-mode activation pushes `Overlay::AgentMode` like any other overlay.

### Why this is in v2 and not v2.1

The user flagged this explicitly when reviewing alpha-bug #240: "we need to fix how keyboard focus and keymap priority works at a systemic level because we keep running into this issue. So it's not an elegant enough solution." Every alpha-bug about input (#236, #240, and likely future entries in the same class) is a symptom of the missing layer. Shipping v2 without this is shipping a v2 that will immediately accumulate new palette-focus bugs as overlays are added. The right structural fix is load-bearing for the v2 release narrative, not polish.

**Trade-off acknowledged:** this adds ~1 week to Month 2. The alternative — a narrow palette-only `consume_key` patch as the #240 fix — was considered and rejected because it would need to be re-solved for every new overlay (notification palette in §6, run palette cards, agent mode UI) that v2 adds.

---

## 8. Typed Pipes Phase 1

Everything in `docs/specs/typed-pipes.md` §2.3 and onward. This section confirms Phase 1 ships with v2 and clarifies two interactions.

### Relationship to `OpenIntent`

Typed pipes carry **data flows** (this file changed, this text got selected). `OpenIntent` carries **launch context** (open this file now). They are not substitutes. An app launching another app passes `OpenIntent` at spawn time; a long-lived pair of apps passes continuous updates over typed pipes.

### Relationship to the event bus

Pipe writes emit `PipeWrite` events on the event bus with size and channel name but **not payload**. This is important: pipes can carry high-frequency data (`core.metric`, `core.event` streams) and logging every payload would overwhelm the log. The bus gets a summary; the pipe consumer gets the data.

### "Linked pane group" definition (resolving contradiction #4 from the earlier review)

A linked pane group is defined as **all panes sharing a common parent that has `[app.links_children] = true` in its manifest, or all panes spawned with `SpawnLayout::Cols` or `SpawnLayout::Rows` from the same parent.** The spatial canvas multi-screen grouping (`spatial-canvas.md` Option B/C) is deferred; v2 uses the simple parent-based rule.

---

## 9. Plexi IQ Stage 1

Implementation is tracked in issues #210, #212. This doc constrains the protocol surface IQ needs.

### What IQ is in v2

- An in-host orchestrator spawned as a child process, speaking the app protocol, with the new `observes = ["*"]` capability granted by default.
- Backend: `claude -p --resume` subprocess (cheaper via prompt caching; #125).
- Responsibilities: agent mode turn handling, delegation to installed agents via `SpawnApp` + `OpenIntent::Prompt`, Run creation and lifecycle, notification dispatch on completion.

### What IQ is **not** in v2

- Not the intelligence gateway (`intelligence-protocol.md` / #213 — deferred). Individual apps still make their own LLM calls when needed, using their own API keys. PGAP is v2.1+.
- Not the trust float system. IQ uses binary capability prompts.
- Not a separate from agent mode. Agent mode is IQ's UI; IQ is agent mode's backend. They are the same subsystem viewed from different angles.

### Resolving the `/approve` ownership contradiction (contradiction #1 from the earlier review)

**IQ owns the approval workflow. Agent mode renders it.** `/approve`, `/deny`, `/status`, `/jobs` are slash commands in agent mode (`agent-mode.md:77`) that translate to IQ draw commands. The data lives in Runs. Agent mode is the UI.

### Resolving the `.plexi/agents/` namespace collision (contradiction #4)

Two separate subdirectories:
- `.plexi/agents/` — orchestrator configurations only (system prompts, memory, versions). Managed by IQ.
- `.plexi/apps/` — installed apps, which may declare `[app.agent]` in their manifest. Managed by the app registry.

Installed agent apps do not live under `.plexi/agents/`. IQ discovers them via the registry by filtering manifests for `[app.agent]` presence. An installed agent app whose id is `parallax` does not collide with an orchestrator configuration for `parallax` — they're in different directories.

---

## 10. Protocol Version Negotiation

### Design

`Init` now carries `protocol_version: u32`. Apps declare support in their manifest:

```toml
[app]
id = "text-editor"
protocol_version = 2  # required; explicit over default
```

### Rules

1. The host reads `protocol_version` from the manifest at load time. Apps missing the field are assumed `protocol_version = 1` and a deprecation warning is logged.
2. On `Init`, the host sends the negotiated version (lower of host and app). v2 hosts running v1 apps send v1 Inits (no `open_intent`, no `run_id`). v1 hosts running v2 apps send v1 Inits and the app falls back.
3. Apps must refuse to start if `protocol_version` is lower than their minimum supported version, with a clear stderr message. No silent degradation of features that require v2 primitives.
4. v3 will add new kinds (likely PGAP routing). v2 apps will keep working. The `OpenIntent` and event bus schemas are designed for forward-compat via JSON.

---

## 11. Resolving Contradictions Between Existing Specs

Summary of the four contradictions the earlier review found, with resolutions as they appear in v2:

| # | Contradiction | Resolution |
|---|---|---|
| 1 | Agent mode `/approve` vs. orchestrator approval workflow | IQ owns workflow; agent mode renders UI. §9. |
| 2 | Trust/risk floats vs. binary permission prompts | v2 ships binary prompts. Floats deferred. `PermissionPrompted` events are logged for v2.1 training. §7. |
| 3 | Directory scope enforcement mechanism | Host-enforced at ApiRequest and SpawnApp layer via path validation. OpenIntent paths are checked. §7. |
| 4 | `.plexi/agents/` vs. installed agent apps namespace | Two dirs: `.plexi/agents/` for orchestrator configs, `.plexi/apps/` for installed apps with `[app.agent]`. §9. |

Spatial canvas "linked pane group" definition also resolved in §8 (simple parent-based rule for v2).

---

## 12. Ship Order (3 Months)

The order is derived from dependencies, not ambition. Each item unblocks the next.

### Month 1 — Plumbing

1. **Protocol version negotiation** (§10) — trivial. 1 day. Lands first so every subsequent change is additive behind a version bump.
2. **Event bus** (§4) — background writer, JSONL format, `EventSubscribe`/`EventData` plumbing, scoping. ~1 week. Everything downstream consumes this.
3. **`OpenIntent` payload** (§3) — add fields, thread through palette/CLI/SpawnApp. Backfill existing spawn paths. ~3-4 days.
4. **Run primitive** (§5) — dumb store, JSONL log, draw commands, run palette card rendering. ~1 week.

### Month 2 — Surface

5. **Rich notifications** (§6) — action enum, run_id binding, palette integration. ~4 days.
6. **Capability enforcement pass** (§7) — runtime prompt flow, permissions.json, `observes` capability, OpenIntent path validation. ~1 week.
7. **Input layering contract** (§7.5) — `src/input_layer.rs` stack, migrate `command_palette.rs`/`quick_note_app.rs`/`notification_palette.rs`/`agent_mode.rs`/`keys.rs` onto the layer API, emit `InputLayerChanged` events. ~1 week. Closes #240 and #236 and prevents every future overlay from re-discovering the same bug class.
8. **Typed pipes Phase 1** (`typed-pipes.md`) — manifest parsing, auto-wiring, linking matrix UI. ~2 weeks. (Largest item; parallelizable with #5-7.)

### Month 3 — Intelligence

9. **Plexi IQ Stage 1** (§9, #210/#212) — in-host orchestrator, claude -p backend, agent mode integration, Run lifecycle, `/approve` workflow. ~2-3 weeks.
10. **Agent flow visualizer app** (validation) — first external consumer of the event bus. Proves the bus is sufficient. Not a must-ship; a must-exist-during-testing.
11. **Migration pass** — all bundled example apps bumped to `protocol_version = 2`. SDK 0.4.0 released with OpenIntent + Run convenience methods. DEV_LOG entry. CHANGELOG.

### What validates each item

- Event bus: `tail -f ~/.plexi-alpha/events.jsonl` during any session shows a coherent stream.
- OpenIntent: `plexi launch text-editor foo.md` opens foo.md without text-editor reading argv.
- Run: the video editor scenario in §5 runs end-to-end.
- Rich notifications: a notification with `ResumeRun` action resumes a blocked run in one click.
- Capability: trying to read a file outside scope from an app returns a permission error, prompts the user, and the decision persists.
- Input layering: open the command palette, press Enter — the palette consumes it, the underlying pane's subprocess receives zero `Key` events (verified via stdin trace), and the event bus records exactly one `InputLayerChanged { layer: "Overlay::CommandPalette", active: true }` then `active: false` on dismissal.
- Typed pipes: two unrelated example apps compose via matching kind+name with no code changes.
- Plexi IQ: agent mode can delegate a task to parallax, track it as a Run, and surface completion.

---

## 13. Explicit Philosophy Alignment

Each non-negotiable from `VISION.md` maps to a v2 decision:

| Non-negotiable | How v2 preserves it |
|---|---|
| Agent-native first, human-friendly always | Every v2 primitive is invokable by agents: OpenIntent via SpawnApp, Runs via IQ, event bus via subscribe, notifications via notify_socket. |
| One install, three interfaces | Manifest adds `protocol_version`, `observes`, `open_intent_kinds`, `create_runs` — no new install surface. |
| The permission model is the product | §7 wires enforcement end-to-end. `PermissionPrompted` events are logged. |
| PGAP is the only path to intelligence | Deferred to v2.1. v2 uses `claude -p` for IQ; individual apps keep their existing patterns. Clear path to PGAP because all LLM calls will be observable via `AgentTurn` events in the bus. |
| Beautiful is not cosmetic | No new draw primitives; notification palette and linking matrix get design pass. |
| Directory is the permission boundary | §7 rule: OpenIntent paths validated at host boundary. |

And against the operational rules from `CLAUDE.md` and DEV_LOG:

- **Keep SDK simple.** v2 adds ~4 optional methods, all stdlib JSON. Python SDK stays zero-dependency. Rust SDK gets a parity pass as part of Month 3.
- **Explicit over defaults.** `protocol_version` is required in manifest. No silent v1 fallback for new apps.
- **Configuration philosophy.** Runs, OpenIntent, notification actions all use discriminated unions — no magic string matching.

---

## 14. What Could Kill This

Risks ordered by likelihood:

1. **Plexi IQ Stage 1 is bigger than Month 3.** The existing issue #210/#212 tracks it but the scope is fuzzy. Mitigation: define IQ's v2 scope as "agent mode delegation + Run lifecycle + notification dispatch" and defer everything else (task decomposition, multi-agent workflows, improvement officer) to v2.1. Ship a dumb IQ.
2. **Event bus hot-loop cost.** If the bus is called on every pipe write it could slow renders. Mitigation: bounded channel, drop-on-full, background writer, periodic flush.
3. **Typed pipes Phase 1 manifest wiring complexity.** The linking matrix UI has been spec'd in 824 lines for a reason. Mitigation: ship auto-wiring first, ship the matrix UI second, accept that if the UI slips to v2.1 the auto-wiring alone is still a huge improvement.
4. **OpenIntent payload escape hatch abuse.** Apps stuff domain-specific schemas into `payload` and the "advisory" contract erodes. Mitigation: require apps using `payload` to declare its schema in `[app.open_intent]` and emit a deprecation warning when undeclared payloads are used.
5. **v1 apps break when the host sends v2 Inits.** Mitigation: JSON forward-compat — unknown fields must be ignored by both SDKs. Test with the full bundled example suite during Month 1.
6. **Capability prompts are annoying.** If every agent turn prompts the user, the agent is unusable. Mitigation: `observes` and `create_runs` are granted to IQ at install time with an explicit "this is the orchestrator" manifest flag. User sees the prompt once for IQ, never again.

---

## 15. What This Doc Replaces, Supersedes, and References

**Supersedes:** nothing. All existing specs remain valid; v2 is additive.

**Resolves:** contradictions in `agent-mode.md`, `agent-orchestration.md`, `typed-pipes.md`, `VISION.md` (see §11).

**Defers:** `intelligence-protocol.md`, `agent-replay-testing.md`, `wasm-pwa-deployment.md`, `sync-architecture.md`, `chat-primitive.md`, `core-text-editor-primitive.md`, `core-advanced-ui-sdk.md`, `core-layout-presets.md`, `app-focus-manager.md`, `app-shell-config.md`. None of these are deleted; none are required for v2.0.

**Depends on:** `app-infrastructure.md` (v1 contract), `typed-pipes.md` (Phase 1), `agent-orchestration.md` (IQ design), `agent-mode.md` (UI surface), manifest schema at `schemas/plexi-manifest-schema.json`.

**Implemented across:** `src/app_protocol.rs` (types), `src/process_app.rs` (subprocess handling), `src/pane_ops.rs` (dispatch), `src/app_registry.rs` (manifest), `src/app_permissions.rs` (enforcement), `src/notification_log.rs` (notifications), `src/notify_socket.rs` (external ingestion), new `src/event_log.rs` (bus), new `src/run_store.rs` (runs), new `src/plexi_iq/` (orchestrator).

---

## 16. Open Questions — Decide Before Month 1

1. **Should Runs have a TTL?** A run in `BlockedOnUser` for 6 months is clutter. Proposal: 30-day default, `expires_at` field in Run, expired runs moved to a separate log. Decide before shipping §5.
2. **Should the event bus be per-workspace or global by default?** Global is easier; per-workspace is the directory-scope story. Proposal: per-workspace when inside `.plexi/`, global otherwise. Decide before shipping §4.
3. **Should `OpenIntent::Resume` be its own kind or folded into `run_id`?** Proposal: just `run_id` on the Init + whatever `OpenKind` the caller specified. `Resume` is a UX concept, not a protocol one.
4. **Protocol version: u32 vs. semver?** Proposal: u32 for simplicity. v2 → 2, v3 → 3. Breaking changes increment; non-breaking changes don't need negotiation.
5. **Rust SDK parity — block v2 on it or defer?** Proposal: defer. Rust SDK gets the event bus, OpenIntent, and Run primitives in Month 3 but lags in polish. No example apps are blocked.

---

**End of spec.** This doc is the contract for Plexi 2.0. Changes to it require a bump to `Last updated` and a DEV_LOG entry.
