# Snake — Proof-of-Concept Game

**Status:** Spec  
**Last updated:** 2026-04-11  
**Depends on:** Advanced UI SDK (delta_time, FrameTimer)  
**App type:** Out-of-process (Python)

---

## Summary

Classic Snake game built entirely on the Plexi draw protocol. Zero permissions. Proves the Advanced SDK's game loop, input handling, and render timing work. Also serves as a minimal example for anyone building a game on Plexi.

---

## Why This App

Snake is the simplest game that exercises every game-relevant feature:
- Fixed-interval tick loop (not frame-rate dependent)
- Grid-based rendering (rects)
- Keyboard input (arrow keys)
- Collision detection (self + walls)
- State machine (menu → playing → game over)
- Score display (text)
- Responsive to pane size (grid scales to fit)

If Snake works smoothly, the protocol can handle any 2D game.

---

## Gameplay

Standard rules:
- Snake moves on a grid. One cell per tick.
- Arrow keys change direction. Can't reverse into yourself.
- Eating food grows the snake by one segment and increments score.
- Hitting the wall or your own body = game over.
- Speed increases every 5 food items (tick interval decreases).

---

## Rendering

### Grid

The game area is a grid that fills the pane. Cell size is computed dynamically:

```python
cell_size = min(ctx.width // GRID_COLS, ctx.height // GRID_ROWS)
offset_x = (ctx.width - cell_size * GRID_COLS) // 2
offset_y = (ctx.height - cell_size * GRID_ROWS) // 2
```

Default grid: 20×20. Adjusts if the pane is very small or very wide.

### Visual Style

- **Background**: dark (`#1e1e2e`)
- **Grid lines**: subtle (`#313244`, 1px lines between cells)
- **Snake head**: bright accent (`#89b4fa`)
- **Snake body**: slightly dimmer gradient from head to tail (`#89b4fa` → `#45475a`)
- **Food**: warm color (`#f38ba8`), pulsing (scale oscillates via sine wave on `ctx.time`)
- **Walls**: match grid lines (implied by the grid boundary)

### States

**Title screen:**
```
         🐍 SNAKE

      Press Enter to start

        High score: 42
```

**Playing:** grid + snake + food + score in top-right corner.

**Game over:**
```
        GAME OVER

        Score: 17

     Press Enter to restart
```

---

## Input

| Key | Action |
|-----|--------|
| Arrow Up / `w` | Change direction: up |
| Arrow Down / `s` | Change direction: down |
| Arrow Left / `a` | Change direction: left |
| Arrow Right / `d` | Change direction: right |
| Enter | Start game / restart after game over |
| Escape | Return to title screen (from playing or game over) |
| `p` | Pause/unpause |

Direction changes are buffered — if the user presses Right then Down in the same tick, both are applied in order on subsequent ticks. This prevents the common bug where fast inputs are lost.

---

## Game Loop

Uses `FrameTimer` from the Advanced SDK:

```python
class SnakeGame:
    def __init__(self):
        self.tick_interval = 0.15  # seconds between moves
        self.timer = FrameTimer(interval=self.tick_interval)

    def on_render(self, ctx):
        if self.state == "playing" and self.timer.ready(ctx.delta_time):
            self.advance()  # move snake, check collisions, spawn food
        self.draw(ctx)
```

The game logic runs at a fixed tick rate regardless of render frame rate. Rendering happens every frame (smooth food pulsing, score animation), but snake movement is decoupled.

---

## Manifest

```toml
[app]
id = "snake"
name = "Snake"
version = "0.1.0"
description = "Classic Snake — proof of concept game"

[capabilities]
# No capabilities needed — pure rendering + input
```

---

## File Structure

```
~/.plexi/apps/snake/
  manifest.toml
  snake.py
  plexi_sdk.py
  plexi_sdk_advanced.py
```

Single file app (`snake.py`). Target: under 300 lines.

---

## MVP

The whole thing. Snake is small enough to ship complete:
1. Title screen → playing → game over state machine
2. Grid rendering, responsive to pane size
3. Snake movement at fixed tick rate
4. Food spawning, collision, score
5. Speed increase over time
6. High score persistence (write to a local file via filesystem permission — or just keep in memory for true zero-permission)

High score persistence is the only feature that would need `filesystem.read_write`. For a true zero-permission demo, keep high score in memory only (resets on app close). Worth it for the simplicity.
