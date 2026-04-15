# Plexi v2.3 — Spatial Canvas & Advanced Primitives

**Status:** Draft (speculative)  
**Depends on:** v2.2 (rich text, clip regions, input layering)

---

## TL;DR

v2.3 introduces the spatial canvas primitive (infinite zoomable canvas with node placement), a node graph component (for flow editors and pipeline UIs), video/media primitives (frame seek, waveform), and WASM/PWA as a deployment target alongside native. These are ambitious features — v2.3 is a design anchor, not a committed release.

---

## Scope

### Spatial canvas primitive
A first-class infinite canvas where apps place nodes at arbitrary coordinates and Plexi handles viewport culling, pan/zoom physics, and hit-testing. Inspired by the `proposals/spatial-canvas.md` draft.

Key design: apps describe a world-space node tree (`SpatialNode { id, x, y, w, h, content_fn }`); Plexi maps it to screen space and only calls `content_fn` for visible nodes. Apps receive click/hover events in world coordinates.

### Node graph primitive
`ctx.node_graph(id, nodes, edges, on_connect, on_move)` — renders a directed graph with drag-to-connect edges, node selection, and minimap. Built on top of the spatial canvas. Target use case: pyflow, pipeline editors, dependency viewers.

### Video / media primitives
- `ctx.video_frame(id, path, timestamp_ms)` — render a decoded video frame at a position. Plexi handles decode + cache.
- `ctx.waveform(id, path, start_ms, end_ms, x, y, w, h, color)` — render an audio waveform strip.
- `ctx.playhead(id, position_ms, duration_ms, on_seek, ...)` — timeline scrub bar.

These unblock the Parallax video editor as a Plexi app rather than a separate process.

### WASM / PWA deployment target
Compile Plexi to WASM with a `wasm32-unknown-unknown` target. Apps still speak the same JSON protocol; instead of subprocess IPC, they run in a Web Worker and postMessage the draw commands.

Implications:
- External apps become JS/TS or WASM modules instead of Python executables
- Python SDK stays for native; JS SDK needed for web
- No PTY / shell integration in PWA mode
- Useful for demos, public-facing tools, and Plexi Teams (shared canvases)

---

## Why These Are v2.3 and Not Earlier

- **Spatial canvas** requires a scene graph abstraction that doesn't exist yet. Adding it on top of the flat draw-command model is a significant protocol extension.
- **Node graph** depends on spatial canvas.
- **Video primitives** require Plexi to own a decode pipeline (likely ffmpeg via subprocess or a Rust crate). Non-trivial dependency; isolated to a feature flag.
- **WASM target** requires rethinking the subprocess model entirely. The protocol is designed to survive this (JSON over any transport), but the egui renderer needs a web backend (egui_web or iced).

None of these block the v2.0–v2.2 feature set. They are deferred until the core is stable and at least one demanding app (Parallax, pyflow) validates the need.

---

## §1 — Spatial Canvas Protocol

```json
{"type": "canvas_begin", "world_x": -5000, "world_y": -5000, "world_w": 10000, "world_h": 10000}
{"type": "canvas_node", "id": "node-1", "x": 100, "y": 200, "w": 160, "h": 80}
// draw commands for this node (relative to node origin)
{"type": "canvas_node_end"}
{"type": "canvas_end"}
```

Host culls nodes outside the viewport (zoom + pan applied via the existing transform stack). Apps receive `CanvasClick { node_id, local_x, local_y }` and `CanvasDrag { node_id, dx, dy }` events.

---

## §2 — Node Graph Protocol

```json
{"type": "node_graph", "id": "pipeline", "nodes": [...], "edges": [...], "selected": ["node-2"]}
```

Node: `{id, x, y, w, h, title, inputs: [{id, label}], outputs: [{id, label}]}`  
Edge: `{from_node, from_port, to_node, to_port}`

Host renders all node chrome (ports, labels, selection highlight). Apps provide per-node content via a `render_node` callback.

---

## §3 — Video Primitives

```python
ctx.video_frame("clip-1", path="/tmp/clip.mp4", timestamp_ms=1250, x=0, y=0, w=320, h=180)
ctx.waveform("audio-1", path="/tmp/audio.wav", start_ms=0, end_ms=10000,
             x=0, y=200, w=800, h=40, color="#89b4fa")
ctx.playhead("timeline", position_ms=1250, duration_ms=10000,
             x=0, y=250, w=800, h=20, on_seek=lambda ms: state.update({"pos": ms}))
```

Protocol: new `DrawCommand` variants `VideoFrame`, `Waveform`, `Playhead`. Host manages decode threads and frame caches.

---

## §4 — WASM / PWA Deployment

Target: `plexi-web` — a static site that loads a Plexi runtime in WASM and runs apps as Web Workers.

App protocol is identical; transport changes from pipe to postMessage. Python apps don't run in this mode; JS/TS SDK needed.

Feature flag: `cargo build --features wasm-target`. Native and web targets share all protocol types; only transport and renderer differ.

---

## Ship Order (tentative)

1. Spatial canvas protocol design + reference implementation
2. Node graph built on spatial canvas
3. Video decode pipeline (feature-flagged)
4. Waveform + playhead primitives
5. WASM renderer investigation + prototype
6. JS/TS SDK (mirrors Python SDK API)
7. PWA packaging + deploy

---

## Cross-references

- v2.2 spec: `docs/specs/releases/plexi-v2.2.md`
- Spatial canvas proposal: `docs/specs/proposals/spatial-canvas.md`
- WASM/PWA proposal: `docs/specs/proposals/wasm-pwa-deployment.md`
- Media primitives proposal: `docs/specs/proposals/media-primitives.md`
