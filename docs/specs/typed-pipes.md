# Plexi Typed Pipes & Linking Matrix

**Status:** Draft — Phase 1 (small-message control plane)
**Last updated:** 2026-04-14
**Owner:** plexi-core

---

## TL;DR

Typed pipes add named, versioned, strongly-typed channels between Plexi apps on top of the existing JSON-over-stdio protocol. An app declares `inputs` and `outputs` in its manifest under `[app.io]`, each with a `kind` drawn from a central type vocabulary (`core.text`, `core.selection`, `core.file_path`, `core.event`, `core.json`, `core.metric`). When two apps land in the same linked pane group and their declared channels match by kind and name, Plexi auto-wires them — a `pipe_emit` from the producer becomes a `pipe_received` on the consumer with no app needing to import, discover, or even know about the other.

A read-only **linking matrix** overlay (`Cmd+Shift+P`) shows the NxM grid of outputs to inputs in the current group so users can see, audit, and break wires. A per-wire trace inspector makes the last 256 messages on every wire visible with one click — the composition system stays debuggable by default, which is what separates a shippable typed-message primitive from a toy one.

This spec covers Phase 1 only: small structured messages over the existing transport. Audio, video, and high-throughput blobs are deferred to a Phase 2 binary plane spec that will share the channel declaration surface but add a separate transport underneath. The design target for Phase 1 is "smallest primitive that makes composition feel free in the user's hands without making app authors coordinate on anything."

---

## Phase 0 — Shipped (alpha, 2026-04-14)

**Scope:** Parent/child pipe routing only. No manifest wiring, no type checking, no linking matrix UI.

**Protocol additions:**
- `DrawCommand::PipeWrite { channel: str, value: JSON }` — app writes to a named channel
- `PlexiEvent::PipeData { from_app: str, channel: str, value: JSON }` — app receives from connected peers
- `DrawCommand::PipeSubscribe { channel: str }` — no-op in Phase 0, accepted for forward compat

**Routing rules (Phase 0):**
- A PipeWrite from a child pane is routed to its parent app (if any)
- A PipeWrite from a parent pane is routed to all its child apps
- No cross-group routing; no channel filtering; no type validation

**SDK (Python 0.3.0+):**
- `emit.pipe_write(channel, value)` / `ctx.pipe_write(channel, value)`
- `@app.on_pipe_data` → handler(from_app, channel, value, emit)

**Intended use:** File browser → parent selection event, mermaid viewer ↔ sidebar, any two spawned apps that need to exchange lightweight JSON messages.

**What Phase 0 does NOT do:** Manifest channel declaration, type validation, linking matrix UI, cross-group wiring, high-throughput binary streams, named buses.

---

## Motivation

Plexi's v1 app protocol (see `app-infrastructure.md`) lets apps render UI, receive events, and call host APIs. It does not let apps talk to each other. The upcoming `spawn_app` draw command will let one app launch another, but a spawned child still has no typed channel back to its parent, nor to any sibling sharing its pane group. The only escape hatch today is emitting bytes into a shared terminal via `run_in_terminal` — untyped, unstructured, and unusable for composition.

The vision is apps as composable I/O blocks. A file browser emits the currently selected path. A text editor consumes a path and emits text-range selections. A git blame app consumes a text range and emits commit metadata. A commit-browser app consumes commit metadata. None of these apps import each other, link against each other, or discover each other by ID. They declare what they produce and what they consume, and Plexi wires them up when the user drops them into the same linked pane group.

Three concrete v1 use cases justify the work:

1. **Agent pipeline.** A prompt editor (outputs `core.text`) feeds an agent runner (consumes `core.text`, outputs `core.event` for turn-complete and `core.text` for streaming output). A log viewer consumes the events. A diff viewer consumes the output text. Four small single-purpose apps compose into an agent workbench without a single cross-app import.
2. **Notebook 2.0.** A markdown cell emits `core.text`. A Python runner consumes `core.text`, emits `core.json` results and `core.metric` for execution time. A plot app consumes `core.json`. The "notebook" is just a linked pane group — any cell is a standalone app, and reordering cells is just rearranging panes.
3. **Observability dashboard.** Multiple producer apps emit `core.metric` at different rates. A single graph app consumes all `core.metric` wires in the group and draws them. Adding a new metric source is a pane split, not a config change.

Two more that fall out naturally and are worth mentioning: **file-kind routing** (a file browser's `core.file_path` auto-pipes to whichever consumer has the matching kind — image viewer, text editor, PDF reader), and **CI dashboards** (build runners emit `core.event`, a status board consumes them).

What's common to all five is the zero-import property: none of these apps know about each other. The prompt editor in the agent pipeline was written before the agent runner existed. The Python cell in the notebook was written before the plot app existed. The metric emitters on the observability dashboard were written by different teams, perhaps months apart. Typed pipes are the feature that turns "this should be possible" into "this is free" — the composition happens in the user's hands at the pane-drop level, not in the app author's head at the import level.

The contrast with the existing protocol is sharp. Today, to get a selection from a file browser into an editor, you would have to either (a) build both surfaces into a single app and use its internal state, (b) hand-wire them via the host API layer with bespoke `SelectFile` messages that only those two apps understand, or (c) abuse `run_in_terminal` to push a path as a shell command, which is untyped and breaks as soon as the consumer app isn't in the same terminal pane. Every one of those is a non-starter for a general composition story. Typed pipes are the smallest abstraction that fixes all three failures at once.

---

## Non-goals (Phase 1)

Cleanly out of scope:

- **Audio, video, and high-throughput byte streams.** JSON-over-stdio is a small-message control plane. Anything that wants to push hundreds of kilobytes per second, or sample-accurate audio, or compressed video frames, belongs on the Phase 2 **binary plane**, which will add a separate shared-memory or local-socket transport keyed off the same channel declarations. A single sibling spec, `typed-pipes-binary-plane.md`, will cover the transport, backpressure, and zero-copy handoff. Phase 1 deliberately does nothing for it, but the manifest schema and the linking matrix are designed so that adding a binary kind in Phase 2 is purely additive: a new `transport = "binary"` flag on the kind definition and a new message type on the wire.
- **Cross-group wiring via named buses.** Phase 1 auto-wires channels only inside a single linked pane group. Pushing data from one group to another — a global "notifications" bus, a shared "selection" bus across every open workspace — requires an explicit named-bus abstraction and is deferred to Phase 2. A single Phase 1 app that genuinely needs cross-pane-group data can still hit the host filesystem or a local socket on its own; the host does not try to route for it.
- **A draggable visual wire editor.** The Phase 1 linking matrix is a read-only patchbay overlay. You can see the wires, you can break unwanted wires with a click, and you can re-form dropped wires by unbreaking them. You cannot draw a new wire between two channels that don't satisfy the auto-wire rules. Full free-form patch editing — the Max/MSP or TouchDesigner experience of dragging a cable from any output to any input — lands in v2 alongside named buses, because the two features share UI surface and conceptual weight.
- **A central public type registry as a separate GitHub repo.** In Phase 1 the type vocabulary lives in the Plexi repo itself, at `docs/types/*.toml`, with each kind as one TOML file. This is a deliberate YAGNI decision. Extracting to a separate repo with its own review process, versioning, and distribution mechanism costs real effort and only pays off once the type vocabulary has external contributors proposing types the core team doesn't want to maintain directly. Until an external contributor proposes a new type, the vocabulary stays in-repo and is reviewed like any other spec change. The extraction plan — new repo `plexi-types`, published as a TOML bundle fetched at app install time — is noted here so we know the path exists, not as a commitment.

---

## Core concepts

### Channel kinds

A **kind** is a type identifier for a channel payload. Kinds are namespaced and versioned. Phase 1 ships with exactly six core kinds. The `core.*` namespace is reserved for the host; any app can use any core kind without further declaration. Standard-track kinds (future `standard.editor.*`, `standard.agent.*`) and vendor kinds (`vendor.<owner>.*`) are out of scope for the seed set.

The six v1 core kinds:

| Kind | Payload shape | Typical use |
|---|---|---|
| `core.text` | `{ text: string }` | Plain text snippets, prompts, responses, cell contents |
| `core.json` | `{ value: any }` | Arbitrary structured data — the escape hatch, use sparingly |
| `core.file_path` | `{ path: string }` | Absolute or repo-relative file paths |
| `core.selection` | `{ kind: string, … }` (discriminated union) | User selections: cursor range, file list, list index |
| `core.event` | `{ name: string, data?: any }` | Discrete events: button clicks, lifecycle signals, run-complete |
| `core.metric` | `{ name: string, value: number, unit?: string, ts?: number }` | Numeric metrics for observability, dashboards, timings |

Six is the target. Six is the commitment. Any addition to `core.*` must pass a higher bar than any other kind: it has to be useful for at least three already-shipping apps, it has to not be expressible as a trivial discriminated union on top of `core.json`, and it has to be worth teaching every app author about forever.

#### Why these six, and not others

The seed set was picked by walking the thirteen use cases on the whiteboard and asking, for each, "what is the smallest number of typed messages that lets this work." Every case reduced to some combination of: plain text, a structured value, a file path, a user selection, a named event, or a numeric metric. The ones that didn't reduce — live audio samples, video frames, realtime controller data — are the exact ones Phase 1 is deliberately not trying to serve. They are the Phase 2 binary plane's job. Everything that Phase 1 claims to cover reduces cleanly to the six.

The six are also the tipping point for a type system that aims to be memorable. A new app author should be able to learn the vocabulary in five minutes and remember it without a reference. Sixteen kinds is a reference-docs problem. Six is a back-of-hand problem. If the vocabulary grows past ten without an exceptional reason, the feature has lost the plot.

What we intentionally did not include in the seed set, with reasoning:

| Considered | Why not in v1 |
|---|---|
| `core.image` | Expressible as `core.file_path` plus a consumer that knows file extensions. Binary-plane territory once we want live frames. |
| `core.audio` | Same. Binary plane. |
| `core.keystroke` | Already covered by the existing `Key` event surface in `app-infrastructure.md`. Pipes shouldn't duplicate it. |
| `core.command` | Looks useful until you realize every app defines its own commands. A cross-app `core.command` kind would force a naming convention no one would actually follow. |
| `core.log` | Already solved by the existing `log()` SDK surface. Pipes are not the place for log forwarding. |
| `core.process_status` | Trivially a `core.event` with a conventional name. Not its own kind. |
| `core.color` | Too narrow. Two apps that actually need to pipe colors can use `core.json` or a vendor kind. |
| `core.position` | Too narrow and too overloaded. Text cursor? Mouse? World-space? Becomes a mess. |

#### `core.text`

The smallest useful payload. A single string.

```json
{ "text": "Hello world" }
```

| Field | Type | Required | Description |
|---|---|---|---|
| `text` | string | yes | The text content. No length limit at the type level, but the transport limit in Phase 1 is ~64 KB per message. Larger bodies should use `core.file_path` to hand off a file instead. |

Example producers: a prompt editor, a markdown cell, a terminal output capture.
Example consumers: an agent runner, a text-to-speech app, a translation app.

The deliberate smallness of this type is its strength. `core.text` is the lingua franca of the control plane — any app that manipulates text can speak it without coordinating on a schema. Two apps that want to exchange richer structure should reach for `core.selection` or `core.json`, not for a hypothetical `core.markdown` or `core.prompt`. The vocabulary stays small by pushing specialization outward, not inward.

#### `core.json`

Arbitrary structured data. This is the escape hatch: if a real type doesn't exist yet, two apps can agree on a JSON shape via convention and pipe it through `core.json`. It is deliberately weaker than the other kinds — no schema check beyond "it parses as JSON" — and app authors are encouraged to upgrade to a stronger kind as soon as one exists.

```json
{ "value": { "foo": 1, "bar": [true, false] } }
```

| Field | Type | Required | Description |
|---|---|---|---|
| `value` | any | yes | Any JSON-representable value. |

Example producers: a Python notebook cell emitting a result dict, a SQL runner emitting a row set.
Example consumers: a plot app, a table viewer, a JSON inspector.

`core.json` is the reason we can ship the seed set at six kinds instead of sixteen. Every use case that would otherwise demand its own kind — a SQL row set, a build log event, a chart configuration — can express itself as `core.json` with a convention between the producer and the consumer. The convention lives in prose, not in the type system. The cost is that the host cannot validate the payload beyond "it parses," so a misshapen dict between two apps becomes a runtime bug rather than a load-time error. The benefit is that the vocabulary stays small and the apps still compose. When a use case that was being served by `core.json` grows enough users, it graduates to a proper kind — first as a vendor kind, then standard, then possibly core. `core.json` is the nursery.

#### `core.file_path`

An absolute or repo-relative path. This is the most-used wire type in the vision, because it drives the file-kind-routing story.

```json
{ "path": "/Users/ian/code/plexi/src/main.rs" }
```

| Field | Type | Required | Description |
|---|---|---|---|
| `path` | string | yes | An absolute path, or a path relative to the consumer's launch directory. Consumers should treat relative paths as resolved against their own launch dir, not the producer's. |

Paths are not validated at emit time — the consumer may find the file missing, unreadable, or outside its capability scope, and must handle that case. The host does not check path existence.

Example producers: a file browser emitting the selected path, a git log app emitting the blamed file, a find/grep app emitting match locations.
Example consumers: a text editor opening the file, an image viewer loading the image, a hex viewer showing bytes.

The file-path kind is also the spine of the "file-kind routing" composition pattern, one of the cleanest wins of the whole system. In a group with a file browser plus multiple file-kind viewers (image, PDF, text, hex), the file browser has no idea which consumers exist — it emits paths and trusts the group to route appropriately. Each consumer filters by extension or magic bytes and either renders or ignores. Adding a new viewer is a pane split, not a patch to the file browser. The composition is held together by one `core.file_path` channel and the auto-wire algorithm, nothing more. This is the smallest concrete demonstration of what typed pipes are actually for.

#### `core.selection`

A user-selected region. This is a discriminated union — the `kind` discriminator tells the consumer which variant the payload is.

```json
{ "kind": "text_range", "path": "src/main.rs", "start_line": 12, "end_line": 20 }
```

| Variant | Fields |
|---|---|
| `text_range` | `path: string`, `start_line: int`, `end_line: int`, `start_col?: int`, `end_col?: int` |
| `file_list` | `paths: string[]` |
| `list_item` | `index: int`, `label?: string` |

Line numbers are 1-indexed. Columns, when present, are 0-indexed. An editor selecting `src/main.rs` lines 12–20 produces the example above. A file browser with three files checked produces a `file_list` variant. A command palette with item 3 highlighted produces a `list_item`.

Example producers: editors, file browsers, list UIs, command palettes.
Example consumers: git blame (on `text_range`), bulk rename (on `file_list`), anything that needs "what is the user looking at right now."

Consumers of `core.selection` must switch on the `kind` discriminator and handle unknown variants gracefully. An editor that understands only `text_range` ignores `list_item` and `file_list` payloads — no error, no drop, just a no-op. This forward-compatibility is how new variants get added in a minor version bump without breaking existing consumers: a v1.1 consumer can emit `code_symbol` (a hypothetical new variant), and a v1.0 consumer that doesn't know what `code_symbol` means simply skips the message. The same discipline applies to every discriminated union in the registry.

#### `core.event`

A discrete named event. Lighter than `core.json`; the consumer can switch on the name without parsing a payload.

```json
{ "name": "run_complete", "data": { "exit_code": 0, "duration_ms": 842 } }
```

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | yes | Event name. Convention: `snake_case`, verb or noun phrase. Not validated. |
| `data` | any | no | Optional payload. Treat as `core.json` semantics. |

Example producers: a build runner emitting `run_start` and `run_complete`, a button widget emitting `click`, a lifecycle controller emitting `app_ready`.
Example consumers: a log viewer, a status board, a toast notifier.

A meaningful chunk of event payloads will not carry structured data — a `click` event, a `focus_gained`, a `saved`. The optional `data` field exists for the cases that need it (`run_complete` with `exit_code`, `selection_changed` with a new index) without forcing the trivial cases to pack empty objects. The convention is: start with just a name, add `data` only when a real consumer asks for it. `core.event` is not a replacement for properly typed domain channels; it is the loose cousin that covers the long tail of discrete notifications that don't deserve a kind of their own.

#### `core.metric`

A single numeric data point with a name, optional unit, and optional timestamp. The observability-dashboard use case rides on this.

```json
{ "name": "cpu.load", "value": 0.72, "unit": "ratio", "ts": 1744668400 }
```

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | yes | Metric name. Convention: `dot.separated.lowercase`. |
| `value` | number | yes | Numeric value. Integer or float. |
| `unit` | string | no | Free-form unit hint: `ms`, `bytes`, `ratio`, `count`, etc. |
| `ts` | number | no | Unix timestamp in seconds. If omitted, the host stamps it on receipt. |

Example producers: a process-stats app emitting `cpu.load` and `mem.rss`, a benchmark runner emitting timings, a Python cell emitting execution duration.
Example consumers: a graph app, a dashboard tile, a logging sink.

Metrics are deliberately single-sample, not batched. A producer that wants to emit a hundred metric points in quick succession sends a hundred `pipe_emit` messages. This is fine for the observability-dashboard scale (a handful of producers sampling at a few Hz each); it stops being fine at the "update every audio frame" scale, at which point the producer belongs on the Phase 2 binary plane. The 1 KB/sec per channel target applies here too.

The `name` field inside the metric payload is distinct from the channel name. A single channel can carry multiple metric names — e.g., a process-stats app has one output channel `stats: core.metric` and emits `cpu.load`, `mem.rss`, and `disk.io` on it alternately. Consumers switch on `payload.name` to route appropriately. This avoids an explosion of channels for dashboards that want many parallel metric streams: declare one output, emit many names. A more opinionated design would force one channel per metric name; we looked at it and decided the flexibility is worth the mild cognitive load, because dashboard producers genuinely want variadic metric streams.

### Channel declarations

Apps declare channels in their manifest, under a new `[app.io]` table. This is strictly additive to the manifest schema in `app-infrastructure.md` — existing manifests without `[app.io]` remain valid and simply declare no channels.

```toml
[app.io]
inputs = [
  { name = "selection", kind = "core.selection", required = false },
  { name = "playhead",  kind = "core.metric",    required = false },
]
outputs = [
  { name = "selection", kind = "core.selection" },
  { name = "main_text", kind = "core.text" },
]
```

Field semantics:

| Field | Applies to | Required | Description |
|---|---|---|---|
| `name` | input, output | yes | Per-app channel name, chosen by the app author. Scoped to this app only. Two apps can both name an input `selection` without conflict. |
| `kind` | input, output | yes | Kind from the type vocabulary. Validated against the registry at app load — unknown kinds fail load with a clear error. |
| `required` | input | no (default `false`) | If `true`, the app refuses to start unless at least one matching wire is resolved at init time. Used rarely; most apps should handle "no data yet" gracefully. |
| `version` | input, output | no (default `^<current major>`) | Semver constraint on the kind version. See Versioning below. |

Names are free-form but subject to two rules: ASCII `[a-z0-9_]`, and unique within a single app's input list and within its output list. An app may use the same name for both an input and an output (e.g., a "filter" app that passes `selection` through).

A full example manifest for a text editor in a composition-ready form:

```toml
[app]
id      = "com.example.editor"
name    = "Editor"
version = "1.2.0"

[capabilities]
filesystem = "read_write"

[app.io]
inputs = [
  { name = "path",      kind = "core.file_path", required = false },
  { name = "goto",      kind = "core.selection", required = false },
]
outputs = [
  { name = "selection", kind = "core.selection" },
  { name = "buffer",    kind = "core.text" },
  { name = "saved",     kind = "core.event" },
]
```

Read: "If there's a file-path producer in my group, open that file. If there's a selection producer, jump to it. I'll tell the group what the user has selected, what's in the current buffer, and when I save." Three outputs and two inputs is a comfortable size for a mid-complexity app. Simpler apps may declare just one of each; an app with no pipes at all simply omits the `[app.io]` block.

The host parses `[app.io]` at app load. Parse failures (unknown kind, malformed array, reserved name) fail app load with an error in `plexi.log` naming the app and the problem. There is no partial load — either the manifest is valid or the app doesn't start.

#### Composition example: agent workbench

A concrete end-to-end composition, four apps in one linked pane group, wired entirely by the host.

```
┌─────────────────────────────┐   ┌─────────────────────────────┐
│  prompt_editor              │   │  agent_runner               │
│                             │   │                             │
│  [app.io]                   │   │  [app.io]                   │
│  outputs = [                │   │  inputs  = [                │
│    { name="prompt",         │──▶│    { name="prompt",         │
│      kind="core.text" }     │   │      kind="core.text" }     │
│  ]                          │   │  ]                          │
│                             │   │  outputs = [                │
└─────────────────────────────┘   │    { name="response",       │──┐
                                  │      kind="core.text" },    │  │
                                  │    { name="lifecycle",      │  │
                                  │      kind="core.event" }    │  │
                                  │  ]                          │  │
                                  └─────────────────────────────┘  │
                                                                   │
┌─────────────────────────────┐   ┌─────────────────────────────┐  │
│  event_log                  │   │  diff_viewer                │  │
│                             │   │                             │  │
│  [app.io]                   │   │  [app.io]                   │  │
│  inputs  = [                │◀──│  inputs  = [                │◀─┘
│    { name="events",         │   │    { name="response",       │
│      kind="core.event" }    │   │      kind="core.text" }     │
│  ]                          │   │  ]                          │
└─────────────────────────────┘   └─────────────────────────────┘
```

The user drops all four apps into a single linked pane group. The resolver wires `prompt_editor.prompt` → `agent_runner.prompt` (exact name match). It wires `agent_runner.response` → `diff_viewer.response` (exact name match). It wires `agent_runner.lifecycle` → `event_log.events` (unambiguous kind match: `event_log.events` is the only `core.event` consumer in the group, and the resolver takes the kind-only fallback). Four apps, three wires, zero configuration. Any of these four apps can be swapped out for a different implementation with the same channel declarations and the composition keeps working.

### Channel messages on the wire

Two new entries in the existing draw command set (see `app-infrastructure.md` for the base set). These extend the App → Plexi and Plexi → App surfaces without replacing anything.

| Type | Direction | Payload |
|---|---|---|
| `pipe_emit` | App → Plexi | `{ channel: string, payload: object }` |
| `pipe_received` | Plexi → App | `{ channel: string, payload: object, from_pane?: id }` |

On `pipe_emit`, Plexi looks up the producer's manifest, confirms that `channel` is one of its declared outputs, validates that `payload` matches the kind's schema, and then dispatches the message as `pipe_received` to every matching consumer in the same linked pane group. `from_pane` on the consumer side is the pane ID of the producer — consumers may use it to tag origin in their UI or to ignore self-loops.

Validation failures (unknown channel, payload schema mismatch, unknown kind) are logged at `warn` level tagged `app::<app_id>` and the message is dropped. Neither side is killed. This is a deliberate choice: channel misuse is a bug, not a security violation, and crashing an app over a malformed pipe is worse than dropping the message and surfacing the warning in the log and the patchbay overlay.

Validation is deep enough to cover required fields and types on the top-level variant, plus discriminator variants on unions like `core.selection`. It is not a full JSON Schema engine; the Phase 1 host validator walks the kind's TOML definition and checks presence and primitive types.

A worked wire example, end to end. The user drops a file browser and an image viewer into a linked pane group. The file browser emits:

```json
{ "type": "pipe_emit", "channel": "selected", "payload": { "path": "/Users/ian/pics/cat.jpg" } }
```

The host:

1. Looks up the file-browser manifest, finds `selected` in the outputs list, confirms kind is `core.file_path`.
2. Walks the `core.file_path` payload schema, confirms `path` is present and is a string.
3. Runs the resolver over the current group, finds `image_viewer.current` as a matching-kind consumer.
4. Sends to the image viewer:

```json
{ "type": "pipe_received", "channel": "current", "payload": { "path": "/Users/ian/pics/cat.jpg" }, "from_pane": 42 }
```

5. Records the message on the wire's trace buffer (256 most recent, per wire) for the patchbay inspector.
6. Returns control to the producer's render loop without waiting for the consumer to process.

Total host work is a handful of hashmap lookups plus one validation walk. In a group of typical size (2–6 panes) this runs in under 50 microseconds on a reasonable machine. The design target is that pipes should never be the frame-rate bottleneck at Phase 1 scale; if a group is emitting enough messages to affect frame time, the profiler should point at JSON encoding on the producer side long before it points at the resolver.

---

## Linking matrix

### Auto-wire algorithm (v1 — implicit)

Two channels auto-wire iff all of the following are true:

1. They live in the **same linked pane group**. Across groups: no wiring in v1.
2. The producer's declared `kind` exactly matches the consumer's declared `kind`, including namespace and major version.
3. Either **the producer's `name` matches the consumer's `name` exactly**, or **no name match is possible for this kind in this group and the match by kind alone is unambiguous** (defined below).

"Unambiguous match by kind alone" means: for a given (producer app, output channel, kind), there is exactly one consumer in the group whose input has that kind and none of that consumer's inputs happens to match by name against any output of any producer in the group. In practice, this lets a file-browser `selected` pipe to an editor `path` without the user ever realizing the names are different, as long as no other `core.file_path`-consuming app is also in the group.

Worked example. Three apps in one linked pane group:

- `file_browser` — outputs: `selected: core.file_path`
- `text_editor` — inputs: `path: core.file_path`
- `image_viewer` — inputs: `current: core.file_path`

The resolver asks: for `file_browser.selected` (kind `core.file_path`), what consumers in this group take `core.file_path`? Answer: `text_editor.path` and `image_viewer.current`. Neither matches `selected` by name. Neither is singled out by any other rule. So **both** consumers are wired — the producer multicasts. When the user picks `cat.png` in the file browser, both consumers get the path. The image viewer shows the image; the editor shows the hex or a friendly "binary file" message.

If the user wants a 1:1 wire — say, to route only images to the image viewer — they open the patchbay overlay and click the wire between `file_browser.selected` and `text_editor.path` to break it. That dot goes from solid to hollow; Plexi stops routing that specific pair. The break is remembered for the life of the group (see Lifecycle, below).

Resolution proceeds deterministically. The resolver iterates pane creation order (oldest first) and wires each producer's outputs against all consumers. Multicast is allowed; fan-in is allowed (see Routing). Breaks override wires but do not override producer order.

In pseudocode, the resolver is roughly:

```
for producer_app in group (oldest first):
  for output in producer_app.outputs:
    matches = [
      (consumer_app, input)
      for consumer_app in group if consumer_app != producer_app
      for input in consumer_app.inputs
      if input.kind == output.kind
         and compatible_version(input.version, output.version)
    ]
    name_matches = [m for m in matches if m.input.name == output.name]
    if name_matches:
      wire_all(name_matches)
    elif len(matches) >= 1 and unambiguous(matches):
      wire_all(matches)
    else:
      # no auto-wire; leave as compatible-but-unwired in the patchbay
      pass
    apply_user_breaks(wires)
```

The exact definition of `unambiguous(matches)` is: "none of the candidate consumers have an input of this kind that matches any *other* producer's output by name." In practice this rule keeps the common case (one producer, one consumer per kind) cheap and the collision case visible rather than silent.

### The patchbay overlay (read-only in v1)

A small UI surfaced via `Cmd+Shift+P`, or by clicking a sticky badge on the linked-pane group outline. Hidden by default — users open it to debug composition. Scoped to the currently focused linked pane group.

The overlay renders an NxM grid: rows are outputs of each pane, columns are inputs of each pane. Each cell is one of:

| Glyph | Meaning |
|---|---|
| `●` | Active wire — message flow is live |
| `○` | Compatible but broken by the user (click to re-form) |
| `⋅` | Compatible and not auto-wired (e.g., name mismatch with no unambiguous fallback) |
| (blank) | Incompatible — kinds don't match |
| `⚠` | Duplicate producer (see Routing) or schema-rejected payload recently |

Example rendering for the file-browser / editor / image-viewer case above:

```
                                     text_editor.path   image_viewer.current
file_browser.selected (core.file_path)        [●]                  [●]
```

Wires are color-coded by kind for fast visual scanning. The key at the bottom of the overlay reminds the user which color is which kind. Hovering a cell shows the most recent message timestamp and the payload preview (first ~200 bytes, truncated). Clicking an `●` breaks the wire. Clicking an `○` re-forms it. Everything else is read-only in v1.

The overlay is scoped to the currently focused linked pane group only. It does not show wires in other groups, because in v1 there are no cross-group wires to show. A status line across the top of the overlay reports the group's pane count, total wire count, and total messages in flight in the last second — a small diagnostic signal that lets users gauge composition health at a glance.

When an app crashes or its process exits, its row and column in the matrix turn grey and a tombstone glyph appears. Clicking the tombstone opens the most recent lines of `plexi.log` filtered to that app's target, giving a one-click path from "why did this wire go dead" to the actual error message. This is one of the affordances that most distinguishes a production composition system from a toy one: the gap between "something broke" and "I found the error" must be a single click.

A small "trace" button per wire opens the **trace inspector** — a scrolling log of every message that crossed that wire with timestamp, size, and a JSON preview. This is the one debugging affordance that is non-negotiable for Phase 1: composition without a message tracer is unshippable because the first time two apps disagree about a payload shape, the user has no way to see it.

Trace inspector layout, sketched:

```
wire: file_browser.selected → image_viewer.current  (core.file_path ^1.0)
  17:42:03.112   142 B   { "path": "/Users/ian/pics/cat.jpg" }
  17:42:01.488   146 B   { "path": "/Users/ian/pics/dog.jpg" }
  17:41:59.873   152 B   { "path": "/Users/ian/pics/bird.jpg" }
  17:41:57.201   REJECTED  missing required field 'path'
```

Rejected messages appear inline in red with the validator error. The buffer is a fixed 256-message ring; older messages fall off. The memory cost per wire is capped at ~64 KB (256 × 256 B preview) and the wire count per group is capped implicitly by pane count. The inspector is opened on demand, so the host does not need to render it per frame — the ring buffer is maintained even when the inspector is closed, so opening it shows the recent past, not just the present.

### Versioning of channels

Each kind in the registry declares a semver. App manifests can pin a constraint:

```toml
[[app.io.outputs]]
name    = "selection"
kind    = "core.selection"
version = "^1.0"
```

If `version` is omitted, the default is `^<current major>` of the kind at app install time. The wire-time compatibility rule is semver-caret:

- Same major, higher minor or patch on one side: compatible, wire forms.
- Different majors: incompatible, wire does not form. The patchbay overlay shows an `⚠` with "version mismatch" on hover.
- Major bump in a kind definition means every app pinned to the old major stops wiring against producers on the new major until the manifest is updated.

Kind authors are expected to major-bump conservatively. Adding optional fields is a minor bump. Removing a field, renaming a field, or changing a field's type is a major bump. The meta-schema validator (see Phase 1f) refuses to merge a TOML change that violates this rule unless the kind's `version` is also bumped appropriately.

Precise rules:

| Change to a kind definition | Bump |
|---|---|
| Typo fix in `description` | patch |
| New example in `examples` | patch |
| New optional field on an existing variant | minor |
| New variant on a discriminated union | minor |
| Add a new required field | major |
| Rename a field | major |
| Change a field's type | major |
| Remove a field | major |
| Remove a variant | major |
| Change the discriminator name | major |

The meta-schema validator runs in CI on any PR that touches `docs/types/`. A PR that violates these rules fails CI with a specific error pointing at the diff. The rules are also documented in the meta-schema field reference so an external contributor writing their first type has them in front of them.

---

## The in-repo type registry (for now)

The Phase 1 type registry lives at `docs/types/` inside the Plexi repo. Each kind is one TOML file. The directory layout is:

```
docs/types/
  core/
    text.toml
    json.toml
    file_path.toml
    selection.toml
    event.toml
    metric.toml
  standard/        # reserved, empty in v1
  vendor/          # reserved, empty in v1
```

At app load, the host reads every file under `docs/types/` and builds an in-memory registry keyed by `<namespace>.<name>`. App manifests that reference an unknown kind fail to load with an error naming the kind and the manifest path.

### Meta-schema

The format of a type definition file. This is itself fixed and changes require a Plexi protocol version bump.

```toml
name        = "selection"
namespace   = "core"              # core | standard.<topic> | vendor.<owner>
version     = "1.0.0"
status      = "stable"            # proposed | stable | deprecated | removed
maintainer  = "plexi-core"        # email, handle, or team name

description = """
A user-selected region. Emitted by editors, file browsers, and list UIs
to tell downstream consumers what the user is currently looking at or
pointing at. A discriminated union over text ranges, file lists, and
list items — further variants may be added as a minor version bump
provided they leave existing variants unchanged.
"""

# One of the two schema blocks below must be present.
# For a simple object payload, use [payload_schema].
# For a discriminated union, use [[payload_schema.variants]].

[payload_schema]
discriminator = "kind"

[[payload_schema.variants]]
kind = "text_range"
fields = [
  { name = "path",       type = "string",  required = true  },
  { name = "start_line", type = "integer", required = true  },
  { name = "end_line",   type = "integer", required = true  },
  { name = "start_col",  type = "integer", required = false },
  { name = "end_col",    type = "integer", required = false },
]

[[payload_schema.variants]]
kind = "file_list"
fields = [
  { name = "paths", type = "string[]", required = true },
]

[[payload_schema.variants]]
kind = "list_item"
fields = [
  { name = "index", type = "integer", required = true  },
  { name = "label", type = "string",  required = false },
]

[examples]
text_range = '{ "kind": "text_range", "path": "src/main.rs", "start_line": 12, "end_line": 20 }'
file_list  = '{ "kind": "file_list", "paths": ["a.txt", "b.txt"] }'
list_item  = '{ "kind": "list_item", "index": 3, "label": "Commit 9f2a" }'
```

#### Meta-schema field reference

| Field | Required | Description |
|---|---|---|
| `name` | yes | Short kind name, ASCII `[a-z0-9_]`. Must match the filename. |
| `namespace` | yes | One of `core`, `standard.<topic>`, `vendor.<owner>`. Determines review process and directory. |
| `version` | yes | Semver. Bumped on any schema change. |
| `status` | yes | Lifecycle state. `proposed` kinds are visible in the registry but apps targeting stable should not pin to them. `stable` is the default for core. `deprecated` kinds still wire but emit a warning on load. `removed` kinds fail app load. |
| `maintainer` | yes | Who's on the hook for semantic questions. |
| `description` | yes | Free-form prose. Aim for a paragraph: what the kind means, what it's used for, what it is deliberately not. |
| `payload_schema` | yes | The payload shape. Either a flat fields table or a `variants` array for discriminated unions. |
| `payload_schema.discriminator` | if union | Name of the field that selects the variant. |
| `payload_schema.variants[].kind` | yes (union) | Value of the discriminator for this variant. |
| `payload_schema.variants[].fields` | yes (union) | Field list for this variant. |
| `payload_schema.fields` | yes (flat) | Field list for non-union payloads. |
| `fields[].name` | yes | Field name. |
| `fields[].type` | yes | One of: `string`, `integer`, `number`, `boolean`, `any`, `<type>[]` (array of), or `object`. |
| `fields[].required` | yes | Whether the field must be present. |
| `examples` | no | Table of named examples, each a JSON string. Useful for docs and for the meta-schema validator's round-trip tests. |

The six v1 core kinds will each ship as one file under `docs/types/core/`. Creating those files is explicitly a follow-up to this spec, not part of it — this spec commits only to the format and location so that an external contributor who reads the spec can write a new kind definition without further documentation.

### The extraction criterion

The in-repo location is deliberately provisional. Extraction to a standalone `plexi-types` repository happens exactly when one of the following is true:

1. An external contributor (not on the core team) proposes a new `standard.*` or `core.*` kind and the proposal is accepted. This signals that the vocabulary has grown beyond "the core team's mental model" and needs an independent review process.
2. The Plexi repo size becomes dominated by type definitions. Not a realistic concern for the foreseeable future — each TOML file is a few hundred bytes — but noted for completeness.
3. An independent runtime (a Plexi-compatible host written in another language, a browser-embedded variant, a headless CI adapter) needs to read the type definitions without cloning the full Plexi source tree. At that point, a standalone `plexi-types` package published to a language-agnostic registry is obviously the right move.

Until one of those three is true, the registry stays in-repo. The migration path is mechanical: move `docs/types/` to a new repo, publish it as a TOML bundle, add a manifest step that fetches and caches the bundle at app install time. No app code changes because apps never import types directly — they reference them by name, and the host resolves the reference. Total migration cost is estimated at a day of work. Deferring is cheap; committing early is not.

### Registry load behavior

At Plexi startup, the host walks `docs/types/` and reads every `*.toml` file. Parse errors on a single file log a `warn` and skip that file — the registry loads without the broken kind, and any app referencing it fails at app load time with a clear error. This is graceful degradation: a bad type file does not brick the host.

The loaded registry is cached in memory for the lifetime of the process. A dev-mode hot-reload watches `docs/types/` for changes and rebuilds the registry when a file is modified; the SDK sees the new version on next app load. The production binary ships with the registry embedded at build time via `include_str!` on a generated Rust module, so production hosts do not need to touch the filesystem at all after startup.

Every type definition also contributes a short entry to the `plexi types` CLI subcommand, which prints the registry and lets a user confirm what's available before authoring an app.

---

## Routing within and across linked pane groups

Phase 1 routing rules, in full:

- **Same linked pane group → auto-wire.** This is the only place wires form in v1.
- **Different groups → not allowed.** Cross-group wiring is the v2 named-bus feature. An app that wants global reach must either live in every group (fine, panes can be duplicated), or wait for v2.
- **Multicast is allowed.** One producer channel may wire to many consumer channels of matching kind in the same group. Every consumer receives every message. Producers do not learn how many consumers they have — the host tracks that but does not leak it.
- **Fan-in is allowed.** Multiple producers of the same kind may wire to a single consumer channel. The consumer receives messages in the order the host processes them — timestamp order at receipt, tie-broken by pane creation order. Consumers that care about ordering should not assume anything stronger than "eventually consistent, same-group."
- **Duplicate producer (same `(kind, name)`):** when two producers in the same group declare outputs with the same kind **and** the same name, the **first producer in pane creation order** wins. Subsequent duplicates are marked with `⚠` in the patchbay overlay and a warning goes to the log (`app::<app_id> duplicate output channel 'selection' (core.selection), shadowed by pane <id>`). This is a safety net, not a feature — apps should avoid relying on it. Two producers of the same (kind, name) is almost always a composition mistake.
- **Self-loops:** an app that is both producer and consumer of the same channel in the same group does not wire to itself. The resolver explicitly skips the reflexive pair. Apps that want internal echo should do it in-process without going through the pipe.
- **Cycles through multiple panes are allowed.** A → B → C → A is a legal topology. The host does not attempt cycle detection. If an app re-emits in response to a received message without a termination condition, it will produce a runaway loop. This is the app's bug, not the host's; however, the host does apply a per-group rate limit as a safety net: if any wire exceeds 100 messages per second sustained, the patchbay flags it with `⚠` and a "possible feedback loop" warning in the log. The messages still flow — throttling would be worse than the loop — but the user has a loud signal to investigate.
- **Message ordering within a single producer channel is preserved.** Messages emitted in order on the same channel arrive at every consumer in the same order. Ordering across different channels, or across different producers into the same consumer, is not guaranteed beyond receipt order at the host.

---

## Lifecycle of wires

What happens when the shape of a group changes.

- **Hot reload of an app.** On reload, the host re-reads the manifest. Channel declarations from the new manifest replace the old ones atomically. The wire resolver re-runs against the group. Wires whose declarations still exist (same `(name, kind)` on both sides) are preserved — the consumer does not see a disconnect event. Wires whose declarations vanished are silently dropped. A single log line summarizes: `app::<app_id> reloaded; N wires preserved, M dropped`. The trace buffer for dropped wires is retained for 60 seconds in case the user hot-reloads again and wants to see the last traffic.
- **App death.** When an app exits (cleanly or via crash), every wire with that app as producer or consumer is removed. The linked pane group survives; the surviving apps simply have fewer wires. A newly spawned app in the same pane slot re-runs the resolver and may form new wires.
- **Pane move in.** When a pane is dragged into a linked group, the resolver runs over the group with the new pane included. New wires form wherever the new app's declarations match.
- **Pane move out.** When a pane is dragged out of a linked group, all wires touching that pane are removed. The remaining panes keep their wires with each other.
- **Move in → out → in** (user dragging experimentally). Wires re-form on each move-in and disappear on each move-out. Break overrides set by the user in the patchbay are remembered per pane pair for the life of the group, not per individual wire instance — re-forming a group that previously had a broken wire preserves the break.
- **Group dissolution.** When the last pane leaves a linked group, the group is destroyed. All break overrides are lost. Trace buffers are freed.
- **Session restore.** When Plexi restarts and restores a saved workspace, linked pane groups are rehydrated with their panes. The resolver then runs fresh on each group as if every pane had just moved in. Break overrides from the previous session are persisted alongside the workspace layout in `~/.plexi/workspace.toml` and re-applied after the resolver runs. Trace buffers are not persisted — recent history is a live-session concept only.
- **Producer delayed start.** A pane that spawns a slow-starting subprocess may take seconds to complete its `Init` handshake. Until the handshake is done, the app's `[app.io]` declarations are not in the wire table, and no wires form against it. When the handshake completes, the resolver runs the group again and wires form retroactively. Consumers that joined the group before the producer finished starting simply see "no messages yet" for those wires until the producer finishes startup and begins emitting.

These rules are simple and cover every composition case we can think of without introducing a lifecycle state machine per wire. Apps should assume: at any given render frame, they may receive zero, one, or many `pipe_received` messages on any declared input, with no per-wire "connected" or "disconnected" notification. If an app wants to know whether a wire is currently live, it tracks the time of the last message — a heartbeat is the consumer's responsibility, not the host's.

This is a deliberate simplification. Every component-model IDL system that tries to expose fine-grained connection lifecycle to clients ends up with half its complexity in that layer: partial-connect states, reconnect-with-last-value replay, connection-health debouncing, stale-wire warnings. We do not want any of it. The host model is: the wire exists or it doesn't, and the consumer's only ground truth is the last message it received. An app that wants "oh the editor just closed" behavior implements a timeout, not a subscription. An app that needs true transactional coordination belongs on a different primitive and can wait for v2.

#### Last-value replay

One small concession to lifecycle: on resolver run, if a wire is newly formed and the producer side has a recent cached last value for that channel, the cached value is replayed to the new consumer as its first `pipe_received`. This prevents the "app joined the group, knows nothing, and waits for the next update" dead-screen problem. The cache is a single slot per (producer, output channel), overwritten on every emit, cleared when the producer dies. A consumer that doesn't want replay (e.g., one that treats every message as a transient event) ignores the `replayed: true` flag on the message.

---

## Capability gating

A new optional capability block, declared alongside the existing `[capabilities]` table in the manifest:

```toml
[app.io.permissions]
emit_kinds    = ["core.selection", "core.file_path"]
consume_kinds = ["*"]
```

Semantics:

- **If `[app.io.permissions]` is absent:** the app's declared `[app.io].outputs` and `[app.io].inputs` are the authoritative capability set. The app can emit anything it declared as an output, and consume anything it declared as an input. This is the common case.
- **If `[app.io.permissions]` is present:** the lists act as a **deny-list narrower** on top of the manifest declarations. `emit_kinds` lists the kinds the app promises not to emit beyond; any declared output whose kind is not in the list is rejected at load time with an error. `consume_kinds` does the same for inputs. The string `"*"` means "any kind allowed by my declarations." Empty list means "nothing, even if declared."
- **Why have it at all.** High-trust apps (especially future agent hosts) can advertise channel hygiene: "even though my output declarations include `core.event`, I am giving up the right to emit it." This gives the user and the host a stricter contract than the declarations alone. It also gives the Permissions Manager (see `app-infrastructure.md`) a place to render per-app toggles for each kind.
- **User overrides.** `~/.plexi/permissions.toml` may further tighten an app's I/O permissions with the same schema. The user override is intersected with the manifest declaration and the manifest permissions block — the effective set is the narrowest of the three. The user cannot widen an app's emit/consume set beyond what the manifest declared.

This is not a full sandbox — a hostile app can still leak information via its legitimately declared outputs, and a misbehaving kind schema can still be used as a side channel. The `[app.io.permissions]` block is a hygiene tool, not a security boundary.

The Permissions Manager app (see `app-infrastructure.md` Phase 6) renders `[app.io.permissions]` as a per-app table in its Apps tab, alongside the existing filesystem and network capabilities. The user sees "this app emits `core.selection` and `core.file_path`" and can toggle each one off. Toggled-off kinds are enforced by the host at `pipe_emit` time — the emit is rejected, the message is dropped, a single line goes to `plexi.log`. The app does not learn it was blocked; it sees its own emit succeed locally and never gets a feedback message telling it the host dropped the payload. This is a deliberate choice: letting apps observe their own sandbox reduces the sandbox's usefulness for adversarial scenarios.

Global kill switches in `~/.plexi/permissions.toml` work the same way as existing capability kill switches: a global entry overrides every per-app override, so the user can wholesale disable `core.event` across all apps if they decide they don't trust the surface.

---

## Drawbacks

Every major red-team point from the vision conversation, with its Phase 1 mitigation. None of these are fully solved; honesty about what's unsolved is the point.

### 1. Type system rot

Six kinds today, sixty in two years if we don't curate. Every project of this shape — IDL-based component systems, message-passing frameworks, plugin protocols — drifts toward a bloated type vocabulary that nobody can hold in their head and that new apps keep re-inventing subtly different variants of.

**Mitigation.** Ruthless review on additions to `core.*`. The vendor namespace (`vendor.<owner>.*`) is the escape valve: anyone can define a vendor kind, it lives in their own repo/plugin, and it wires happily with any app that imports it. Standard-track kinds (`standard.<topic>.*`) are the promotion path for vendor kinds that prove widely useful. The bar for `core.*` promotion is: used by three+ unrelated apps in production and reviewed by the core team. The bar stays high on purpose.

The registry itself makes rot visible. The `status` field on each type definition lets us mark kinds as `deprecated` and eventually `removed`. A deprecated kind still wires but emits a log warning on load, giving a visible deadline for apps to migrate. A removed kind fails to load. This gives us an actual pruning mechanism — the vocabulary can shrink as well as grow. Every typed-message system that lacks a deprecation path eventually accumulates a decade of half-used types; we commit to having one from day one.

### 2. Semantic drift on shared types

Two apps both emit `selection`, but one means "text under cursor" and the other means "highlighted item in a list." Both are `core.selection`, both are valid, both have the same name, but they mean different things in context, and the auto-wire happily connects them to the same consumer.

**Mitigation.** The `core.selection` kind is a discriminated union precisely to force producers to declare which variant they emit, so a consumer that only understands `text_range` can ignore `list_item` payloads. The patchbay overlay surfaces collisions with a `⚠` glyph so users can see "I have two different things called `selection` in this group" before debugging why their wire produces nonsense. It is still possible to wire semantically unrelated things together; the mitigation is visibility, not prevention.

The long arc here is that app authors learn, over time, which names are load-bearing in the ecosystem. Two editors both naming their output `selection` is good — that's the convention working. Two completely different kinds of UI both naming their outputs `selection` is a mistake that will produce bad compositions, and the patchbay makes that mistake visible in under ten seconds of user debugging. We are not trying to eliminate the problem, we are trying to make its cost small enough that the feature is still worth having.

### 3. Performance

JSON-over-stdio is fine for small control messages and terrible for high-throughput data. A ~1 KB/sec sustained rate per channel is inside the comfort zone. A pane emitting thousands of metric updates per second will start to visibly impact frame rate, and an audio sample stream is simply a non-starter.

**Mitigation.** Document the ~1 KB/sec per channel sustained and ~64 KB per message ceiling as hard Phase 1 constraints. The manifest validator rejects any kind that looks like it wants to be large or fast — the meta-schema will gain a `transport` hint in Phase 2 (`control` vs `binary`), and Phase 1 implicitly assumes `control`. Apps that need more throughput wait for the Phase 2 binary plane, which will share the channel declaration surface but use a different transport underneath.

The host enforces these constraints at runtime. A `pipe_emit` payload larger than 64 KB is dropped, logged, and flagged on the wire in the patchbay. A producer sustaining emit rates past a configurable threshold (default ~16 Hz per channel, roughly 1 KB/sec at average payload size) gets a yellow rate-limit glyph in the patchbay and a warning in the log. The host does not throttle the producer — it delivers every message — but the warning is loud enough that app authors notice before shipping.

### 4. State coordination

Pipes are for messages, not state. Two apps that both want to edit the same document can't coordinate via pipes without inventing a CRDT protocol on top. The pipe tells you "the selection moved" but it doesn't tell you who owns the document.

**Mitigation.** Phase 1 channels are strictly unidirectional per declaration — a channel is either an input or an output, never both. An app that needs request/response semantics (e.g., "ask the editor what's at line 12") declares two channels: an output `query` and an input `response`. The resolver wires them independently. This does not solve shared-state coordination, but it keeps the pipe abstraction honest: pipes are a data flow primitive, not a state primitive. State coordination is a separate problem for a separate feature.

Apps that want CRDT-like document sharing are not going to get it from typed pipes in v1 or v2. That is a different abstraction — probably a host-provided "shared document" API where ownership and conflict resolution are the host's problem, not the protocol's. Typed pipes are an explicit "ships message, consumer decides" primitive, and pushing state coordination through them would break the feature by turning every consumer into a replicated state machine.

### 5. Auto-wire surprises

The user drops an app into a group expecting one wire and gets three, because three consumers all match by kind. Or gets zero, because no name matches and no unambiguous kind-only fallback exists. "It just worked" cuts both ways — when it's wrong, it's wrong invisibly.

**Mitigation.** The patchbay overlay. Every wire is inspectable with one keystroke. The overlay shows both active and compatible-but-not-wired dots so users can see opportunities they're missing. Breaks are persistent per pane pair. There is no way to avoid all surprise while keeping zero-config composition, so the design bets on fast, obvious debugging over clever defaults.

### 6. Lifecycle complexity

Panes come and go, apps crash, hot reloads change manifest declarations. A naïve "wires are forever" implementation produces ghost wires that point at dead processes and messages that dispatch into the void.

**Mitigation.** The simple rules in Section 8: wires live with the group, die with the pane, re-form on move-in, silently drop on reload. The host's wire registry is rebuilt from scratch on every resolver run rather than patched incrementally. Rebuild cost is linear in the group size and in practice negligible — groups are small (single-digit panes) in every realistic workflow.

### 7. Debugging composition

A file browser sends a path, an editor doesn't open it, the user has no idea why. Is the wire formed? Is the payload malformed? Is the consumer ignoring it? Without tooling, this is unshippable.

**Mitigation.** The trace-channel inspector, shipped as part of Phase 1d. Click a wire in the patchbay, see every message that crossed it in the last N seconds with timestamp, size, full JSON preview, and any validator errors. This is the feature that makes the whole design credible — every Max/MSP-style system that became usable added a message monitor early, and every one that didn't stayed a toy.

### 8. Security gap

Pipes can be abused to exfiltrate data. A consumer can be tricked into acting on malicious payloads. The capability gate in Section 9 narrows what kinds an app touches but does not protect against a malicious producer that legitimately holds the capability.

**Mitigation.** The `[app.io.permissions]` block is partial coverage: it lets high-trust apps advertise tight hygiene and lets users narrow untrusted apps. Full isolation — process sandboxing, message-level auditing, capability-scoped payload inspection — depends on the broader sandbox work tracked in `app-infrastructure.md` Phase 7. Typed pipes do not make that work harder, and in some ways they make it easier (structured messages are more auditable than freeform bytes), but they also do not solve it.

### 9. Versioning fuzziness

Semver per kind helps but isn't airtight. A minor bump that adds an optional field is technically compatible, but a consumer written before the field existed will silently ignore data the producer expects it to use. "Compatible" is a blunt instrument.

**Mitigation.** The meta-schema validator refuses breaking changes without a major bump. The kind's `description` field is encouraged to note behavioral expectations, not just schema. The patchbay overlay shows the resolved version on each wire, so users can see when a consumer is running against an older kind definition than the producer. This does not solve the fundamental problem that behavioral compatibility is not syntactic compatibility; it makes the problem visible.

Long-term, the right answer is probably conformance tests per kind: a set of canned producer/consumer pairs that every new app targeting a kind runs through to confirm they behave the way the kind's description demands. That is a Phase 3+ project, well outside v1 scope, and is noted here only so we remember it exists.

---

## Phasing

| Phase | Scope | Effort |
|---|---|---|
| 1a | Manifest `[app.io]` parsing + registry validation. Extends the existing manifest loader; rejects unknown kinds with a clear error. | 2 days |
| 1b | `pipe_emit` / `pipe_received` protocol additions. Adds two message types on the stdin/stdout transport; extends the Rust enum, wires the JSON codec, updates test harness. | 2 days |
| 1c | Auto-wire algorithm + wire registry on the host. Runs the resolver on every group shape change; stores wires in a per-group table; dispatches `pipe_emit` to matched consumers. | 3 days |
| 1d | Patchbay overlay UI (read-only, with click-to-break/re-form). Includes the color key, hover preview, and trace-inspector slide-out. | 3 days |
| 1e | Trace channel debugger. Per-wire ring buffer of last 256 messages with timestamps and sizes; surfaced via the patchbay wire click. | 2 days |
| 1f | 6 core kind TOML files + `docs/types/` directory + meta-schema validator. The validator enforces the major/minor/patch rules on CI. | 2 days |
| 1g | SDK helpers in Python and Rust. `ctx.emit("selection", payload)` on the producer side, `on_pipe("selection", handler)` on the consumer side, auto-validated against the manifest. | 2 days |
| 1h | Documentation in `app-infrastructure.md` cross-references and a SKILL.md addition for the Plexi install/authoring skill so new app scaffolds get `[app.io]` stubs. | 1 day |
| **Total** | | ~17 working days (≈2.5 weeks) |

Order matters. 1a and 1b can go in parallel; 1c depends on both; 1d and 1e can go in parallel after 1c; 1f can go in parallel with everything; 1g waits on 1a/1b/1c; 1h waits on everything. No phase is a hard research step — every phase has a known implementation path.

A reasonable team structure is one engineer on 1a/1b/1c (the protocol and resolver backbone), a second engineer on 1d/1e (the patchbay and trace inspector UI), and the type registry (1f) plus SDK wrappers (1g) picked up by either as slack. 1h is a shared closing task.

One-engineer shipping is also viable, at roughly the same 2.5-week wall-clock budget, because the sequential constraints match the single-engineer working pattern: each phase leaves the codebase in a working state, and the patchbay UI can be stubbed with a text-only printout for the duration of 1c so the resolver can be end-to-end tested before the overlay lands.

### Follow-up work captured

These items came up during the design discussion and are intentionally out of Phase 1 scope, but each has a home in a future spec or phase:

- **Conformance tests per kind.** A canned producer/consumer pair per kind that any new app claiming to support the kind must pass. Phase 3+.
- **Named buses for cross-group wiring.** The v2 successor to auto-wire-within-group. A global "notifications" bus, a "selection" bus across every workspace. Planned for `named-buses.md`.
- **Draggable wire editor.** Full patchbay editing — drag from any output to any input, draw wires across apps that don't auto-match. Planned for v2 alongside named buses.
- **Binary plane.** Shared-memory or local-socket transport for audio, video, and large blobs. Planned for `typed-pipes-binary-plane.md`.
- **Plexi-types standalone repository.** Extracted when criteria in the registry section are met.
- **Per-kind conformance CLI.** `plexi types check <kind>` runs the conformance suite against every installed app that declares the kind.
- **Replay from disk.** Loading a recorded trace from a `.plexi/traces/` directory and replaying it into a group for reproducible debugging. Phase 3.
- **Time-travel inspector.** Scrub backwards through a trace buffer with the group's state synced to each historical message. Phase 3.
- **Typed pipe metrics.** Per-wire throughput and latency surfaced in the Permissions Manager as an observability tab. Phase 2.

None of these are blockers for Phase 1. Each is documented here so we remember the thinking that produced the Phase 1 scope boundary, and so the next spec author knows where the ideas live.

### What ships at v1

At the end of Phase 1, the user can:

- Declare `[app.io]` in a manifest and have the host validate it at app load.
- Drop two apps with matching channels into a linked pane group and have them auto-wire with zero configuration.
- Open the patchbay overlay with `Cmd+Shift+P`, see every wire in the group, and break or re-form wires with a click.
- Click any wire and see its recent message history with full payloads and any validator errors.
- Write a new app in Python or Rust using the SDK helpers, with channel names checked against the manifest at decoration time.
- Hot-reload an app and watch its wires preserve, drop, or re-form based on the new manifest.

What deliberately does not ship at v1:

- Cross-group wiring.
- Draggable new wires in the patchbay.
- A binary transport for audio/video/large blobs.
- A separate `plexi-types` repository.
- Bidirectional channels (request/response is two unidirectional channels).
- Conformance testing per kind.
- Automatic schema evolution for existing in-flight messages on a major kind bump.

Each of those has a known follow-up home — most in Phase 2, a few in Phase 3+.

### Rough risk log

| Risk | Likelihood | Phase 1 response |
|---|---|---|
| Resolver perf scales poorly beyond 20 panes | Low | Groups are small in practice; resolver is O(n·m) per rebuild |
| Validator is a bottleneck on high-rate emits | Medium | Rate warnings in the patchbay; tighter kinds in Phase 2 |
| Two apps shipping with clashing output names | Medium | Patchbay collision glyph; documented convention |
| JSON overhead on `core.selection` with large ranges | Low | 64 KB per-message cap; reuse `core.file_path` for large handoffs |
| Manifest migration burden for existing apps | None | `[app.io]` is strictly opt-in; existing apps unaffected |
| WASM path can't reuse stdin/stdout transport | Deferred | Channel declarations are transport-agnostic; new message bus plugs in cleanly |

---

## See also

- `docs/specs/app-infrastructure.md` — the v1 app protocol. Typed pipes extend this without modifying it: new manifest table, two new draw commands, no breaking changes.
- `docs/specs/typed-pipes-binary-plane.md` — Phase 2, deferred. Audio, video, and high-throughput streams. Shares the channel declaration surface but adds a separate transport and backpressure model.
- `docs/specs/wasm-pwa-deployment.md` — mobile and web deployment path. Typed pipes are transport-agnostic at the declaration layer, so the WASM path inherits them with a WASM-local message bus replacing stdin/stdout.
- `DEV_LOG.md` entry "typed pipes vision brainstorm" — original riff on the 13 use cases and the rationale for the three-tier type registry.
- Future: `docs/specs/named-buses.md` — Phase 2. Cross-group wiring via named global channels. The v2 successor to the auto-wire-within-group model.
- Future: `docs/types/` — the in-repo type registry. To be populated with the 6 core kind TOML files as a follow-up to this spec.

### Implementation calls to make explicit

These are not gaps in the design — each has a working answer above — but they are the points where the implementation will have to make a concrete call and deserve to be called out:

- **Wire storage layout.** The resolver output is a set of (producer pane, output channel) → list of (consumer pane, input channel). The implementation can store this as a flat vector rebuilt per resolver run, a per-pane index, or a hybrid. The flat vector is simpler and cheaper at Phase 1 scale; the per-pane index becomes worth it only once groups routinely exceed 20 panes. Start flat, revisit if profiling demands it.
- **Patchbay as built-in or external app.** The overlay could be a built-in in-process Rust component (fast, no IPC overhead) or an external app consuming its own meta-channel (dogfoods the system but adds a cycle). Phase 1 ships built-in; the external-app version is a Phase 2 exercise once the dogfood story matters.
- **Break override storage.** Breaks are stored per pane-pair for the life of the group. Persisting them to the workspace file adds the nicety of "my break survives a restart" at the cost of extra file churn. Phase 1 persists to workspace; simple and low-risk.
- **Type registry hot reload in production.** Dev-mode hot reload is clearly useful. Production hot reload of the type registry is more questionable — it only matters if users are authoring new kinds without restarting. Phase 1 defers production hot reload until a real use case appears.

These calls are made explicit here so the first implementer doesn't have to re-derive them from the design intent.

None of them are research problems. Each has a default answer above, and the defaults are picked to favor the smallest possible Phase 1 footprint. The spec's commitment is that no part of Phase 1 requires inventing new mechanisms — every piece is either extending the existing protocol, writing a resolver pass, or rendering a UI. If an implementer hits something that feels like a research problem, it almost certainly means the spec failed to cover a case and should be updated, not that the implementer should improvise.

The author of the first implementation PR should feel comfortable reading this spec once, writing the code without re-reading it, and only returning to the spec to confirm naming choices on wire messages and manifest fields. That is the bar for a v1 design doc in this codebase — if the implementer has to re-read sections to figure out what was meant, the spec is too vague and the PR should include a clarification edit against this file.
