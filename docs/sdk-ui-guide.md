# Plexi SDK v2 — UI Guide

**Status:** shipped in alpha 2026-04-23.
**Source:** [`sdk/python/plexi_sdk/ui.py`](../sdk/python/plexi_sdk/ui.py)
**Reference app:** [`examples/ui-playground/`](../examples/ui-playground/)

---

## TL;DR

Don't paint pixels. Describe the UI as a tree, pass it to `ctx.render(...)`, and the SDK handles layout, padding, truncation, and wrapping.

```python
from plexi_sdk import App, RenderContext
from plexi_sdk.ui import Column, Header, Card, KeyRow, Spacer, Footer

class MyApp(App):
    def on_render(self, ctx: RenderContext) -> None:
        ctx.render(Column([
            Header("My App", "Short subtitle"),
            Card([
                KeyRow("m", "Do the thing"),
                KeyRow("q", "Quit"),
            ]),
            Spacer(grow=True),
            Footer("Hint at the bottom"),
        ]))
```

Result: a clean, padded, responsive pane with no hand-positioned pixels.

---

## Why this exists

Before SDK v2, every app used `ctx.rect`, `ctx.text`, `ctx.circle` with hand-chosen pixel coordinates. This was correct for low-level power but wrong for defaults: every new app reinvented header layout, card padding, footer placement, text truncation. Mistakes accumulated (text falling off edges, inconsistent gaps, bottom sections clipping when panes shrank).

The same lesson every UI framework eventually learns: **give apps components with opinionated defaults, and make the primitives the escape hatch, not the starting point.**

---

## When to use what

Every SDK v2 layout starts with a root `Column`. Inside, stack components vertically. When you need nested stacking inside a container, use `Card` (a styled surface) — it takes its own list of children and handles internal padding + gap.

| Need | Use |
|---|---|
| Root container | `Column` |
| Top-of-pane title block | `Header(title, subtitle=...)` |
| Inline section divider with label | `Section("Events")` |
| Plain horizontal rule | `Divider()` |
| Standalone title text | `Heading(text, level=1-3)` |
| Paragraph / caption / hint | `Label(text, tone="body"/"caption"/"hint")` |
| Key hint in a list | `KeyRow("m", "Description")` |
| Surface-colored box with children | `Card([...], padding=SPACE_MD)` |
| Scrollable, bounded log | `ScrollLog(lines)` |
| Fixed vertical gap | `Spacer(size=SPACE_MD)` |
| Fill remaining space, push later items down | `Spacer(grow=True)` |
| Caption row pinned at the bottom | `Footer("text")` |

---

## The layout rules

1. **Root must be a `Column`.** (For MVP — more root types may come later.)
2. `Column` lays children top-to-bottom with uniform `gap` and outer `padding`.
3. Each component declares a fixed height, or `grow=True` (currently only `Spacer` grows).
4. `Column` measures fixed children first, then distributes leftover space to grow spacers.
5. When the pane is too small, fixed-height children lower in the stack get clipped (rare — watch for it). When it's bigger than needed, grow spacers soak up the slack.

---

## Responsive behavior

Every component was designed to degrade gracefully when the pane is narrow:

- `Heading` and `KeyRow` truncate with `…` when text exceeds the available width.
- `Label` wraps up to 3 lines, then truncates.
- `Footer` wraps up to 2 lines, then truncates.
- `ScrollLog` shows only as many recent lines as fit; older lines scroll off the top.
- `Card` and `Column` shrink their inner content width with outer width; padding stays constant.

Open the `ui-playground` app and drag the pane edges — everything reflows without breaking.

---

## Style tokens

The `ui` module re-exports these constants. Use them for any custom pixel values so you stay on the Plexi scale:

```python
from plexi_sdk.ui import (
    SPACE_XS, SPACE_SM, SPACE_MD, SPACE_LG, SPACE_XL,
    TEXT_HINT, TEXT_CAPTION, TEXT_BODY, TEXT_HEADING, TEXT_TITLE, TEXT_TITLE_XL,
    RADIUS_SM, RADIUS_MD, RADIUS_LG,
    BG, SURFACE, HIGHLIGHT, ACCENT, MUTED, FG, RED, GREEN, YELLOW,
)
```

These mirror [`src/style.rs`](../src/style.rs) on the Rust side — keep them in sync when adding new tokens.

---

## The escape hatch

SDK v2 components emit low-level `DrawCommand`s under the hood. If you need something the components don't cover (canvas apps, games, custom widgets), every existing `ctx.rect`, `ctx.text`, `ctx.circle`, `ctx.line`, `ctx.arc` method still works. You can mix: render a component tree at the top of `on_render`, then draw custom primitives in the space below.

```python
def on_render(self, ctx):
    # Components render the frame chrome.
    ctx.render(Column([
        Header("Canvas Demo"),
    ]))
    # Then paint a custom game loop.
    ctx.circle(ctx.w / 2, ctx.h / 2, 20.0, ACCENT)
```

Note: the component tree paints *over* a `ctx.clear(BG)` at the start of `ctx.render`. If you want the component tree to sit on top of a custom background, call your own `ctx.rect` draws *after* `ctx.render` with appropriate positioning.

---

## Writing a new component

Subclass `Component` and implement `measure(avail_w) -> height` and `render(ctx, x, y, w, h) -> None`. Keep it small. Opt into `is_grow()` only if your component is meant to fill slack.

```python
from dataclasses import dataclass
from plexi_sdk.ui import Component, ACCENT, SPACE_SM, TEXT_BODY

@dataclass
class Badge(Component):
    text: str
    color: str = ACCENT

    def measure(self, avail_w: float) -> float:
        return TEXT_BODY + 2 * SPACE_SM

    def render(self, ctx, x, y, w, h) -> None:
        ctx.rect(x, y, w, h, self.color, radius=4.0)
        ctx.text(x + SPACE_SM, y + h - SPACE_SM - 2,
                 self.text, size=TEXT_BODY, color="#000000", bold=True)
```

Contribute it back to `sdk/python/plexi_sdk/ui.py` once it proves useful in two or more apps.

---

## What's not here (yet)

- **Row layouts.** Today everything stacks vertically. When two apps need horizontal layout for the same element, add `Row(children, gap=...)`.
- **Tabs / accordions / modals.** Modals are host-level (see the notification modal); tabs and accordions will come if enough apps want them.
- **Animation.** No transitions yet. Component appearance is frame-by-frame static.
- **Scrolling containers beyond `ScrollLog`.** A general `Scroll` container is future work.

Add these when the need exists. Don't speculatively invent components — the cost of a bad API in the SDK is rewrites across every app.

---

## Testing Apps with AppHarness

`AppHarness` in `plexi_sdk.testing` spawns a real Python Plexi app subprocess and lets you drive it headlessly — no running Plexi instance or display required.

```python
from plexi_sdk.testing import AppHarness

with AppHarness("my_app.py", width=400, height=300) as h:
    cmds = h.run(1)                     # step one render frame
    h.key("enter")                      # inject a key event
    cmds = h.run(1)                     # render again to see effects
    assert any(c.get("type") == "text" for c in cmds)
```

### Methods

- `run(n_frames=1)` — step N render frames; returns draw commands from the last frame
- `key(key, modifiers=None)` — inject a synthetic key event (e.g. `"enter"`, `"a"`)
- `screenshot()` — render draw commands to PNG bytes (requires `plexi` binary or `PLEXI_RENDER_BIN`)
- `assert_pixel(x, y, expected, tolerance=4)` — assert a pixel color in the last rendered frame
- `save_snapshot(path)` — write the last rendered frame to a PNG file
- `close()` — shut down the subprocess; also called by `__exit__`

### CI usage

`AppHarness` runs without a display. Pixel assertions via `screenshot()` require the `plexi` binary; skip them in CI by guarding with:

```python
import os
if os.environ.get("PLEXI_RENDER_BIN") or Path("target/release/plexi").exists():
    h.assert_pixel(10, 10, "#ff0000")
```

Run the full test suite with:
```
cd sdk/python && uv run pytest tests/
```
