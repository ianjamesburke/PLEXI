# Plexi Protocol v2 — Recursive Agent-Native Foundation

**Status:** Draft
**Last updated:** 2026-04-16
**Owner:** plexi-core
**Target:** Plexi 2.0 — recursive `.plexi` instance foundation

---

## TL;DR

Plexi v1's protocol is solid for single-app rendering and parent↔child composition (see `subsystems/app-infrastructure.md`, `subsystems/typed-pipes.md` Phase 0). It is not sufficient for the agent-native vision in `VISION.md` because it lacks the recursive substrate: `.plexi` directories as sealed instance boundaries, nested Plexi processes as PGAP-speaking children, and root-visible depth state.

Plexi 2.0 makes recursion foundational:

1. **Depth is structural** — a `.plexi` directory is an instance boundary and a node in the depth tree.
2. **PGAP is the recursive boundary** — nested instances are subprocesses; stdin/stdout JSON is the only shared interface.
3. **Capabilities attenuate downward** — child instances can receive fewer permissions than parents, never more.
4. **Root keeps the global view** — event bus, tree status, Runs, notifications, and IQ make depth visible and controllable.

The earlier v2 primitives remain, but they now serve the recursive model. `OpenIntent`, the event bus, Runs, rich notifications, typed pipes, capability enforcement, protocol version negotiation, and Plexi IQ are not parallel pillars; they are the machinery that makes recursive `.plexi` instances usable.

The explicit design constraint: **recursion must be visible before it is complete.** v2.0 first proves `.plexi` directory discovery and navigation, then layers embedded rendering, capability manifests, depth notifications, and portals on top.

---

## 1. Scope and Non-Goals

### In scope for Plexi 2.0

- Fractal PGAP foundation (`subsystems/fractal-pgap.md`, `roadmaps/fractal-pgap/`)
- `.plexi` boundary discovery and depth-tree proof of concept
- Process lifecycle foundation: process groups, shutdown, `Suspend`, `Resume`
- Render summary protocol: `RenderMode`, `StatusSummary`, `PaneSummary`, `Health`
- Embedded Plexi spike: `plexi --embedded`
- TreeStatus rollup and depth-addressed notifications
- Capability manifest and root-mediated secret broker MVP
- Init `OpenIntent` payload (new, this doc)
- Host event bus / `.plexi` event log (#91, this doc formalizes)
- `Run` primitive (new, this doc)
- Rich notification action payloads (#218/#219/#221, this doc closes spec gap)
- Plexi IQ Stage 1 in-host orchestrator (#210/#212)
- Typed pipes Phase 1 (`subsystems/typed-pipes.md`)
- Capability enforcement pass (`app_permissions.rs` → runtime prompt)
- Protocol version negotiation (new, trivial)
- Portals and direct pipe promotion proof, at POC quality

### Explicitly deferred to v2.1+

- PGAP intelligence gateway (#213, `subsystems/intelligence-protocol.md`) — v2 keeps `claude -p --resume` subprocess
- Trust/risk float learning (`subsystems/agent-orchestration.md` §4) — v2 uses binary Yes once/Yes always/No prompts
- Agent replay testing (`proposals/agent-replay-testing.md`)
- WASM/PWA deployment (`proposals/wasm-pwa-deployment.md`)
- SpacetimeDB sync (`proposals/sync-architecture.md`)
- Chat primitive (`proposals/chat-primitive.md`)
- Core text editor primitive (`proposals/core-text-editor-primitive.md`)
- Advanced UI SDK egui widgets (`proposals/core-advanced-ui-sdk.md`, #132)
- Spatial canvas Option B/C beyond the `.plexi` depth tree proof
- Spawn `SpawnLifecycle::Prompt` — stays stubbed as `Orphan`
- Production-grade hibernation for deep inactive instances
- Full 3D depth visualization

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

v2 adds: `protocol_version`, `OpenIntent`, optional capability manifest on `Init`, `RenderMode`, `Suspend`/`Resume`, `StatusSummary`, `TreeStatus`, `RunCreate`/`RunUpdate`/`RunComplete`, `RunEvent`, `EventSubscribe`, and `EventData`. Existing apps remain valid because new fields are optional and new events/commands are additive.

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
- Replay testing (`proposals/agent-replay-testing.md`) — events are the replay log
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

Trust scores and the float-based trust system (`subsystems/agent-orchestration.md` §4) stay deferred. v2 uses binary prompts. The data (`PermissionPrompted` events) is logged, so v2.1 can train trust scores from it without a migration.

---

## 8. Typed Pipes Phase 1

Everything in `docs/specs/subsystems/typed-pipes.md` §2.3 and onward. This section confirms Phase 1 ships with v2 and clarifies two interactions.

### Relationship to `OpenIntent`

Typed pipes carry **data flows** (this file changed, this text got selected). `OpenIntent` carries **launch context** (open this file now). They are not substitutes. An app launching another app passes `OpenIntent` at spawn time; a long-lived pair of apps passes continuous updates over typed pipes.

### Relationship to the event bus

Pipe writes emit `PipeWrite` events on the event bus with size and channel name but **not payload**. This is important: pipes can carry high-frequency data (`core.metric`, `core.event` streams) and logging every payload would overwhelm the log. The bus gets a summary; the pipe consumer gets the data.

### "Linked pane group" definition (resolving contradiction #4 from the earlier review)

A linked pane group is defined as **all panes sharing a common parent that has `[app.links_children] = true` in its manifest, or all panes spawned with `SpawnLayout::Cols` or `SpawnLayout::Rows` from the same parent.** The spatial canvas multi-screen grouping (`proposals/spatial-canvas.md` Option B/C) is deferred; v2 uses the simple parent-based rule.

---

## 9. Plexi IQ Stage 1

Implementation is tracked in issues #210, #212. This doc constrains the protocol surface IQ needs.

### What IQ is in v2

- An in-host orchestrator spawned as a child process, speaking the app protocol, with the new `observes = ["*"]` capability granted by default.
- Backend: `claude -p --resume` subprocess (cheaper via prompt caching; #125).
- Responsibilities: agent mode turn handling, delegation to installed agents via `SpawnApp` + `OpenIntent::Prompt`, Run creation and lifecycle, notification dispatch on completion.

### What IQ is **not** in v2

- Not the intelligence gateway (`subsystems/intelligence-protocol.md` / #213 — deferred). Individual apps still make their own LLM calls when needed, using their own API keys. PGAP is v2.1+.
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

## 12. Ship Order

The order is derived from the recursive model. Each slice must be end-to-end testable and should be safe to hand to a Codex agent in series.

### Phase 1 — See The Depth Tree

1. **Protocol version negotiation** (§10) — keep every later change additive and explicit.
2. **Process lifecycle foundation** (`roadmaps/fractal-pgap/01-process-lifecycle.md`) — process groups, shutdown, `Suspend`, `Resume`.
3. **Depth tree proof of concept** (`roadmaps/fractal-pgap/02-depth-tree-poc.md`) — discover `.plexi` directories and render them as depth nodes.
4. **Event bus** (§4) — log app lifecycle, depth transitions, and pipe summaries.

### Phase 2 — Make Depth Protocol-Native

5. **OpenIntent with depth context** (§3) — launches carry file/prompt/caller/run/depth context.
6. **Render summary protocol** (`roadmaps/fractal-pgap/03-render-summary-protocol.md`) — `RenderMode`, `StatusSummary`, `PaneSummary`, `Health`.
7. **TreeStatus + depth notifications** (`subsystems/fractal-pgap.md`) — root can see active depths and jump to notification sources.
8. **Run primitive** (§5) — multi-step work is scoped to a depth and stored as events.

### Phase 3 — Make Depth Safe And Recursive

9. **Embedded Plexi spike** (`roadmaps/fractal-pgap/04-embedded-instance-spike.md`) — `plexi --embedded` proves PGAP input/output or documents the blocker.
10. **Capability containers** (`roadmaps/fractal-pgap/05-capability-containers.md`) — capability manifest, attenuation, TTL, secret broker MVP.
11. **Typed pipes Phase 1** (`subsystems/typed-pipes.md`) — manifest wiring and auto-wire for app composition.
12. **Plexi IQ Stage 1, depth-aware** (§9) — root/depth-aware delegation using Runs, event bus, OpenIntent, and capabilities.

### Phase 4 — Prove The Fractal UX

13. **Portals and direct pipe promotion proof** (`roadmaps/fractal-pgap/06-portals-and-direct-pipes.md`) — root can view a child depth and focused-depth I/O avoids unnecessary render loops.
14. **SDK 0.4.0 rewrite** — Python and Rust SDKs expose v2 recursive protocol fields; vendored/example SDK copies are regenerated.
15. **App reduction + migration pass** — keep only apps that validate the new protocol. Existing proof-of-concept apps may be deleted or rewritten instead of preserved.
16. **Installable fractal worktree** — `fractal` worktree installs cleanly and demonstrates recursive `.plexi` navigation.

### What validates each item

- Depth tree: fixture `.plexi` directories render as navigable nodes.
- Lifecycle: pane close/crash reaps child process trees.
- Event bus: `tail -f ~/.plexi-alpha/events.jsonl` shows app/depth events.
- OpenIntent: depth/app launch carries context without argv conventions.
- Render summary: parent can request cheap child status.
- Notification: child depth notification can jump back to source.
- Embedded: `plexi --embedded` exchanges valid PGAP JSON.
- Capability: child reads allowed scope and is denied outside it.
- Typed pipes: unrelated apps compose via matching kind/name.
- IQ: agent mode delegates to an installed agent app and tracks a depth-scoped Run.

---

## 13. Explicit Philosophy Alignment

Each non-negotiable from `VISION.md` maps to a v2 decision:

| Non-negotiable | How v2 preserves it |
|---|---|
| Agent-native first, human-friendly always | Every depth/app capability can be invoked through PGAP; UI is one surface over the same capability. |
| One install, three interfaces | A `.plexi` directory owns apps, skills, agents, permissions, events, and state. |
| The permission model is the product | Capability manifests and root-mediated secret access make nested instances sealed boxes. |
| PGAP is the only path to intelligence | Nested instances and IQ communicate through PGAP; direct model calls are phased out of protocol-conforming apps. |
| Beautiful is not cosmetic | The depth tree and portal surfaces are product requirements, not debug overlays. |
| Directory is the permission boundary | `.plexi` directories are the structural boundary; filesystem scope is enforced at host/API/spawn boundaries. |

Operational rules:

- **Protocol first, apps second.** Existing example apps are disposable. Keep or rewrite only the apps that prove the v2 protocol.
- **Explicit over defaults.** v2 apps declare protocol version, capabilities, skill/agent surfaces, and pipe contracts.
- **No invisible authority.** A child can only narrow its parent's grants.

---

## 14. What Could Kill This

Risks ordered by likelihood:

1. **Embedded rendering is harder than expected.** Mitigation: depth-tree POC ships first; `--embedded` is a spike with a documented yes/no result before broad renderer refactors.
2. **Scope becomes too wide.** Mitigation: every Fractal roadmap file must produce a testable artifact. Anything not testable moves out.
3. **Old app compatibility drags the architecture backward.** Mitigation: v2 may delete or rewrite proof-of-concept apps. Backward compatibility is best-effort for installed third-party apps, not for repo examples.
4. **Event bus hot-loop cost.** Mitigation: bounded channel, drop-on-full, background writer, summaries for pipe writes.
5. **Capability prompts are noisy.** Mitigation: manifests declare ceilings, runtime prompts persist decisions, IQ gets explicit orchestrator grants.
6. **Depth UX is visually confusing.** Mitigation: ship the depth tree pane before portals/direct pipes, and make the structure inspectable at all times.

---

## 15. What This Doc Replaces, Supersedes, and References

**Supersedes:** the previous v2 framing where OpenIntent/event bus/Runs/notifications were the release's top-level story. They remain in scope, but as machinery for recursive `.plexi` instances.

**Resolves:** contradictions in `subsystems/agent-mode.md`, `subsystems/agent-orchestration.md`, `subsystems/typed-pipes.md`, `VISION.md` (see §11), plus the earlier ambiguity about whether issue #260 belonged inside Plexi 2.0.

**Defers:** `subsystems/intelligence-protocol.md`, `proposals/agent-replay-testing.md`, `proposals/wasm-pwa-deployment.md`, `proposals/sync-architecture.md`, `proposals/chat-primitive.md`, `proposals/core-text-editor-primitive.md`, `proposals/core-advanced-ui-sdk.md`, `proposals/core-layout-presets.md`, `proposals/app-focus-manager.md`, `proposals/app-shell-config.md`. None are deleted; none are required for v2.0.

**Depends on:** `subsystems/app-infrastructure.md`, `subsystems/typed-pipes.md`, `subsystems/agent-orchestration.md`, `subsystems/agent-mode.md`, `subsystems/fractal-pgap.md`, and `roadmaps/fractal-pgap/`.

**Implemented across:** `src/app_protocol.rs`, `src/process_app.rs`, `src/pane_ops.rs`, `src/app_registry.rs`, `src/app_permissions.rs`, `src/app_api.rs`, `src/notification_log.rs`, `src/notify_socket.rs`, `src/context.rs`, `src/plexi_iq/`, and new depth/event/run modules as needed.

---

## 16. Open Questions — Decide Before Implementation

1. **How much embedded rendering must v2.0 ship?** Proposal: require a PGAP JSON proof and one visible nested frame; defer polish.
2. **Should the first depth tree be a built-in Rust pane or external app?** Proposal: built-in until TreeStatus is stable, then expose to apps.
3. **What is the canonical depth address format?** Proposal: stable path-derived IDs plus display labels from `.plexi/bookmarks.toml`.
4. **Should Runs have a TTL?** Proposal: inherit optional TTL from capability manifest; default none for user-visible runs.
5. **Which example apps survive v2?** Proposal: keep a small conformance suite: depth tree, text/file opener, typed-pipe pair, notification sender, IQ agent stub, and one visual app.

---

**End of spec.** This doc is the contract for Plexi 2.0. Changes require a `Last updated` bump and a DEV_LOG entry.
