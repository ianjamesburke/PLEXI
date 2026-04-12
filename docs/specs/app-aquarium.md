# Aquarium Simulator

**Status:** Spec  
**Last updated:** 2026-04-11  
**Depends on:** Advanced UI SDK (animation, delta_time, bezier/circle draw commands)  
**App type:** Out-of-process (Python)

---

## Summary

An ambient aquarium with procedurally animated fish, plants, and particles. Zero permissions. Proves the Advanced SDK handles visually rich, continuously animated content at smooth frame rates over the JSON draw protocol.

This is a stress test and a showcase. If an aquarium with 20+ independently animated entities renders smoothly, the protocol can handle any visual app.

---

## Why This App

Exercises features Snake doesn't:
- **Continuous animation** — every frame is different (not tick-based)
- **Many independent moving entities** — tests draw command throughput
- **Floating-point coordinate movement** — smooth motion, not grid-snapped
- **Bezier curves** — fish body shapes, plant sway
- **Circles** — bubbles, eyes
- **Opacity** — depth layering, bubble transparency
- **No input** (ambient mode) — proves an app can be purely visual

---

## Scene

### Fish

Each fish is a procedural shape drawn from primitives:

- **Body**: filled ellipse (approximated with a wide rect + high radius, or a custom bezier outline)
- **Tail**: triangle (two lines + fill) that oscillates with a sine wave
- **Eye**: small white circle + smaller black circle
- **Fin**: small triangle, oscillates opposite to tail

Fish properties (randomized per fish):
- **Size**: 20–60 px body length
- **Color**: random hue, saturation 60–80%, lightness 50–70%
- **Speed**: 20–80 px/sec (smaller fish faster)
- **Depth**: 0.0–1.0 (affects opacity and speed — background fish are dimmer and slower)
- **Direction**: left or right (flips horizontally)
- **Swim pattern**: gentle sine wave on Y axis (amplitude 5–15px, period 2–4sec)

Fish swim horizontally. When they exit one side of the screen, they re-enter from the opposite side (with a new random Y position and depth).

### Plants

Bottom-anchored, swaying seaweed:

- 3–6 plants at random X positions along the bottom.
- Each plant is a series of connected bezier curves from bottom to top.
- Sway is driven by a sine wave with per-plant phase offset (so they don't move in unison).
- Color: greens (`#a6e3a1`, `#94e2d5`) with slight variation per segment.

### Bubbles

Occasional bubbles rise from plants or fish:

- Spawn at random intervals from plant tips or fish mouths.
- Rise with slight horizontal drift (sine wave on X).
- Grow slightly as they rise (pressure decrease visual).
- Fade out (opacity → 0) near the top.
- Circle draw command, stroke only, white with 30–60% opacity.

### Background

- **Water gradient**: dark blue at bottom (`#1e1e3e`) to slightly lighter at top (`#2e2e4e`). Rendered as a series of horizontal rects with interpolated colors.
- **Sand**: a rect at the very bottom, warm color (`#45475a` or `#585b70`), with a few scattered circle "pebbles".
- **Light rays**: 2–3 diagonal lines from top, very low opacity (`#ffffff10`), slowly drifting. Creates underwater caustic feel.

---

## Animation System

All animation is driven by `ctx.time` (seconds since app start) and `ctx.delta_time`:

```python
def on_render(self, ctx):
    dt = ctx.delta_time

    # Update fish positions
    for fish in self.fish:
        fish.x += fish.speed * fish.direction * dt
        fish.y = fish.base_y + math.sin(ctx.time * fish.freq + fish.phase) * fish.amplitude

        # Wrap around screen
        if fish.direction > 0 and fish.x > ctx.width + fish.size:
            fish.x = -fish.size
            fish.base_y = random.uniform(50, ctx.height - 100)
        elif fish.direction < 0 and fish.x < -fish.size:
            fish.x = ctx.width + fish.size
            fish.base_y = random.uniform(50, ctx.height - 100)

    # Update bubbles
    for bubble in self.bubbles:
        bubble.y -= bubble.speed * dt
        bubble.opacity -= 0.3 * dt
    self.bubbles = [b for b in self.bubbles if b.opacity > 0 and b.y > 0]

    # Draw everything
    self.draw(ctx)
```

### Performance Budget

Target: smooth 60fps rendering. This means the app must emit all draw commands for a frame within ~16ms.

Estimated draw commands per frame:
- Background: ~10 rects (gradient) + 3 lines (light rays) = ~13
- Sand + pebbles: 1 rect + 5 circles = 6
- Plants (5 plants × 4 segments): 20 bezier curves
- Fish (15 fish × 5 primitives): 75 commands
- Bubbles (10 active): 10 circles
- **Total: ~124 draw commands per frame**

This is a good stress test. If the JSON serialization + stdin/stdout pipe + egui rendering pipeline handles 124 commands at 60fps, we're in excellent shape.

---

## Interaction (Optional)

The base version is fully ambient — no interaction. But optional interactions could be added:

| Action | Effect |
|--------|--------|
| Click anywhere | Fish scatter away from click point, then gradually return |
| Tap food key (`f`) | Drop a food particle; nearest fish swims toward it |
| Cmd+scroll | Zoom in/out on the scene |

These are fun additions but not MVP. The value is in the ambient visual.

---

## Manifest

```toml
[app]
id = "aquarium"
name = "Aquarium"
version = "0.1.0"
description = "Ambient underwater scene with procedural fish and plants"

[capabilities]
# No capabilities needed — pure rendering
```

---

## File Structure

```
~/.plexi/apps/aquarium/
  manifest.toml
  aquarium.py
  plexi_sdk.py
  plexi_sdk_advanced.py
```

Single file app. Target: under 400 lines.

---

## MVP

1. Water gradient background + sand
2. 10–15 fish with independent swimming patterns, wrapping at screen edges
3. 3–5 swaying plants
4. Bubbles rising from plants
5. Smooth animation via delta_time

**Defer:** Light rays, fish scatter on click, food dropping, depth-of-field blur, pebbles.
