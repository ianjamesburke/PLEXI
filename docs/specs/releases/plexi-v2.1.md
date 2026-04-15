# Plexi v2.1 — UI Primitives

**Status:** Implemented  
**Depends on:** v2.0 (orchestration layer, OpenIntent, Runs, event bus, typed pipes Phase 1)

---

## TL;DR

v2.1 adds the drawing primitives apps need to build real UIs: a transform system for pan+zoom viewports, exact font measurement, and five high-level components (viewport, text_input, tabs, grid, modal). It also formalizes the feature negotiation contract so apps can declare what they need and the host can refuse gracefully.

---

## Scope

**In v2.1:**
- `PushTransform` / `PopTransform` draw commands with translate + scale support
- `MeasureText` / `TextMetrics` round-trip for exact font measurement
- `HOST_FEATURES` const + `[app.protocol] requires` manifest section for feature negotiation
- Python SDK: `ctx.viewport`, `ctx.text_input`, `ctx.tabs`, `ctx.grid`, `ctx.modal`
- Reference app: `mermaid-viewer` using `ctx.viewport` + `ctx.modal` + `ctx.tabs`

**Out of scope (deferred to v2.2):**
- Multi-line text editor primitive
- IME composition support
- Rich text runs (syntax highlighting)
- Clip regions (`ClipRect` / `ResetClip`)
- Input layering formalization
- `plexi-sdk` on PyPI

---

## Why These Primitives

The v2.0 drawing model (Rect, Text, Line, List) is sufficient for simple data-display apps but breaks down for anything with spatial content. A diagram viewer, a canvas editor, a map — all need a coordinate transform. Adding PushTransform/PopTransform as first-class protocol commands means apps don't need to implement their own matrix math for the common cases (zoom, pan, simple layouts).

MeasureText closes the layout loop: apps can't position elements accurately without knowing how wide text will be. The blocking round-trip is acceptable because it's cached per-frame and rare in steady-state rendering.

The five SDK components (viewport, text_input, tabs, grid, modal) aren't new protocol concepts — they're composition patterns built on existing draw commands. Shipping them in the SDK establishes conventions that all apps can reuse, rather than every app reimplementing tabs differently.

---

## §1 — Protocol: PushTransform / PopTransform

```json
{"type": "push_transform", "scale_x": 2.0, "scale_y": 2.0, "translate_x": 100, "translate_y": 50}
{"type": "pop_transform"}
```

**Transform semantics:**
- Transforms compose multiplicatively (push onto a stack).
- On `FrameDone`, the stack is reset.
- `rotate` is accepted but logged as a warning and skipped in v2.1 — reserved for v2.2.
- `origin_x` / `origin_y` are accepted but unused in v2.1.

**Host behavior:** Apply the current stack product to all coordinate values in subsequent draw commands until the matching `PopTransform`.

---

## §2 — Protocol: MeasureText / TextMetrics

App sends:
```json
{"type": "measure_text", "request_id": 42, "text": "Hello", "size": 14.0, "monospace": false, "bold": false}
```

Host responds:
```json
{"type": "text_metrics", "request_id": 42, "width": 31.4, "height": 17.0, "ascent": 13.6}
```

The app must handle this as a synchronous blocking call — flush the current command buffer, send the request, then read stdin until the matching `request_id` reply arrives. See §5 for SDK implementation.

---

## §3 — Feature Negotiation

Apps declare required host features in `manifest.toml`:

```toml
[app.protocol]
requires = ["ui_primitives_v1"]
```

The host checks this at launch against `HOST_FEATURES`:
```rust
pub const HOST_FEATURES: &[&str] = &[
    "core_v1", "open_intent_v1", "event_bus_v1",
    "runs_v1", "typed_pipes_v1", "ui_primitives_v1",
];
```

If any required feature is missing, the host refuses to launch the app and logs a clear error. Apps that don't declare `[app.protocol]` are always launched (backward compatible).

**`ui_primitives_v1` covers:** PushTransform, PopTransform, MeasureText, TextMetrics.

---

## §4 — Rust Host Changes

- `src/app_protocol.rs`: Added `PushTransform`, `PopTransform`, `MeasureText` to `DrawCommand`; added `TextMetrics` to `PlexiEvent`.
- `src/process_app.rs`: Added inline transform stack to `render_draw_commands`; MeasureText resolved to TextMetrics event via egui font context.
- `src/app_registry.rs`: Added `AppProtocolSection`, `HOST_FEATURES` const, launch-time feature check.

---

## §5 — Python SDK Changes (0.5.0)

New dataclass: `TextMetrics(width, height, ascent)`

New `RenderContext` methods:
- `measure_text_exact(text, size, monospace, bold) → TextMetrics` — blocking, cached per-frame
- `viewport(viewport_id, content_fn, zoom, pan, ...)` — push/pop transform around content_fn
- `text_input(input_id, value, on_change, ...)` — single-line input with blinking cursor
- `tabs(tab_id, tabs, selected, ...)` — tab bar with underline indicator
- `grid(grid_id, cols, rows, render_cell, ...)` — uniform cell grid
- `modal(modal_id, visible, content_fn, ...)` — centered modal with backdrop

---

## §6 — Reference App: mermaid-viewer

Located at `examples/mermaid-viewer/`. Demonstrates:
- `ctx.viewport` for pan+zoom diagram navigation
- `ctx.tabs` for diagram/source switching
- `ctx.modal` for help overlay and error display
- Keyboard-driven zoom/pan (arrow keys, +/-, r to reset)
- `[app.protocol] requires = ["ui_primitives_v1"]` manifest declaration

---

## §7 — Ship Order

1. Protocol types (`app_protocol.rs`) — no behavior, just type definitions
2. Host implementation (`process_app.rs`, `app_registry.rs`)
3. SDK additions (`plexi_sdk.py`)
4. Reference app (`mermaid-viewer`)
5. Spec files (`plexi-v2.2.md`, `plexi-v2.3.md`)

---

## Cross-references

- v2.0 spec: `docs/specs/releases/plexi-v2.0.md`
- v2.2 spec (deferred): `docs/specs/releases/plexi-v2.2.md`
- App infrastructure: `docs/specs/app-infrastructure.md`
