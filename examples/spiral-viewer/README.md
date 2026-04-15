# Spiral Viewer

A development-time Plexi app that renders another Plexi app at 8 sizes
simultaneously, arranged on a Fibonacci (logarithmic) spiral. Purpose:
visually verify how a target app's UI degrades as its pane shrinks, so you
can identify the exact width and height where it starts to break.

This is the visual half of the agent-builds-apps loop. Pair it with the SDK
breakpoint primitives (`min_width`, `min_height`) to discover where your
layout needs defensive code.

## How it works

1. Spiral Viewer takes a target app via `argv[1]` — either a full path to a
   `manifest.toml`, or a bare app id like `snake` that it will look up in
   `~/.plexi-alpha/apps/`, `~/.plexi-beta/apps/`, or `~/.plexi/apps/` (and
   the local `examples/` tree).
2. It reads the target's manifest to learn its `min_width` / `min_height`.
3. It computes 8 positions on a logarithmic spiral around the center of its
   own pane. Smaller sizes sit near the center (worst-case breakpoints),
   larger sizes sit on the outside.
4. For each position, it spawns a fresh subprocess of the target, sends
   `init` + `render`, captures the draw command stream, then kills the
   subprocess. No orphaned processes.
5. For each captured frame, it translates and scales every `rect`, `text`,
   `line`, and `image` command into its own canvas at the instance's spiral
   position. Complex primitives (`list`, `file_grid`, `video_thumbnail`) are
   replaced with a small "N unrenderable" label in v1.
6. A thin border and a `WxH` label surround each instance so you can see
   the exact dimensions of every rendering.
7. Every 5 seconds — or on `r` — the full spawn cycle re-runs, picking up
   any source changes you've made to the target. (If Plexi's host hot-
   reloads the target on file save, you'll see it pick up automatically on
   the next poll.)

## Usage

```
spiral_viewer snake
spiral_viewer ~/.plexi-alpha/apps/wikipedia/manifest.toml
spiral_viewer /path/to/your/app/manifest.toml
```

If no arg is provided, the pane shows instructions.

## Keybindings

| Key        | Action                                     |
|------------|--------------------------------------------|
| `r`        | Re-spawn all instances immediately         |
| `+` / `=`  | Increase instance count (4 → 6 → 8 → 12 → 16) |
| `-` / `_`  | Decrease instance count                    |
| `q` / Esc  | Quit request (logs only; Plexi closes pane) |
| click      | Shows the clicked instance's WxH size      |

## Spiral math

For each index `i ∈ [0, n)`:

```
theta = i * (2π / φ²)            # φ = 1.618... (golden ratio)
r     = base_radius * φ^(i/4)
cx    = pane_cx + r·cos(theta)
cy    = pane_cy + r·sin(theta)
size  = lerp(min_size * 0.9, pane_size * 0.45, i/(n-1))
```

Positions are clamped to stay inside the pane with a small margin so the
largest box still has room for its label and border.

## Limitations (v1)

- `list`, `file_grid`, `video_thumbnail`, `set_cursor`, and `drop_target`
  draw commands are not re-emitted. They are counted and the instance shows
  an "N unrenderable" footer.
- No stateful drive: each captured frame is a one-shot render at t=0. If
  the target needs multiple render frames to settle (animations, async
  data), we see only the first frame. Adequate for breakpoint visualization.
- Target subprocesses get a hard 2s render timeout. A hanging or crashing
  target shows a red error rect but spiral viewer itself keeps running.
- Stdlib only, no third-party deps.
