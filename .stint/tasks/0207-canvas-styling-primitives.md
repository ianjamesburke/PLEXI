---
id: "0207"
title: "Canvas styling primitives the SDK should have offered"
status: done
estimate: "2h"
actual: "335m"
started_at: "2026-06-17T01:21:25Z"
completed_at: "2026-06-17T06:56:22Z"
blocked_by: []
gh_issue: []
area:
  - "sdk/python"
  - "ui/widgets"
tags:
  - "v1"
  - "sdk"
---



Add the canvas drawing primitives that app authors (and their agents) reach for but the SDK does not yet provide. Seeded from the Stats app redesign: building real visual polish (glows, bordered cards, gradient bars, RPG flair) required hacking around missing primitives with stacked alpha rects and four-line borders. The exact final set is determined by a design experiment (4 independent agents each built the Stats UI against the current SDK and reported what they hacked around); the recurring wishes across those agents are the real gaps and get folded into Scope below before implementation.

Each new primitive spans the full draw-command path: `sdk/python/plexi_sdk/_render_context.py` (Python API) → `src/protocol/commands.rs` (DrawCommand variant, serde defaults so old apps keep working) → `src/render/headless_renderer.rs` (test/scene renderer) → the live wgpu painter. No backward-compat shims: extend the existing `rect`/draw commands with optional fields, don't fork new command types unless a primitive is genuinely new geometry.

## Scope

Confirmed set from the 4-agent experiment (each primitive reached for by ≥2 of 4 independent agents), ranked by recurrence:

- **Glow / soft shadow on `rect` and `circle` (4/4 agents):** add `glow_color: str | None = None` and `glow_radius: float = 0.0` to `ctx.rect()` and `ctx.circle()`. Replaces the universal hack of stacking 3-5 expanding low-alpha rects/circles to fake a halo. Highest impact — every agent built a glow helper.
- **Linear gradient fill on `rect` (4/4 agents):** a gradient fill, e.g. `ctx.rect(..., gradient={"from": "#aabbccff", "to": "#ddeeffff", "dir": "h"|"v"})` (final shape TBD during impl — keep it one call, support `#rrggbbaa` stops). Replaces faking gradients with a single translucent overlay rect. Used for XP bars, hero band, quest bars.
- **Stroke / border on `rect` (+ `circle`) (3/4 agents):** add `stroke: str | None = None`, `stroke_width: float = 1.0`. Removes the four-line / fill+inset-fill border hack for cards and tiles.
- **`arc_ring` (2/4 agents):** stroked ring / donut arc, e.g. `ctx.arc_ring(cx, cy, r, start_angle, end_angle, color, stroke_width)` — distinct from the existing filled-pie `arc`. Used for radial XP / level rings.

Each primitive: Python API in `_render_context.py` + protocol variant (serde-default so old apps keep working) + headless renderer + wgpu painter, with a `PlexiUiHarness`/scene smoke test proving it paints.

- **Canvas API documentation + discoverability (rolled in):** the current canvas drawing path is under-documented, which is why agents reverse-engineer `_render_context.py` to build visual apps. A primitive no agent can discover is useless, so this ships with the primitives:
  - Replace the "Canvas escape hatch" stub in `docs/sdk-v2.md` with a complete canvas drawing reference: `rect` (full signature incl. radius, `#rrggbbaa` alpha, and the new `stroke`/`glow`/gradient args), `circle` (+ stroke/glow), `arc`, `arc_ring`, `text` (incl. `align`/`max_width`/`monospace`/`bold`), `line`, and the `ctx.theme.*` token list + `dim()`.
  - Point the authoring entry points at it: scaffold header (`app_init.py`) and `SDK_QUICKSTART.md` should reference the canvas reference, not just `plexi_sdk/ui.py`.
  - Add the one-line "read the SDK canvas reference before building a canvas/visual app" note to the `plexi-cli` skill's app section (the "verify docs before building" step the authoring flow is currently missing).

## Non-Scope

- The Stats app redesign itself (built separately; this task only delivers the primitives it surfaces).
- New layout components in `ui.py` (this is canvas-level drawing primitives, not declarative widgets).
- One-off wishes from a single agent: text outline/`stroke_color`, `letter_spacing`, per-corner radius, clip regions, round line-caps, `divider`/`tick_row` convenience helpers, vignette. Revisit only if a second use case appears.
- Right-aligned-text helper: not a gap — `ctx.text` already supports `align` + `max_width`. Fix via docs/examples, not a new primitive.

## Why

The SDK should be shaped by what an agent building with it naturally reaches for. Faking glows and borders with stacked alpha rects is the recurring tell that a primitive is missing; closing those gaps makes every app's visuals cleaner and the authoring path obvious.

## References

- `sdk/python/plexi_sdk/_render_context.py` — canvas drawing API (`rect`, `circle`, `arc`, `text`, `line`)
- `src/protocol/commands.rs` — `Rect` and sibling DrawCommand variants
- `src/render/headless_renderer.rs` — scene/test renderer that must paint each new primitive
- `apps/stats/stats.py` — the redesign that surfaced these gaps
- `/tmp/stats-mockups/agent-{1..4}/stats.py` — experiment mockups + per-agent SDK wishlists (synthesis source for the pending Scope items)
