# Fractal PGAP — Recursive Instance Nesting & Agent Isolation

**Status:** Draft subsystem
**Draft for:** Plexi v2.0
**Depends on:** PGAP v2 (protocol version negotiation, PR #254)

**Execution roadmap:** [`../roadmaps/fractal-pgap/`](../roadmaps/fractal-pgap/)

---

## Overview

Plexi instances can be recursively nested via the PGAP (Plexi Generic App Protocol). Every `.plexi` directory is an instance boundary. The directory tree IS the nesting tree. This enables directory-scoped agent containers, version-pinned embedded instances, and a self-similar fractal infrastructure where anything you can do at one level, you can do at any level.

This subsystem is the organizing principle for Plexi v2.0. The v2 primitives (event bus, `OpenIntent`, Runs, rich notifications, capability enforcement, typed pipes, protocol version negotiation) are not separate product pillars; they are the substrate required for recursive `.plexi` instances, depth-scoped agents, and portable capability containers.

---

## Core Concepts

### Depth as a Z-Axis

Panes split horizontally and vertically (X/Y axes). `.plexi` directories add a Z-axis — depth.

- **Cmd+Enter** on a pane whose cwd contains a `.plexi` subdirectory: descend one depth. Animation zooms in.
- **Cmd+Escape**: ascend one depth. Parent layout restores exactly as it was.
- `cd` within a depth is free — no instance transition. Depth transitions only occur when crossing a `.plexi` boundary.
- Each depth has its own set of named contexts (workspace layouts), persisted in `.plexi/workspaces/`.

### Instance Isolation

Each `.plexi` instance is a separate OS process:

- Spawned with the Plexi binary and an `--embedded` flag.
- Communication with parent is exclusively through PGAP (newline-delimited JSON over stdin/stdout pipes).
- No shared memory, no inherited file descriptors beyond the PGAP pipe.

Instances declare a mode in `.plexi/config.toml`:

- `mode = "root"` — ignores parent, full autonomy. For testing and development.
- `mode = "embedded"` — participates in the depth hierarchy, inherits scoped capabilities.

### Direct Pipe Promotion

When focused on depth N, depths 0 through N-1 become pure pass-through:

- The root establishes a direct stdin/stdout pipe to the focused depth, bypassing all intermediates.
- Intermediate instances receive a `Suspend` event and stop render loops. CPU drops to zero for unfocused levels.
- Memory stays allocated but processing stops. Restores instantly on focus change.
- No inspection, no transformation at intermediate layers while suspended.

### Splitting Rules

Splitting and depth are orthogonal and do not interact except visually:

- **Splitting** is purely spatial (X/Y) — unlimited panes within a single depth.
- **Depth creation** is purely a directory operation (Z) — `plexi init` or a UI affordance creates the `.plexi` directory.
- A split pane can show a preview of a child depth without any structural coupling.

---

## PGAP Extensions

### Render Modes

The `Render` event gains a `mode` field:

- `"full"` — standard interactive render. Full draw command vocabulary.
- `"preview"` — condensed render for above-depth viewing. Child returns a miniaturized summary.

### Preview Levels

Determined by the available screen space allocated by the parent:

| Level | Trigger | Content |
|---|---|---|
| Status-only | 3+ depths above | Icon + one-line status text |
| Single-depth preview | Default for parent tiles | Workspace layout wireframe, per-pane status (process name, last activity, task count) |
| Double-depth preview | Sufficient tile height | Actual pane layout rendered small — like macOS Mission Control. Text too small to read but activity shape is visible |

### StatusSummary Draw Command

A new draw command apps can emit to provide structured metadata to parent instances:

- Uptime
- Process count
- Last activity timestamp
- Current working directories of each pane
- Custom status text string
- Health indicator: `running` | `idle` | `error`

### TreeStatus Rollup Message

Each depth periodically sends a `TreeStatus` message upward containing its own `StatusSummary` plus a rollup of everything beneath it. The root accumulates these into a flat registry:

- Node ID
- Depth level
- Directory path
- Status
- Child count
- Last activity timestamp

This enables a global process tree view — every active node at every depth — surfaceable as a flat list, collapsible tree, sidebar badge, or any plugin-defined visualization. Plugin authors consume `TreeStatus` data directly; the spec does not prescribe a rendering format.

### Notifications with Depth Address

Any node at any depth can emit notifications that bubble up through PGAP:

- Each notification carries its full depth address (e.g., `root/project/agents/scraper`) plus depth level integer.
- Notifications are jumpable — a hotkey or click navigates directly to the originating depth and pane via direct pipe promotion.
- The root maintains a unified notification stream from all active nodes.

### Named Depths / Bookmarks

Persistent labels for locations in the depth tree:

- Examples: `"scraper agent"`, `"staging test"`, `"v2.8 harness"`.
- Stored in root's `.plexi/bookmarks.toml`.
- Surfaced via whatever visualization plugin is active — sidebar, overlay, command palette, etc.
- Notifications attach to bookmark locations naturally, since the depth address IS the identifier.

---

## Capability-Based Agent Containers

### Capability Manifest

Each nested instance is spawned with an explicit capability set, passed via the `Init` event:

```json
{
  "cwd": "/project/agents/scraper",
  "secrets": ["GITHUB_TOKEN"],
  "network": ["api.github.com"],
  "fs_read": ["/project/data"],
  "fs_write": ["/project/output"],
  "ttl_seconds": 3600
}
```

All fields are allowlists. Anything not listed is denied.

### Security Model (WASI/Capsicum-inspired)

- **No ambient authority** — nested instances cannot see the parent's filesystem, secrets, or network beyond what was explicitly granted in the capability manifest.
- **Single credential service at root** — the root holds the secret store. Nested instances request secrets through PGAP via a `RequestSecret` command. Root validates against the capability manifest before responding.
- **Agents never hold raw credentials** — they hold a scoped, time-limited channel to a broker. The credential never crosses the PGAP pipe in plaintext.
- **TTL enforcement** — a watchdog in the parent sends `Shutdown` and kills the process after the declared TTL expires.
- **Capability attenuation only** — children can further restrict capabilities for their own children, never amplify. A child cannot grant a grandchild something the child itself doesn't have.

### Inter-Agent Communication

- Agents at the same depth or across depths communicate through the root's event bus.
- Root validates that both agents have declared intent to communicate before routing any message.
- No direct pipe between agents ever exists — all messages are mediated by root.

### Root Hotline

At any depth, a global hotkey (e.g., Cmd+Shift+Space) opens a command palette that talks directly to the root process — bypasses the depth stack entirely, never touches intermediate instances.

- Implemented as a second always-open PGAP pipe to root, alongside the focused-depth pipe.
- Root receives the message with full depth context: current depth address, focused app, last interaction.
- Enables commands like "turn this into a skill", "share this context with depth 0", "spawn a new agent from this conversation."
- The root hotline is the control plane — always one hotkey away regardless of nesting depth.

---

## Visualization Plugins

`TreeStatus` rollup data is structured JSON available to any app through PGAP. Visualization of the depth tree is not built into the core renderer — it's an app concern. Example plugins:

- **3D depth tree (wgpu)**: a Rust app that renders the depth hierarchy as a navigable 3D structure using wgpu (already Plexi's rendering backend). Renders to a texture composited into an egui pane. Click a node → emit a PGAP navigation command → root handles the depth transition.
- **2D force-directed graph (egui)**: depth nodes as a 2D network diagram using egui's native drawing primitives (lines, bezier curves, circles). Shows inter-agent communication flows. Easiest to build, likely sufficient for most navigation.
- **2D minimap (egui)**: compact sidebar showing the tree as a collapsible outline.

All visualization is Rust-native — no browser, no webview, no JS runtime. Any app that consumes `TreeStatus` can render the tree however it wants. No special privileges required — `TreeStatus` is standard protocol data. Community plugins can build novel visualizations without touching Plexi internals.

---

## Hot Reload

### Python Apps (MVP path)

1. File watcher on the app's directory detects changes.
2. Parent sends `Shutdown`, respawns the Python process, sends `Init`.
3. New process tries to load persisted state from a `.plexi/` state file.
4. If state schema doesn't match: crash with a helpful error, reinitialize fresh. No migration during development.
5. Tracebacks appear in `plexi.log` as app stderr warnings tagged `app::<app_id>`.

### Embedded Plexi Instances

1. Parent sends `Shutdown` on the old pipe.
2. Spawns the (possibly new) binary version, sends `Init` with the same capability manifest.
3. New process picks up persisted workspace state from `.plexi/workspaces/`.
4. If state is incompatible: reinitialize fresh, log a warning. State migration is only supported for stable-to-stable upgrades.

### Version-Pinned Instances

- A `.plexi/config.toml` can pin a specific Plexi binary version.
- Parent spawns that version's binary with `--embedded`.
- Parent is version-agnostic — it only speaks PGAP.
- PGAP version negotiation (PR #254) handles protocol compatibility.
- Unknown draw commands or events degrade gracefully (ignored with a `warn` log).

### Granular State Survival

App state is structured as named fields. On hot reload:

1. Deserialize existing state from disk.
2. Keep all recognized fields.
3. Drop unknown fields (from old code).
4. Initialize new fields with defaults.

This enables game-dev style hot loading: change a behavior parameter without resetting the session state (position, selection, scroll offset, etc.).

---

## Navigation UX

### Depth Transitions

| Action | Condition | Result |
|---|---|---|
| Cmd+Enter | Focused pane, not full-screen | Zoom pane to full screen within current depth |
| Cmd+Enter | Full-screen pane, cwd has `.plexi` subdirectory | Descend one depth. Zoom-in animation. |
| Cmd+Escape | Any depth > 0 | Ascend one depth. Parent layout restores. |

### Per-Depth Contexts

- Each `.plexi` directory owns named contexts (workspace layouts).
- Hotkeys 1–5 (or equivalent) switch between contexts at the current depth.
- Each context independently preserves its pane layout and per-pane app state.
- Context state is persisted to `.plexi/workspaces/<name>.toml`.

---

## Scriptable Test Harness

Every user interaction is a `PlexiEvent` JSON message. Every app response is a `DrawCommand` JSON message. This makes protocol-level testing trivially scriptable:

- A test harness writes `PlexiEvent` JSON to the instance's stdin and reads/asserts `DrawCommand` JSON from stdout.
- No UI automation frameworks, no screen scraping, no timing dependencies.
- Develop an app and its test harness simultaneously.
- CI runs headless — just pipe JSON, assert JSON.
- Works identically for Python apps and embedded Plexi instances because both speak PGAP.

---

## Known Gotchas & Future Work

1. **PGAP version skew** — the draw command vocabulary will grow across releases. Capability advertisement at connection time and graceful degradation for unknown commands are required. Version negotiation (PR #254) is the foundation; capability advertisement is the next step.

2. **Focus/input routing complexity** — keyboard shortcuts bound at multiple depths need clear precedence rules. Direct pipe promotion handles the interactive case (only one depth processes input at a time), but hotkeys that cross depth boundaries (e.g., global notification dismiss) need an explicit priority model.

3. **Resource cleanup on crash** — cascading failures across depths require a process tree reaper at the root, not just single-child cleanup. A crashed intermediate node must not leave grandchildren as orphans.

4. **Rendering latency** — one compositing step per nesting level for preview renders. Imperceptible at 2–3 levels, potentially noticeable at 10+. Direct pipe promotion eliminates this for the actively focused depth.

5. **Memory footprint** — each nested instance is approximately 30–50 MB resident. Practical at 5–10 concurrent instances, needs profiling beyond that. `Suspend` (CPU to zero) is already in the design; a future `Hibernate` (serialize + unload) could reduce memory for deeply nested idle instances.

---

## Implementation Phases

The end-to-end implementation plan lives in [`../roadmaps/fractal-pgap/`](../roadmaps/fractal-pgap/). That roadmap splits this subsystem into small specs that can be handed to Codex agents one at a time, with each slice producing a testable result before the next begins.

---

## Open Questions

- **Bookmark storage location** — root's `.plexi/bookmarks.toml` is correct for the root instance, but a nested instance might want its own local bookmarks. Decision deferred until Phase 4.
- **Preview render budget** — how often does a child in `"preview"` mode re-render? On change event only? On a timer? On parent request? Likely event-driven to avoid unnecessary work.
- **Hibernate threshold** — at what idle duration (or depth) does it make sense to hibernate an instance? Needs memory profiling data first. Not a Phase 1–6 concern.
- **Inter-agent protocol** — what message format do agents use to communicate through root's event bus? PGAP wraps it, but the payload schema is TBD. Likely a typed `AgentMessage` envelope.
