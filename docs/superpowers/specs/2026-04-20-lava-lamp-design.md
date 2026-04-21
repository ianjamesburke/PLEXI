# Lava Lamp — Plexi App Design

**Date:** 2026-04-20
**Type:** Example app
**Location:** `examples/lava-lamp/`

## Goal

A visually satisfying lava lamp simulation as a Plexi v3 example app. Demonstrates soft-body-feeling fluid motion using only `DrawCommand::Circle`, with fake-metaball blending so blobs appear to merge.

## Scope

- 6–8 colored blobs rising and falling in a tall pane driven by buoyancy + temperature physics.
- Fake metaballs: nearby blobs render connective translucent halos so they visually merge.
- Click to inject heat at a point (perturbs the nearest blob upward).
- No lamp silhouette, no glass chrome, no lighting passes. Dark gradient background only.

Out of scope: true per-pixel metaballs, pixel buffer primitives, audio, persistence.

## Architecture

Single Python file `lava_lamp.py` following the `examples/balls/balls.py` pattern:

- `LavaLampApp(App)` with `on_init`, `on_render`, `on_click`.
- `Blob` class with `x, y, vx, vy, r, temperature, color`.
- Physics step: temperature-driven buoyancy (hot rises, cool sinks), lateral drag, soft wall repulsion, viscous damping between touching blobs (no hard collisions — they should glide through each other).
- Temperature model: each blob trends toward ambient based on vertical position (warm near bottom, cool near top), with a small random wobble.
- 60 fps via `ctx.emit.schedule_render(16)`.

## Rendering (fake metaballs)

Per frame:

1. `ctx.clear("#0a0a1a")` — deep navy background.
2. For each blob:
   - 3 stacked translucent circles at increasing radius and decreasing alpha (halo/glow).
   - One solid core circle at base radius.
   - Small specular highlight (same trick as balls.py).
3. For each pair of blobs within `merge_distance = (r_a + r_b) * 1.4`:
   - Draw 4–6 translucent circles along the line between their centers, radius interpolated between the two, alpha scaled by proximity. This produces the visual "bridge" that reads as a merge.

Color palette: warm lava tones (reds, oranges, magentas) on dark blue background.

## Controls

- **Click:** find nearest blob to click point, boost its temperature and give it an upward impulse.

## Manifest

```toml
[app]
id = "lava-lamp"
name = "Lava Lamp"
version = "0.1.0"
description = "Fluid blob simulation with fake-metaball blending — demonstrates DrawCommand::Circle layering."
entry = "lava_lamp.py"

[app.capabilities]
capabilities = []

[launch]
layout_hint = { side = "right", split = 0.35 }
```

Narrow pane matches the tall form factor of a real lava lamp.

## Testing

- Manual: `just install-v3`, launch the app, verify blobs rise/fall smoothly and click injects heat.
- Smoke: `scripts/smoke-test.sh` covers PGAP Init + ready within 3s and no-panic. No new test infra.

## Success criteria

- Runs at 60 fps on a modern Mac without dropping frames.
- Blobs visibly merge and separate — not just circles passing through each other.
- No panics on install or first frame.
