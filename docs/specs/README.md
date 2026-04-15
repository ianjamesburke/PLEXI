# Plexi Specs — Index

Single source of truth for where every spec lives. Start here for any spec question.

---

## Releases

| File | Version | Status | Summary |
|------|---------|--------|---------|
| [`releases/plexi-v2.1.md`](releases/plexi-v2.1.md) | v2.1 | Implemented | UI primitives: PushTransform, MeasureText, viewport/text_input/tabs/grid/modal, feature negotiation |
| [`releases/plexi-v2.2.md`](releases/plexi-v2.2.md) | v2.2 | Draft | Rich text, clip regions, multiline input, IME, input layering, PyPI SDK |
| [`releases/plexi-v2.3.md`](releases/plexi-v2.3.md) | v2.3 | Draft (speculative) | Spatial canvas, node graph, video primitives, WASM/PWA target |

---

## Subsystems

| File | Scope |
|------|-------|
| [`app-infrastructure.md`](app-infrastructure.md) | App registry, manifest format, launch lifecycle, pipe wires |

---

## Proposals

Proposals live under `proposals/` and are promoted to release specs when scoped and accepted.

| File | Topic | Target |
|------|-------|--------|
| `proposals/input-layering.md` | Key dispatch priority tiers | v2.2 §7.5 |
| `proposals/spatial-canvas.md` | Infinite zoomable canvas | v2.3 §1 |
| `proposals/wasm-pwa-deployment.md` | WASM/PWA build target | v2.3 §4 |
| `proposals/media-primitives.md` | Video frame, waveform, playhead | v2.3 §3 |

---

## Feature → Spec mapping

| Feature | Spec |
|---------|------|
| `core_v1` | v2.0 |
| `open_intent_v1` | v2.0 |
| `event_bus_v1` | v2.0 |
| `runs_v1` | v2.0 |
| `typed_pipes_v1` | v2.0 |
| `ui_primitives_v1` | v2.1 |
