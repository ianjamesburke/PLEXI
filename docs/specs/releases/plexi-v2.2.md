# Plexi v2.2 — Rich Text & Input Maturity

**Status:** Draft  
**Depends on:** v2.1 (UI primitives — viewport, transforms, MeasureText)

---

## TL;DR

v2.2 completes the text editing story: multi-line input, selection/cursor rendering, IME composition, and rich text runs for syntax highlighting. It adds clip regions for proper viewport clipping, formalizes the input layering contract, and ships `plexi-sdk` on PyPI.

---

## Scope

**Planned for v2.2:**

### Text editing primitives
- `ctx.multiline_text_input(id, value, on_change, ...)` — multi-line text editor with line numbers, soft-wrap option, and configurable height
- Selection primitive: cursor + selection range rendering (highlight rect + cursor beam)
- IME composition support: forward `CompositionStart`, `CompositionUpdate`, `CompositionEnd` as protocol events
- Rich text runs: `ctx.rich_text(x, y, runs)` where `runs` is a list of `{text, size, color, bold, monospace}` — needed for syntax highlighting without multiple overlapping `text()` calls

### Clip regions
- `DrawCommand::ClipRect { x, y, w, h }` — push a clip rectangle; all subsequent draw commands are clipped to this rect
- `DrawCommand::ResetClip` — pop the clip rect
- Required by viewport (content outside bounds leaks in v2.1), text_input (text overflows), and any scrollable container

### Input layering contract (§7.5)
Formalize `docs/specs/proposals/input-layering.md` as a normative spec section:
- Priority tiers: system shortcuts > host chrome > focused app > background apps
- Apps declare `input_priority: "foreground" | "background"` in manifest
- Host guarantees exclusive key delivery to the foreground app; background apps get a filtered subset

### Distribution
- `plexi-sdk` Python package on PyPI (semver, `pip install plexi-sdk`)
- Versioned alongside Plexi releases (0.5.x for v2.1, 0.6.x for v2.2)
- `plexi_sdk.py` remains the canonical single-file copy-paste option for simple apps

---

## Why v2.2 and Not v2.1

These features require deeper changes than v2.1:

- **ClipRect** requires a stateful clipping layer in the renderer — it can't be emulated with transforms.
- **IME** requires OS-level composition event routing that egui doesn't expose cleanly today. Needs investigation.
- **Rich text runs** need a layout pass (wrapping, baseline alignment) that's separate from the simple `painter.text()` calls v2.1 uses.
- **Input layering** is correct but wide-reaching — changing key dispatch semantics needs careful testing against all existing apps.
- **PyPI packaging** is ops work (CI publishing, versioning, README) that doesn't block apps from shipping.

Deferring keeps v2.1 shippable in one focused pass.

---

## §1 — Rich Text Runs

```python
ctx.rich_text(x=10, y=20, runs=[
    {"text": "def ", "size": 13, "color": "#cba6f7", "bold": False, "monospace": True},
    {"text": "hello", "size": 13, "color": "#89b4fa", "bold": False, "monospace": True},
    {"text": "(name):", "size": 13, "color": "#cdd6f4", "bold": False, "monospace": True},
])
```

Protocol: `DrawCommand::RichText { x, y, runs: Vec<TextRun> }` where `TextRun` mirrors the dict above.

---

## §2 — Clip Regions

```json
{"type": "clip_rect", "x": 0, "y": 36, "w": 800, "h": 400}
// ... draw commands clipped to above rect ...
{"type": "reset_clip"}
```

Stack-based like transforms. `reset_clip` pops the last pushed clip; the stack is reset on `FrameDone`.

---

## §3 — Multiline Text Input

```python
bounds = ctx.multiline_text_input(
    "editor",
    value=state["text"],
    on_change=lambda v: state.update({"text": v}),
    x=0, y=0, w=ctx.width, h=ctx.height - 40,
    font_size=13.0,
    line_numbers=True,
    monospace=True,
)
```

Handles its own scrolling. Returns bounding rect. Cursor position tracked in app state.

---

## §4 — IME Composition Events

New `PlexiEvent` variants:
```
CompositionStart
CompositionUpdate { text: String }
CompositionEnd { text: String }
```

SDK: `@app.on_composition_update(text, emit)` handler.

---

## §5 — Input Layering Contract

Normative rules for key event delivery:

1. **System shortcuts** (macOS menu, Cmd+Q, Cmd+Tab) — consumed by OS, never reach Plexi.
2. **Host chrome shortcuts** (Cmd+N, Cmd+W, Ctrl+/) — consumed by Plexi before any app sees them. Defined in `src/keys.rs`.
3. **Foreground app** — receives all remaining key events first. Can consume via returning `true` from `handle_key`.
4. **Background apps** — receive only non-consumed events if they have `input_priority = "background"` and the event type is in their `observes` list.

Manifest declaration:
```toml
[app]
input_priority = "foreground"  # default
observes = ["key_events"]      # opt-in for background key observation
```

---

## §6 — PyPI Distribution

Package name: `plexi-sdk`  
Import: `from plexi_sdk import App`  
Versioning: `0.x.y` matching Plexi minor.patch (0.6.0 for v2.2)

Single module (`plexi_sdk.py`) — no dependencies, no build step. `pip install plexi-sdk` is just a convenience wrapper around the same file.

---

## Ship Order

1. ClipRect protocol + renderer
2. Rich text runs protocol + renderer
3. Multiline text input SDK component
4. Selection + cursor rendering
5. IME event routing (if egui API permits)
6. Input layering contract finalization
7. PyPI package setup + CI publish

---

## Cross-references

- v2.1 spec: `docs/specs/releases/plexi-v2.1.md`
- v2.3 spec: `docs/specs/releases/plexi-v2.3.md`
- Input layering proposal: `docs/specs/proposals/input-layering.md`
