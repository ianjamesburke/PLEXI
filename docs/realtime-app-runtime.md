# Real-time app runtime

Status: active

Stint: 0553

Plexi should present interactive apps at a stable display cadence under normal host load. The target is not an average of 60 frames per second. The target is predictable frame delivery, immediate input, and bounded recovery when either the app or the host misses a deadline.

[`wasm-runtime.md`](wasm-runtime.md) describes the runtime Plexi ships today. This PRM defines the destination for interactive animation, instruments, and small games across the Python and native WASM paths.

## Call

Keep Python as the fastest authoring and compatibility path. Use native WASM surfaces for apps that need the smallest rendering overhead. Move frame timing and presentation ownership into one host contract shared by both.

The runtime should not make every app pretend to be a game. Declarative UI remains event-driven. Real-time apps get a fixed-step simulation clock and a latest-frame presentation path. Audio stays on its real-time callback and never waits for UI paint.

## What the Balls work tells us

Balls can hold its declared cadence in steady state. The current measurements and the meanings of `paint_fps`, `guest_fps`, `avg_host_ms`, and `avg_roundtrip_ms` live in the Performance section of [`wasm-runtime.md`](wasm-runtime.md#performance).

That result clears the basic host renderer. Painting the decoded canvas is cheap. It does not prove that the full system has consistent frame pacing:

- `avg_roundtrip_ms` includes guest work, transport, decode, and time spent waiting for a host paint. It is not a pure measure of Python compute.
- Average FPS hides missed deadlines. A run can average 60 FPS and still show visible clusters of short and long frame intervals.
- The Python pane admits work, receives completed frames, presents trees, and schedules its next repaint from the host UI path. A synchronous host operation can delay all of those at once.
- Stint 0548 records the observed host-wide stalls and owns moving long host operations off the UI thread. That work comes before a runtime rewrite.

PR #2489 tests useful local reductions: fewer redundant repaint wakes, a smaller canvas payload, and a choice between full-tree and delta encodings. Those changes can reduce steady-state work. They do not change the presentation model. The wake suppression can also let a completed continuous frame wait for the already scheduled paint, which trades a full-host repaint for up to one presentation interval of latency.

Treat those changes as tactical. Keep the parts that are measured, contained, and readable. Do not make their exact heuristics part of the future runtime contract.

## Current boundaries

### Python

The host runs the CPython WASI Preview 1 module in Wasmtime and starts `plexi_sdk._v3_process`. The SDK writes newline-delimited JSON through the module's stdout. A decoder thread converts the JSON into the WIT UI tree and canvas command types that the shared host renderer understands.

For a continuous canvas app, the frame crosses these boundaries:

```text
Python simulation
    -> SDK UI tree and canvas commands
    -> JSON encode and stdout
    -> Rust JSON and tree decode
    -> host UI tree renderer
    -> egui paint
```

Tree deltas, compact JSON, and coordinate quantization reduce the cost of this path. They do not remove the per-frame serialization boundary.

`PythonFrameScheduler` owns the Python cadence. Python telemetry reports averages for guest cadence, round trip, decode, render, and payload. It does not yet report the presentation interval distribution needed to explain isolated stutters.

### Native WASM

Native WASM components use the typed lifecycle in [`wit/plexi.wit`](../wit/plexi.wit): `init`, `update`, and `view`. `FrameClock` and `FrameTelemetry` pace and measure native surface panes. A GPU `surface` node lets the guest update a host-owned texture without sending a canvas command list through JSON.

The native pane currently derives a repeating cadence from timer behavior. Cadence should become an explicit app declaration instead of an inference from a timer.

### The duplication

Python and native WASM have separate schedulers, admission rules, wake behavior, and telemetry. Both eventually present through the same host frame. Fixes can land in one path without reaching the other, and the two sets of metrics cannot answer the same questions.

## Destination

```text
app input
    -> fixed-step simulation
    -> latest completed frame
    -> display-paced host compositor
    -> screen
```

The simulation clock and presentation clock are separate.

The app advances simulation in fixed steps. Bounded catch-up handles a late producer without letting a backlog grow forever. The producer publishes its latest complete frame into a latest-wins mailbox. The host compositor presents at the display cadence and repeats the previous frame if the next one is not ready. Old unfinished presentation work is dropped.

Input enters the simulation queue immediately. It does not wait for the next render admission deadline. Telemetry records when the host received the input, when the guest consumed it, when frame production finished, and when the frame was presented.

This gives Plexi one frame contract:

- The host owns presentation timing.
- The app owns simulation state and declares its target cadence.
- At most one frame waits to be presented.
- Slow producers drop visual frames instead of building latency.
- A late host operation appears as presentation delay, separate from guest production time.
- Python and native WASM report the same timing stages.

## Presentation paths

| App work | Transport | Paint policy |
|---|---|---|
| Forms, lists, editors, dashboards | Typed declarative UI tree | Event-driven |
| Animated canvas and simple interactive scenes | Typed or binary display list in a latest-wins frame slot | Display-paced |
| High-object-count games, shaders, video, custom rendering | Host-owned GPU surface | Display-paced |
| Audio and MIDI processing | Typed real-time pipes and the audio callback | Never coupled to UI paint |

The declarative tree remains the default. It is the right primitive for most Plexi apps and preserves host-owned layout, accessibility, input, and theming.

The real-time display-list path replaces newline JSON as the hot frame transport. It can preserve the Canvas authoring API while encoding commands into a typed binary buffer. The host decodes or maps only the newest completed buffer.

The GPU surface path is the performance ceiling. An air hockey game or a dense particle instrument should use a native WASM surface when the display list becomes the limit. The Pong WASM proof of concept is the reference for this path.

## Performance contract

The release gate measures frame intervals, not visual impressions or average FPS alone.

Each real-time pane reports:

- target presentation interval
- produced and presented frames
- missed presentation deadlines
- dropped producer frames and dropped catch-up steps
- median, p95, and p99 production time
- median, p95, and p99 presentation interval
- input-to-update and input-to-present latency
- time attributed to guest work, transport, decode, queue wait, host update, and paint

The reference test runs Balls while the host performs normal pane and context work. A clean idle run is necessary but insufficient. The gate must also exercise context changes, workspace persistence, pane churn, input bursts, and another animated pane.

The first target is a stable 60 Hz presentation on a decent computer with no sustained backlog. A missed producer deadline should repeat one frame and recover on the next presentation slot. It should not cause a burst of catch-up paints.

## Migration order

First, complete stint 0548 and establish the host's worst presentation stalls under the same workload. A frame architecture cannot compensate for seconds of synchronous UI-thread work.

Next, add shared interval telemetry to both runtimes. Record production, queue, and presentation timestamps before changing scheduling. This creates the falsifiable baseline for every later decision.

Then move both paths behind one host-owned presentation clock and latest-frame mailbox. Keep the existing Python JSON bridge as the first producer so the scheduler change can be measured without changing transport at the same time.

After the clock is shared, add the typed display-list transport for continuous Canvas apps. Ordinary UI trees stay on the current event-driven path. Native GPU surfaces continue to bypass the display list.

Finally, test a direct Python component route against the real SDK and maintained apps. `componentize-py` is the first upstream route to evaluate because it maps Python to the component model. Adoption depends on package compatibility, startup cost, artifact size, and whether it removes the duplicate JSON decoder without rebuilding the current bridge inside a component. CPython compatibility can remain a separate lane if the direct component route cannot support the maintained app set.

## Multiplayer boundary

Multiplayer is not a rendering feature. It needs a deterministic simulation state, ordered inputs, reconciliation, and a network protocol. The fixed-step simulation clock is the prerequisite because it gives local and remote peers a shared unit of progress.

The renderer consumes snapshots. It does not own rollback, prediction, or network state. Those belong to a later game-session contract built on the event bus for structured control data and typed pipes where high-rate binary data is justified.

## Non-goals

- Replacing Python because it appears slower in isolation
- Turning the declarative UI toolkit into a general game engine
- Making every app repaint continuously
- Coupling audio timing to the host compositor
- Building multiplayer, physics, or scene authoring in this runtime task
- Preserving the internal heuristics from PR #2489 as public API

## Open questions

- Which eframe or platform signal should drive the display-paced compositor when the display refresh rate is not 60 Hz?
- Should the Canvas display list use a WIT resource, shared memory, or a typed pipe buffer?
- Can a direct Python component support the maintained Python app set without shipping a second compatibility runtime inside it?
- Does the host need one compositor deadline for all real-time panes or per-display deadlines for windows on different monitors?

## References

- [`wasm-runtime.md`](wasm-runtime.md) for the shipped runtime and current performance measurements
- [`wit/plexi.wit`](../wit/plexi.wit) for the typed lifecycle and surface contracts
- `src/host/wasm_python.rs` for the CPython bridge, decoder, scheduler, and Python telemetry
- `src/host/wasm_frame.rs` for the native frame clock and telemetry
- `src/host/wasm_pane.rs` for native lifecycle and surface presentation
- `apps/wasm-poc/pong` for direct GPU surface rendering
- [Bytecode Alliance componentize-py](https://github.com/bytecodealliance/componentize-py) for the direct Python component experiment
