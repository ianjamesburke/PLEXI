# Plexi SDK v2 — Developer Reference

SDK v2 is the Python authoring API. PGAP v3 (`pgap/3`) is the host/app wire
protocol the SDK speaks. They are different version lines: SDK v2 apps normally
emit PGAP v3 component trees.

## The Pattern (memorize this)

```python
from plexi_sdk import App
from plexi_sdk.ui import Column, AppBar, Label, Spacer, FooterKeys

class CounterApp(App):
    def on_init(self) -> None:
        self.count = self.state.get("count", 0)

    def view(self):
        return Column([
            AppBar("Counter"),
            Spacer(grow=True),
            Label(str(self.count), bold=True),
            Spacer(grow=True),
            FooterKeys([("+", "increment"), ("-", "decrement")]),
        ])

    def on_key(self, key: str, mods: dict) -> None:
        if key == "plus":    self.count += 1
        elif key == "minus": self.count -= 1
        self.state.save({"count": self.count})
        self.emit.schedule_render()

CounterApp().run()
```

## Lifecycle hooks

| Hook | Signature | When called | Notes |
|------|-----------|-------------|-------|
| `on_init` | `def on_init(self)` | Once after handshake | No ctx. Use `self.state.get()` for state. |
| `view` | `def view(self)` | Every Render event | Must return a Component. No I/O. |
| `on_key` | `def on_key(self, key, mods)` | Keypress | `key`: "plus", "minus", "return", "escape", "up", "down", "left", "right", "space", "a"-"z", "f1"-"f12". `mods`: `{"shift", "ctrl", "alt", "meta"}` booleans. |
| `on_click` | `def on_click(self, x, y, button)` | Pointer event | `button`: "primary", "secondary", "middle". |
| `on_path_changed` | `def on_path_changed(self, cwd)` | Workspace dir changed | `cwd`: absolute path string. |
| `on_escape` | `def on_escape(self) -> bool` | Escape key | Return `True` if handled (e.g. dismissed a modal). Return `False` to let host close the app. |
| `on_shutdown` | `def on_shutdown(self)` | Host closing app | Cleanup only. No emit after this. |

All hooks may be `async def`. Async hooks can `await self.emit.*` helpers.

## State

```python
self.state.get("key", default)  # read one value
self.state.all()                 # get full dict
self.state.save({"key": value}) # write and persist full dict
```

State is persisted by the host across app restarts. Call `self.emit.schedule_render()` after mutations.

## Reactive fields (optional convenience)

```python
from plexi_sdk import App, State

class MyApp(App):
    count = State(0)  # auto-calls schedule_render() on assignment
```

`State` descriptors are complementary to `self.state` (host persistence). Use `State` for fields that trigger re-render on change; use `self.state.save()` to persist across restarts.

## I/O (self.emit)

```python
self.emit.notify(title, priority, body="")       # fire notification
self.emit.schedule_render(after_ms=16)           # request next frame
self.emit.info/warn/error(msg)                   # structured logging
await self.emit.http_get(url)                    # HTTP GET (requires net.http capability)
await self.emit.ai_query(tier, system, messages) # LLM call (requires ai.query capability)
await self.emit.secret_get("API_KEY")            # read from secrets store
```

## Pane dimensions

`self.w`, `self.h` — current pane width/height in logical pixels. Updated each frame. Use in `view()` for responsive layouts.

## App with async I/O

```python
class WikiApp(App):
    def on_init(self) -> None:
        self.result = ""

    def view(self):
        return Column([
            AppBar("Wikipedia"),
            Label(self.result or "Press s to search"),
        ])

    async def on_key(self, key, mods):
        if key == "s":
            self.result = "Searching..."
            self.emit.schedule_render()
            self.result = await self.emit.http_get("https://en.wikipedia.org/...")
            self.emit.schedule_render()
```

## Canvas escape hatch (games, animation)

Override `on_render(self, ctx)` instead of `view()`. Use `ctx.rect()`, `ctx.text()`, `ctx.circle()`, etc.

```python
class SnakeApp(App):
    def on_render(self, ctx):   # NOT view() — this is the pixel-control path
        ctx.rect(x, y, w, h, "#ff0000")
        ctx.text(16, 16, "Score: 5", size=14.0, color="#cdd6f4")
```

**Never override both `view()` and `on_render()`.**

## Layout components (plexi_sdk.ui)

| Component | Usage |
|-----------|-------|
| `Column([...])` | Vertical stack. The root container for all apps. |
| `AppBar(title, subtitle=None)` | Top bar. Host shows Esc-to-close in the chrome. |
| `Spacer(grow=True)` | Fills remaining space. Put exactly one before FooterKeys. |
| `Label(text, tone="body", bold=False)` | Text. `tone`: "body", "caption", "hint". |
| `Section(title)` | Section header with divider line. |
| `FooterKeys(shortcuts)` | Bottom key hint row. `shortcuts`: list of `(key, description)` tuples. |
| `SelectList(items, selected_idx=0)` | Keyboard-navigable list. |
| `TextInput(id, placeholder="")` | Single-line text input. |
| `TextEdit(node_id, placeholder="", multiline=False, height=48.0)` | Host-rendered text editor. Change/submit events via `on_component_event`. |
| `Divider()` | Horizontal rule. |

## Agent dev loop (testing and automation)

Agents drive Plexi apps the same way Playwright drives browsers:

```bash
# Render app with a given state — get UiNode tree as JSON
plexi app render apps/counter/counter.py --state '{"count": 5}'

# Read live app state from a running pane
plexi pane state <pane_id>

# Send a key event to a live pane
plexi pane key <pane_id> plus
plexi pane key <pane_id> minus
```

Use this loop for testing: render -> observe tree -> send key -> render again -> assert change.
