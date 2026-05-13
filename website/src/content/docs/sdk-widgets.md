---
title: "Widgets"
description: "Reusable UI components for Plexi apps"
verified_version: "3.6.54"
---

# Widgets

Import widgets from `plexi_sdk.widgets`:

```python
from plexi_sdk.widgets import (
    Button, ButtonStyle,
    KeyMap,
    ListView,
    ScrollState,
    TextArea, TextAreaTheme,
    TextBuffer, Cursor, Selection,
    emit_text_input,
)
```

## ButtonStyle

## Button

Stateful button widget. Tracks its own hover/click state across frames.

Usage::

    btn = Button("ok", x=20, y=40, w=80, h=32, label="OK")

    def on_render(self, ctx):
        if btn.render(ctx):
            handle_ok()

### `render`

```python
def render(ctx: 'RenderContext') -> bool
```

Draw the button and return True if clicked this frame.

---

## KeyMap

Declarative keyboard shortcut registry.

Usage::

    km = KeyMap()
    km.bind("s", mod="cmd", action="save")
    km.bind("q", action="quit")

    def on_key(self, ctx, key, mods):
        action = km.handle(key, mods)
        if action == "save":
            save()
        elif action == "quit":
            quit()

### `bind`

```python
def bind(key: str, action: str, mod: str = '') -> None
```

Bind a key + optional modifier to an action name.

`mod` is a pipe-separated string of modifier names:
"cmd", "shift", "ctrl", "alt". Order does not matter.
Example: ``km.bind("s", "save", mod="cmd")``

---

### `handle`

```python
def handle(key: str, mods: dict) -> 'str | None'
```

Return the action name for this key+mods, or None if unbound.

`mods` is the dict from on_key: {"shift": bool, "ctrl": bool,
"alt": bool, "meta": bool}. "meta" is treated as "cmd".

---

## ListView

Stateful scrollable list widget with fixed item height.

Usage::

    class MyApp(App):
        def on_init(self, ctx):
            self._list = ListView(item_height=48.0)

        def on_render(self, ctx):
            ctx.render(Column([
                AppBar("Title"),
                self._list.render([
                    ListItem(title=items[i], selected=i == self._list.selected_index)
                    for i in range(len(items))
                ]),
            ]))

        def on_key(self, ctx, key, mods):
            if self._list.handle_key(key):
                self.emit.schedule_render(after_ms=16)

        def on_click(self, ctx, x, y, button):
            idx = self._list.hit_test(y)
            if idx is not None:
                self._list.set_selected(idx)

### `selected_index`

```python
def selected_index() -> int
```

---

### `set_selected`

```python
def set_selected(idx: int) -> None
```

---

### `handle_key`

```python
def handle_key(key: str) -> bool
```

Move selection for j/k/arrows. Returns True if the key was consumed.

---

### `hit_test`

```python
def hit_test(y: float) -> int | None
```

Return item index at screen-coord y (from last render), or None.

---

### `render`

```python
def render(items: list) -> '_ListViewRenderer'
```

Return a renderable Component containing the given items.

Call inside Column([...]) on every frame — the returned object is
lightweight and safe to recreate each render.

---

## ScrollState

Pure data model for vertical scroll state.

Offset is always clamped to [0, max_offset] after any mutation.
Negative inputs for content_height and viewport_height are clamped to 0
with a warning rather than raising, so callers with stale layout data
degrade gracefully.

### `content_height`

```python
def content_height() -> float
```

---

### `content_height`

```python
def content_height(value: float) -> None
```

---

### `viewport_height`

```python
def viewport_height() -> float
```

---

### `viewport_height`

```python
def viewport_height(value: float) -> None
```

---

### `offset`

```python
def offset() -> float
```

---

### `max_offset`

```python
def max_offset() -> float
```

---

### `scroll_by`

```python
def scroll_by(dy: float) -> None
```

Positive dy scrolls down (reveals content below).

---

### `scroll_to`

```python
def scroll_to(y: float) -> None
```

Set offset directly. Clamped to [0, max_offset].

---

### `ensure_visible`

```python
def ensure_visible(y: float, margin: float = 0.0) -> None
```

Adjust offset so content-coord y is inside the viewport.

If y is above the viewport (y < offset + margin), scroll up so
y lands at offset + margin.
If y is below the viewport (y > offset + viewport_height - margin),
scroll down so y lands at offset + viewport_height - margin.
Otherwise no-op. Result is always clamped.

---

## TextAreaTheme

## TextArea

TextBuffer + ScrollState + theme → DrawCommands.

The widget owns geometry (position + size) at render time; the buffer and
scroll are passed in so callers control state.

### `on_key`

```python
def on_key(key: str, shift: bool = False, ctrl: bool = False) -> None
```

Handle a key event. Mutates buffer + keeps cursor visible via scroll.

---

### `ensure_cursor_visible`

```python
def ensure_cursor_visible() -> None
```

---

### `render`

```python
def render(x: float, y: float, w: float, h: float) -> list[dict]
```

Return DrawCommands for a TextArea at the given viewport rect.

Updates scroll.viewport_height and scroll.content_height as a side
effect (so it clamps correctly when the viewport changes size).
All returned coordinates are absolute (include x, y offsets).

---

## Cursor

## Selection

Anchor is fixed; head follows the cursor.

The selection is inclusive of anchor and exclusive of head, like most
editors. Use normalized() to always get (start, end) in document order.

### `is_empty`

```python
def is_empty() -> bool
```

---

### `normalized`

```python
def normalized() -> tuple[Cursor, Cursor]
```

Return (start, end) where start <= end in document order.

---

## TextBuffer

Pure text model: lines, cursor, and optional selection.

Internal representation: list of strings, no trailing newlines per line.
An empty buffer contains exactly one empty string.

### `lines`

```python
def lines() -> list[str]
```

Read-only view of the line list.

---

### `cursor`

```python
def cursor() -> Cursor
```

---

### `selection`

```python
def selection() -> Selection | None
```

---

### `text`

```python
def text() -> str
```

---

### `selected_text`

```python
def selected_text() -> str
```

---

### `insert`

```python
def insert(ch: str) -> None
```

Insert text at cursor, replacing any existing selection first.

---

### `insert_newline`

```python
def insert_newline() -> None
```

Convenience wrapper; equivalent to insert('\n').

---

### `delete_back`

```python
def delete_back() -> None
```

Backspace. Deletes selection if one exists, else deletes char before cursor.

---

### `delete_forward`

```python
def delete_forward() -> None
```

Forward delete. Symmetric to delete_back.

---

### `move_cursor`

```python
def move_cursor(direction: str, select: bool = False) -> None
```

Move cursor in the given direction.

Valid directions: 'left', 'right', 'up', 'down',
                  'home', 'end', 'doc_start', 'doc_end'.

If select=True the selection head is extended; otherwise any existing
selection is cleared first (standard editor behaviour: pressing an
arrow key without Shift collapses to the nearer edge).

---

### `set_cursor`

```python
def set_cursor(row: int, col: int, select: bool = False) -> None
```

Jump cursor to (row, col), clamped to valid positions.

---

### `select_to`

```python
def select_to(row: int, col: int) -> None
```

Extend selection head to (row, col); creates selection if none.

---

### `clear_selection`

```python
def clear_selection() -> None
```

---

### `select_all`

```python
def select_all() -> None
```

---

