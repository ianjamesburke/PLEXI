# Layout Presets

**Status:** Spec  
**Last updated:** 2026-04-11  
**Depends on:** Pane tree model  
**Ships with:** Plexi (built-in)

---

## Summary

Named layout templates that restructure the current workspace's pane arrangement with one action. Includes standard layouts (even split, thirds, focus) and a golden spiral that auto-calculates pane count from screen dimensions and a minimum pixel threshold.

Layouts are triggered from the command palette, a keybinding, or a layout picker overlay.

---

## Preset Definitions

### Even Split

Two equal panes side by side.

```
┌────────────┬────────────┐
│            │            │
│    50%     │    50%     │
│            │            │
└────────────┴────────────┘
```

- Split: horizontal
- Ratio: `0.5`
- Panes: 2

---

### Even Vertical

Two equal panes stacked.

```
┌─────────────────────────┐
│           50%           │
├─────────────────────────┤
│           50%           │
└─────────────────────────┘
```

- Split: vertical
- Ratio: `0.5`
- Panes: 2

---

### Thirds

Three equal columns.

```
┌────────┬────────┬────────┐
│        │        │        │
│  33%   │  33%   │  33%   │
│        │        │        │
└────────┴────────┴────────┘
```

- Split: horizontal, then horizontal on remainder
- Ratios: `0.333`, `0.5` (of remainder)
- Panes: 3

---

### Focus

Large main pane with narrow sidebar.

```
┌───────────────────┬──────┐
│                   │      │
│       75%         │ 25%  │
│                   │      │
└───────────────────┴──────┘
```

- Split: horizontal
- Ratio: `0.75`
- Panes: 2

---

### Focus + Stack

Large main pane with two stacked sidebar panes.

```
┌───────────────────┬──────┐
│                   │      │
│                   │ 50%  │
│       70%         ├──────┤
│                   │      │
│                   │ 50%  │
└───────────────────┴──────┘
```

- Split: horizontal at `0.7`, then vertical at `0.5` on remainder
- Panes: 3

---

### Quadrants

Four equal panes in a 2×2 grid.

```
┌────────────┬────────────┐
│            │            │
│    TL      │    TR      │
├────────────┼────────────┤
│            │            │
│    BL      │    BR      │
└────────────┴────────────┘
```

- Split: vertical at `0.5`, then horizontal at `0.5` on each half
- Panes: 4

---

### Golden

Two panes at the golden ratio. The primary pane is the larger one.

```
┌────────────────┬──────────┐
│                │          │
│     61.8%      │  38.2%   │
│                │          │
└────────────────┴──────────┘
```

- Split: horizontal
- Ratio: `0.618`
- Panes: 2

---

### Golden Spiral

The signature layout. Recursively subdivides at the golden ratio, alternating horizontal and vertical splits, until the next subdivision would produce a pane smaller than `min_size` on either axis.

**The algorithm:**

```
function golden_spiral(rect, min_size, direction, panes):
    # Check if the next split would produce a pane too small
    if direction is horizontal:
        primary_width = rect.width * 0.618
        remainder_width = rect.width * 0.382
        if remainder_width < min_size:
            panes.append(rect)  # can't split further, this is a leaf
            return
    else:  # vertical
        primary_height = rect.height * 0.618
        remainder_height = rect.height * 0.382
        if remainder_height < min_size:
            panes.append(rect)
            return

    # Split at golden ratio
    primary, remainder = split(rect, 0.618, direction)
    panes.append(primary)

    # Recurse on the remainder, alternating direction
    next_direction = vertical if direction is horizontal else horizontal
    golden_spiral(remainder, min_size, next_direction, panes)
```

**Example: 1920×1080 screen, min_size = 200, origin = bottom-right**

```
┌──────────────────────┬───────────┐
│                      │           │
│                      │  733×668  │
│     1187×1080        │     ②     │
│         ①            ├─────┬─────┤
│                      │ 453 │ 280 │
│                      │×412 │×412 │
│                      │ ③   │ ④   │
└──────────────────────┴─────┴─────┘
```

4 panes. The largest (①) is the main workspace. Each subsequent pane is smaller by the golden ratio. The spiral tightens toward the bottom-right.

**Example: 2560×1440 screen, min_size = 200**

More pixels = one more subdivision:

```
┌──────────────────────────┬──────────────┐
│                          │              │
│                          │   890×890    │
│       1582×1440          │      ②       │
│           ①              ├──────┬───────┤
│                          │ 550  │ 340   │
│                          │ ×550 │ ×550  │
│                          │  ③   ├───┬───┤
│                          │      │340│340│
│                          │      │×340│×210│
│                          │      │ ④ │ ⑤ │
└──────────────────────────┴──────┴───┴───┘
```

5 panes. Same algorithm, just more room to recurse.

**Example: 1280×800 screen (laptop), min_size = 200**

```
┌─────────────────┬────────┐
│                 │        │
│    791×800      │ 489×494│
│       ①        │   ②    │
│                 ├────────┤
│                 │489×306 │
│                 │   ③    │
└─────────────────┴────────┘
```

3 panes. Laptop screen can't fit as many subdivisions.

### Origin Control

The `origin` parameter controls which corner the spiral tightens toward. This determines where the smallest pane ends up and therefore where the largest pane (your main workspace) sits.

| Origin | Largest pane | Spiral tightens toward |
|--------|-------------|----------------------|
| `bottom-right` (default) | Top-left | Bottom-right corner |
| `bottom-left` | Top-right | Bottom-left corner |
| `top-right` | Bottom-left | Top-right corner |
| `top-left` | Bottom-right | Top-left corner |
| `center` | Outer ring | Center (splits from outside in, not a true spiral but a centered recursive subdivision) |

Implementation: the `origin` just controls which side the "primary" (larger) pane goes on at each split. For `bottom-right`, primary goes left on horizontal splits and top on vertical splits, pushing the remainder toward bottom-right.

---

## Configuration

### Global Default

In `~/.plexi/config.toml`:

```toml
[layouts]
default = "golden-spiral"

[layouts.golden-spiral]
min_size = 200
origin = "bottom-right"

[layouts.focus]
ratio = 0.75
```

### Per-Workspace Override

In `.plexi/workspace.json`:

```json
{
  "layout": "golden-spiral",
  "layout_config": {
    "min_size": 250,
    "origin": "top-left"
  }
}
```

### Custom Presets

Users can define their own named layouts in config:

```toml
[layouts.my-layout]
type = "custom"
splits = [
    { direction = "horizontal", ratio = 0.6 },
    { pane = 1, direction = "vertical", ratio = 0.7 },
]
```

This creates:
```
┌───────────────┬─────────┐
│               │         │
│     60%       │  70%    │
│               ├─────────┤
│               │  30%    │
└───────────────┴─────────┘
```

Custom layouts are a flat list of splits applied in order. Each split says which existing pane to subdivide, the direction, and the ratio. Simple, composable, no recursion needed.

---

## Trigger Mechanism

### Command Palette

Type `layout` in the command palette → shows all available presets with a live preview thumbnail.

```
> layout
  ┌─────────────────────────────────┐
  │ Even Split          ▐█ █▌      │
  │ Thirds              ▐█ █ █▌    │
  │ Focus               ▐███ █▌    │
  │ Focus + Stack       ▐███ █▌    │
  │                     ▐    █▌    │
  │ Quadrants           ▐██▌       │
  │                     ▐██▌       │
  │ Golden              ▐████ ██▌  │
  │ Golden Spiral ★     ▐████ ██▌  │
  │                     ▐     █▌   │
  └─────────────────────────────────┘
```

Selecting a layout applies it immediately. Cmd+Z undoes it (restores the previous pane arrangement).

### Keybinding

Default: **Cmd+Shift+\\** → cycles through presets.

Or specific bindings:

```toml
[keybindings]
"cmd+shift+1" = "layout:even"
"cmd+shift+2" = "layout:thirds"
"cmd+shift+g" = "layout:golden-spiral"
```

### Layout Picker Overlay

A floating overlay (not a pane) showing visual thumbnails of all layouts. Triggered by keybinding or command palette. Click a thumbnail to apply. Arrow keys to navigate, Enter to select.

This is the nicest UX but lowest priority — command palette covers the same functionality.

---

## Behavior When Applying a Layout

### Existing Panes

When you apply a layout, Plexi maps existing panes into the new arrangement:

1. **Fewer panes than the layout has slots:** Empty slots get new terminal panes.
2. **More panes than the layout has slots:** Extra panes become tabs in the last slot (smallest pane in golden spiral). Nothing is closed.
3. **Same number:** Panes are mapped by position — top-left stays top-left, etc.

The focused pane always maps to the largest slot (pane ① in golden spiral). This is the most natural behavior — you're reorganizing your workspace, and your main focus stays front and center.

### Undo

Applying a layout pushes the previous arrangement onto a layout history stack. **Cmd+Z** (when no pane is focused or when triggered from the command palette) pops the stack and restores the previous arrangement.

### Resize After Apply

After a layout is applied, panes are still freely resizable. The layout is a starting point, not a constraint. Dragging a border adjusts the ratio. The layout name is no longer "active" — it was a one-shot application.

If you want to re-apply the layout (snap back to clean ratios), trigger it again from the command palette.

---

## Responsive Recalculation

When the Plexi window itself is resized (dragging the window border, or moving between monitors with different resolutions):

- **Static layouts** (even, thirds, focus, quadrants, golden): ratios are preserved. A 50/50 split stays 50/50 regardless of window size.
- **Golden spiral**: if a previously-applied golden spiral is active and the window shrinks enough that a pane would go below `min_size`, that pane collapses into a tab on its neighbor. If the window grows, collapsed panes can be restored. This is opt-in behavior — only kicks in if `responsive = true` in the layout config.

Default: `responsive = false`. Panes just scale proportionally on resize, same as they do today.

---

## Data Model

```rust
enum LayoutPreset {
    Even,
    EvenVertical,
    Thirds,
    Focus { ratio: f32 },
    FocusStack { ratio: f32 },
    Quadrants,
    Golden,
    GoldenSpiral { min_size: u32, origin: Origin },
    Custom { splits: Vec<SplitStep> },
}

enum Origin {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Center,
}

struct SplitStep {
    pane_index: usize,          // which pane to split (0 = first/whole screen)
    direction: SplitDirection,  // Horizontal | Vertical
    ratio: f32,                 // 0.0–1.0, how much the first pane gets
}
```

---

## MVP Scope

1. **Even, Focus, Golden presets** — the three most useful. Applied from command palette.
2. **Golden Spiral** — the `min_size` recursive algorithm. One origin (bottom-right).
3. **Command palette trigger** — type `layout`, pick a preset, applied immediately.
4. **Pane mapping** — focused pane → largest slot, extras become tabs.

**Defer:** Custom user-defined presets, layout picker overlay, responsive recalculation, undo stack, origin control (ship with bottom-right only), keybinding cycling, per-workspace overrides, live preview thumbnails in command palette.
