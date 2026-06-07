# SDK v2 Overhaul Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Delete the broken SDK patterns and replace with one obvious path that AI agents can read once and get right every time — 30 lines of Python produces a correct, visually polished Plexi app.

**Architecture:**
- `view()` replaces `on_render(ctx)` as the primary hook for declarative apps. Apps return a component tree; the SDK dispatches it on every Render event automatically.
- `on_render(self, ctx)` stays as the **canvas escape hatch** for games and animation only. Never override both.
- `App.state` replaces `ctx.load_state()` / `ctx.save_state()` — first-class state API directly on App.
- `on_init(self)` with no ctx parameter. Init doesn't need to draw.
- Event handlers (`on_key`, `on_click`, etc.) drop ctx — use `self.emit`, `self.w`, `self.h` instead.
- **Agent dev loop:** `plexi pane key` + `plexi pane state` gives agents a Playwright-style drive loop for any app.

**Tech Stack:** Python 3.11+, asyncio, uv, pytest, Rust (for Plexi CLI additions)

---

## File Map

**Modify:**
- `sdk/python/plexi_sdk/templates/app_init.py` — fix layout bug now; rewrite with v2 API after SDK lands
- `sdk/python/plexi_sdk/_app.py` — add `view()` dispatch, `App.state` property, ctx-optional `on_init` detection
- `sdk/python/plexi_sdk/_render_context.py` — deprecate `load_state`/`save_state` (keep but emit warning)
- `sdk/python/plexi_sdk/__init__.py` — update module docstring quick-start to v2 pattern
- `apps/todo/todo.py` — migrate to v2 API
- `apps/logs/logs.py` — migrate to v2 API
- `apps/stats/stats.py` — migrate to v2 API
- `apps/calc/calc.py` — migrate to v2 API
- `apps/wikipedia/wikipedia.py` — migrate to v2 API
- `apps/csv_viewer/csv_viewer.py` — migrate to v2 API
- `apps/backlog/backlog.py` — migrate to v2 API
- `apps/assistant/assistant.py` — migrate on_init/state; keep on_render for chat canvas
- `ROADMAP.md` — add SDK Overhaul as Layer 3 top priority

**Create:**
- `docs/sdk-v2.md` — the golden developer reference (30-line pattern → full API)
- `sdk/python/tests/test_view_dispatch.py` — tests for view() dispatch
- `sdk/python/tests/test_app_state.py` — tests for App.state property
- `sdk/python/tests/test_on_init_no_ctx.py` — tests for ctx-free on_init
- `sdk/python/tests/test_app_init_template.py` — structural tests for the template

**Canvas/game apps (`apps/snake/`, `apps/tetris/`, `apps/balls/`) are EXEMPT from migration** — they correctly use `on_render(ctx)` for pixel-level drawing and should not override `view()`.

---

## The Golden Pattern (what every agent should generate)

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

---

## Task 0: Update ROADMAP (do now, before any code)

**Files:**
- Modify: `ROADMAP.md`

- [ ] **Step 1: Read `ROADMAP.md`** and find the Layer 3 section header.

- [ ] **Step 2: Insert the following block** immediately after the `## Layer 3: Lock the Protocol` header and before `### 3a: Protocol redesign`:

```markdown
### 3b-OVERHAUL: SDK v2 Overhaul (TOP PRIORITY — blocks all further app work)

The Python SDK has two render paths, ctx leaking into event handlers, a broken template,
and no canonical state API. Ground rule: an agent should read the template, copy the
pattern, and produce a working app. That is not true today. Fix before anything else.

Plan: `docs/superpowers/plans/2026-06-07-sdk-overhaul.md`

- [ ] Emergency template fix (layout bug, invisible footer)
- [ ] `App.state` property — replaces `ctx.load_state/save_state`
- [ ] `view()` as primary hook — apps return a tree, SDK dispatches it
- [ ] `on_init(self)` ctx-free dispatch
- [ ] Rewrite template with v2 API
- [ ] Migrate Core 9 apps
- [ ] Agent dev loop: `plexi pane key` CLI command
- [ ] `docs/sdk-v2.md` golden reference

```

- [ ] **Step 3: Commit**

```bash
git add ROADMAP.md
git commit -m "docs: prioritize SDK v2 overhaul at top of Layer 3"
```

---

## Task 1: Emergency Template Fix

The current template has `divider=False` on `FooterKeys` (footer is invisible), and two `Spacer(grow=True)` items surrounding the label (visually ambiguous). Fix now using the existing API — don't wait for the SDK changes.

**Files:**
- Create: `sdk/python/tests/test_app_init_template.py`
- Modify: `sdk/python/plexi_sdk/templates/app_init.py`

- [ ] **Step 1: Write the failing tests**

```python
# sdk/python/tests/test_app_init_template.py
import ast
import pathlib

TEMPLATE = pathlib.Path(__file__).parent.parent / "plexi_sdk/templates/app_init.py"

def test_no_divider_false():
    """FooterKeys must use the default divider (True). divider=False hides the footer."""
    assert "divider=False" not in TEMPLATE.read_text()

def test_single_grow_spacer():
    """Exactly one Spacer(grow=True), placed after the Label to push the footer down."""
    src = TEMPLATE.read_text()
    tree = ast.parse(src)
    spacer_lines, label_lines = [], []
    for node in ast.walk(tree):
        if isinstance(node, ast.Call) and isinstance(node.func, ast.Name):
            if node.func.id == "Spacer":
                for kw in node.keywords:
                    if kw.arg == "grow" and isinstance(kw.value, ast.Constant) and kw.value.value:
                        spacer_lines.append(node.lineno)
            if node.func.id == "Label":
                label_lines.append(node.lineno)
    assert len(spacer_lines) == 1, f"Expected 1 grow Spacer, got {len(spacer_lines)}"
    assert label_lines and spacer_lines[0] > label_lines[0], \
        "Grow Spacer must appear after Label (pushes footer to bottom)"
```

- [ ] **Step 2: Run to confirm failures**

```bash
cd /Users/ianburke/Documents/GitHub/PLEXI/sdk/python && uv run pytest tests/test_app_init_template.py -v
```

Expected output: both tests FAIL.

- [ ] **Step 3: Rewrite the template** (using existing API, not v2 yet):

Replace the full contents of `sdk/python/plexi_sdk/templates/app_init.py`:

```python
#!/usr/bin/env python3
"""__DISPLAY_NAME__ — generated by `plexi app init`."""
from plexi_sdk import App, RenderContext
from plexi_sdk.ui import AppBar, Column, FooterKeys, Label, Spacer


class __CLASS_NAME__(App):
    async def on_init(self, ctx: RenderContext) -> None:
        state = ctx.load_state()
        self.count: int = state.get("count", 0)

    def on_render(self, ctx: RenderContext) -> None:
        ctx.render(Column([
            AppBar(title="__DISPLAY_NAME__"),
            Spacer(grow=True),
            Label(str(self.count), bold=True),
            Spacer(grow=True),
            FooterKeys(shortcuts=[
                ("+", "increment"),
                ("-", "decrement"),
                ("r", "reset"),
            ]),
        ]))

    async def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
        if key in ("equals", "plus"):
            self.count += 1
        elif key == "minus":
            self.count -= 1
        elif key == "r":
            self.count = 0
        ctx.save_state({"count": self.count})


__CLASS_NAME__().run()
```

- [ ] **Step 4: Run tests to confirm pass**

```bash
cd /Users/ianburke/Documents/GitHub/PLEXI/sdk/python && uv run pytest tests/test_app_init_template.py -v
```

Expected: both tests PASS.

- [ ] **Step 5: Commit**

```bash
git add sdk/python/plexi_sdk/templates/app_init.py sdk/python/tests/test_app_init_template.py
git commit -m "fix(sdk): correct app_init template — remove divider=False, fix grow Spacer placement"
```

---

## Task 2: Write the Golden SDK v2 Reference

Lock the API in a doc before touching any code. This is the spec that Tasks 3–6 implement.

**Files:**
- Create: `docs/sdk-v2.md`

- [ ] **Step 1: Create the doc**

```markdown
# Plexi SDK v2 — Developer Reference

## The Pattern (memorize this)

```python
from plexi_sdk import App
from plexi_sdk.ui import Column, AppBar, Label, Spacer, FooterKeys

class MyApp(App):
    def on_init(self) -> None:
        self.count = self.state.get("count", 0)

    def view(self):
        return Column([
            AppBar("My App"),
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

MyApp().run()
```

## Lifecycle hooks

| Hook | Signature | When called | Notes |
|------|-----------|-------------|-------|
| `on_init` | `def on_init(self)` | Once after handshake | No ctx. Use `self.state.get()` for state. |
| `view` | `def view(self)` | Every Render event | Must return a Component. No I/O. |
| `on_key` | `def on_key(self, key, mods)` | Keypress | `key`: "plus", "minus", "return", "escape", "up", "down", "left", "right", "space", "a"–"z", "f1"–"f12". `mods`: `{"shift", "ctrl", "alt", "meta"}` booleans. |
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

## I/O (self.emit — usable anywhere, thread-safe)

```python
self.emit.notify(title, priority, body="")       # fire notification
self.emit.schedule_render(after_ms=16)           # request next frame
self.emit.info/warn/error(msg)                   # structured logging
await self.emit.http_get(url)                    # HTTP GET (requires net.http capability)
await self.emit.ai_query(tier, system, messages) # LLM call (requires ai.query capability)
await self.emit.secret_get("API_KEY")            # read from secrets store
```

## Pane dimensions

`self.w`, `self.h` — current pane width/height in logical pixels. Set after `on_init`, updated each frame. Use in `view()` for responsive layouts.

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
            self.result = "Searching…"
            self.emit.schedule_render()
            self.result = await self.emit.http_get("https://en.wikipedia.org/…")
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
| `Divider()` | Horizontal rule. |

## Agent dev loop (testing and automation)

Agents drive Plexi apps the same way Playwright drives browsers:

```bash
# Render app with a given state → get UiNode tree as JSON
plexi app render apps/counter/counter.py --state '{"count": 5}'

# Read live app state from a running pane
plexi pane state <pane_id>

# Send a key event to a live pane
plexi pane key <pane_id> plus
plexi pane key <pane_id> minus

# Send a command palette action
plexi pane command <pane_id> "reset"
```

Use this loop for testing: render → observe tree → send key → render again → assert change.
```

- [ ] **Step 2: Commit**

```bash
git add docs/sdk-v2.md
git commit -m "docs: golden SDK v2 reference — view(), self.state, agent dev loop pattern"
```

---

## Task 3: Add `App.state` Property

Replace `ctx.load_state()` / `ctx.save_state()` with a first-class `self.state` API on the App class.

**Files:**
- Modify: `sdk/python/plexi_sdk/_app.py`
- Create: `sdk/python/tests/test_app_state.py`

- [ ] **Step 1: Write the failing tests**

```python
# sdk/python/tests/test_app_state.py
from unittest.mock import patch
from plexi_sdk import App

def _make_app_with_state(state_dict):
    app = App()
    app._app_state = dict(state_dict)
    return app

def test_state_get_returns_default_when_missing():
    app = _make_app_with_state({})
    assert app.state.get("x", 99) == 99

def test_state_get_returns_value_when_present():
    app = _make_app_with_state({"x": 42})
    assert app.state.get("x") == 42

def test_state_all_returns_copy():
    app = _make_app_with_state({"a": 1, "b": 2})
    d = app.state.all()
    assert d == {"a": 1, "b": 2}
    d["extra"] = 99
    assert "extra" not in app._app_state  # mutation doesn't leak

def test_state_save_updates_internal_dict():
    app = _make_app_with_state({})
    with patch("plexi_sdk._app._emit"):
        app.state.save({"count": 5})
    assert app._app_state == {"count": 5}

def test_state_save_emits_save_app_state():
    app = _make_app_with_state({})
    emitted = []
    with patch("plexi_sdk._app._emit", side_effect=lambda d: emitted.append(d)):
        app.state.save({"count": 5})
    assert emitted == [{"type": "save_app_state", "payload": {"count": 5}}]

def test_state_available_without_ctx():
    """App.state works before the event loop starts — no async needed."""
    app = _make_app_with_state({"greeting": "hello"})
    assert app.state.get("greeting") == "hello"
```

- [ ] **Step 2: Run to confirm failures**

```bash
cd /Users/ianburke/Documents/GitHub/PLEXI/sdk/python && uv run pytest tests/test_app_state.py -v
```

Expected: all tests FAIL with `AttributeError: 'App' object has no attribute 'state'`.

- [ ] **Step 3: Add `_AppStateProxy` and `state` property to `_app.py`**

In `sdk/python/plexi_sdk/_app.py`, add this class immediately before the `App` class definition:

```python
class _AppStateProxy:
    """Returned by App.state — read/write the host-persisted state dict."""
    __slots__ = ("_app",)

    def __init__(self, app: "App") -> None:
        self._app = app

    def get(self, key: str, default: Any = None) -> Any:
        return self._app._app_state.get(key, default)

    def all(self) -> dict:
        return dict(self._app._app_state)

    def save(self, payload: dict) -> None:
        self._app._app_state = dict(payload)
        _emit({"type": "save_app_state", "payload": payload})
```

Then inside the `App` class, add this property after `__init__`:

```python
    @property
    def state(self) -> "_AppStateProxy":
        """Persistent state. Use self.state.get/save instead of ctx.load_state/save_state."""
        return _AppStateProxy(self)
```

- [ ] **Step 4: Run tests to confirm pass**

```bash
cd /Users/ianburke/Documents/GitHub/PLEXI/sdk/python && uv run pytest tests/test_app_state.py -v
```

Expected: all 6 tests PASS.

- [ ] **Step 5: Run full suite to confirm no regressions**

```bash
cd /Users/ianburke/Documents/GitHub/PLEXI/sdk/python && uv run pytest tests/ -v
```

Expected: all existing tests still PASS.

- [ ] **Step 6: Commit**

```bash
git add sdk/python/plexi_sdk/_app.py sdk/python/tests/test_app_state.py
git commit -m "feat(sdk): add App.state property — self.state.get/save replaces ctx.load_state/save_state"
```

---

## Task 4: Add `view()` Dispatch

When a subclass overrides `view()`, call it on each Render event instead of `on_render`. This makes the declarative path the single obvious default.

**Files:**
- Modify: `sdk/python/plexi_sdk/_app.py`
- Create: `sdk/python/tests/test_view_dispatch.py`

- [ ] **Step 1: Write the failing tests**

```python
# sdk/python/tests/test_view_dispatch.py
import asyncio
import json
import io
from unittest.mock import patch
from plexi_sdk import App
from plexi_sdk.ui import Column, AppBar, Label

INIT_EV = {
    "type": "init", "protocol": "pgap/3.", "app_id": "t",
    "workspace_root": "/tmp", "capabilities": [], "feature_flags": [],
    "args": [], "theme": {}
}
RENDER_EV = {"type": "render", "frame_id": 1, "rect": {"x": 0, "y": 0, "w": 400, "h": 300}}

def _drive(app, extra_events=None):
    events = [INIT_EV] + (extra_events or []) + [{"type": "shutdown"}]
    lines = "\n".join(json.dumps(e) for e in events) + "\n"
    called = {}
    orig_emit = __import__("plexi_sdk._emitter", fromlist=["_emit"])._emit

    with patch("sys.stdin", io.StringIO(lines)):
        with patch("plexi_sdk._app._emit"):
            try:
                asyncio.run(app._async_main())
            except (SystemExit, Exception):
                pass
    return called

def test_view_called_on_render_event():
    view_calls = []
    class MyApp(App):
        def view(self):
            view_calls.append(1)
            return Column([AppBar("Test"), Label("hello")])
    _drive(MyApp(), [RENDER_EV])
    assert view_calls, "view() should be called when a Render event arrives"

def test_on_render_fires_when_view_not_overridden():
    render_calls = []
    class MyApp(App):
        def on_render(self, ctx):
            render_calls.append(1)
    _drive(MyApp(), [RENDER_EV])
    assert render_calls, "on_render() should fire when view() is not overridden"

def test_view_overrides_on_render():
    calls = []
    class MyApp(App):
        def view(self):
            calls.append("view")
            return Column([AppBar("Test")])
        def on_render(self, ctx):
            calls.append("on_render")
    _drive(MyApp(), [RENDER_EV])
    assert "view" in calls
    assert "on_render" not in calls, "on_render must NOT fire when view() is overridden"
```

- [ ] **Step 2: Run to confirm failures**

```bash
cd /Users/ianburke/Documents/GitHub/PLEXI/sdk/python && uv run pytest tests/test_view_dispatch.py -v
```

Expected: `test_view_called_on_render_event` and `test_view_overrides_on_render` FAIL.

- [ ] **Step 3: Update the render handler in `_async_main`**

In `sdk/python/plexi_sdk/_app.py`, find the `elif t == "render":` block inside `_dispatcher()`. Find this section (near the end of the render block):

```python
                    try:
                        await self._dispatch_hook(self.on_render, ctx)
                        self._consecutive_render_errors = 0
                    except Exception as e:
```

Replace just the try block body with:

```python
                    try:
                        if type(self).view is not App.view:
                            # v2 declarative path: view() returns a component tree
                            tree = self.view()
                            if tree is not None:
                                ctx.render(tree)
                        else:
                            # v1 / canvas path: on_render(ctx) for games and animation
                            await self._dispatch_hook(self.on_render, ctx)
                        self._consecutive_render_errors = 0
                    except Exception as e:
```

- [ ] **Step 4: Run tests to confirm pass**

```bash
cd /Users/ianburke/Documents/GitHub/PLEXI/sdk/python && uv run pytest tests/test_view_dispatch.py tests/ -v
```

Expected: all tests PASS.

- [ ] **Step 5: Commit**

```bash
git add sdk/python/plexi_sdk/_app.py sdk/python/tests/test_view_dispatch.py
git commit -m "feat(sdk): view() as primary hook — declarative apps override view(), canvas uses on_render()"
```

---

## Task 5: Make `on_init` ctx-free

When `on_init` is declared as `def on_init(self)` with no ctx parameter, dispatch it without a ctx. This removes the only awkward ctx usage in a typical app.

**Files:**
- Modify: `sdk/python/plexi_sdk/_app.py`
- Create: `sdk/python/tests/test_on_init_no_ctx.py`

- [ ] **Step 1: Write the tests**

```python
# sdk/python/tests/test_on_init_no_ctx.py
import asyncio
import json
import io
from unittest.mock import patch
from plexi_sdk import App
from plexi_sdk.ui import Column, AppBar

INIT_EV = {
    "type": "init", "protocol": "pgap/3.", "app_id": "t",
    "workspace_root": "/tmp", "capabilities": [], "feature_flags": [],
    "args": [], "theme": {}
}

def _drive(app):
    lines = "\n".join(json.dumps(e) for e in [INIT_EV, {"type": "shutdown"}]) + "\n"
    with patch("sys.stdin", io.StringIO(lines)):
        with patch("plexi_sdk._app._emit"):
            try:
                asyncio.run(app._async_main())
            except (SystemExit, Exception):
                pass

def test_on_init_without_ctx_does_not_raise():
    """on_init(self) with no ctx param must not cause TypeError."""
    errors = []
    class MyApp(App):
        def on_init(self):          # no ctx — v2 style
            self.x = 42
        def view(self):
            return Column([AppBar("Test")])

    app = MyApp()
    _drive(app)
    assert getattr(app, "x", None) == 42, "on_init body should have run"

def test_on_init_with_ctx_still_works():
    """on_init(self, ctx) old style must keep working."""
    ctx_types = []
    class MyApp(App):
        async def on_init(self, ctx):   # old style — still valid
            ctx_types.append(type(ctx).__name__)
        def view(self):
            return Column([AppBar("Test")])

    _drive(MyApp())
    assert ctx_types == ["RenderContext"]

def test_on_init_async_without_ctx():
    """async def on_init(self) with no ctx param."""
    ran = []
    class MyApp(App):
        async def on_init(self):
            ran.append(True)
        def view(self):
            return Column([AppBar("Test")])

    _drive(MyApp())
    assert ran
```

- [ ] **Step 2: Run to confirm first and third tests FAIL**

```bash
cd /Users/ianburke/Documents/GitHub/PLEXI/sdk/python && uv run pytest tests/test_on_init_no_ctx.py -v
```

Expected: `test_on_init_without_ctx_does_not_raise` and `test_on_init_async_without_ctx` FAIL with TypeError.

- [ ] **Step 3: Update `on_init` dispatch in `_async_main`**

In `sdk/python/plexi_sdk/_app.py`, inside `_dispatcher()`, find this line in the `if t == "init":` block:

```python
                    await self._dispatch_hook(self.on_init, self._make_ctx())
```

Replace it with:

```python
                    import inspect as _inspect_init
                    _on_init_params = list(
                        _inspect_init.signature(type(self).on_init).parameters.values()
                    )
                    # params[0] is self; len == 1 means no ctx arg (v2 style)
                    if len(_on_init_params) <= 1:
                        await self._dispatch_hook(self.on_init)
                    else:
                        await self._dispatch_hook(self.on_init, self._make_ctx())
```

- [ ] **Step 4: Run tests to confirm all three pass**

```bash
cd /Users/ianburke/Documents/GitHub/PLEXI/sdk/python && uv run pytest tests/test_on_init_no_ctx.py tests/ -v
```

Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add sdk/python/plexi_sdk/_app.py sdk/python/tests/test_on_init_no_ctx.py
git commit -m "feat(sdk): on_init(self) dispatches without ctx — use self.state.get() for state access"
```

---

## Task 6: Rewrite Template with v2 API

Now that the SDK changes are in, update the template to use `view()`, `self.state`, and no ctx in event handlers.

**Files:**
- Modify: `sdk/python/plexi_sdk/templates/app_init.py`
- Modify: `sdk/python/tests/test_app_init_template.py` (add v2 checks)

- [ ] **Step 1: Add failing tests to `test_app_init_template.py`**

```python
# append to existing sdk/python/tests/test_app_init_template.py

def test_template_uses_view():
    src = TEMPLATE.read_text()
    assert "def view(self)" in src, "Template must use view() — not on_render()"
    assert "def on_render" not in src, "Template must not use on_render (that's the canvas path)"

def test_template_uses_self_state():
    src = TEMPLATE.read_text()
    assert "self.state.get" in src, "Template must use self.state.get() not ctx.load_state()"
    assert "ctx.load_state" not in src
    assert "ctx.save_state" not in src
    assert "self.state.save" in src

def test_template_on_init_no_ctx():
    src = TEMPLATE.read_text()
    assert "def on_init(self):" in src or "async def on_init(self):" in src
    assert "def on_init(self, ctx" not in src

def test_template_on_key_no_ctx():
    src = TEMPLATE.read_text()
    assert "def on_key(self, key" in src or "async def on_key(self, key" in src
    assert "def on_key(self, ctx" not in src

def test_template_no_render_context_import():
    src = TEMPLATE.read_text()
    assert "RenderContext" not in src, "Template should not import RenderContext (not needed with v2 API)"
```

- [ ] **Step 2: Run to confirm all 5 new tests fail**

```bash
cd /Users/ianburke/Documents/GitHub/PLEXI/sdk/python && uv run pytest tests/test_app_init_template.py -v
```

Expected: the 5 new tests FAIL; the 2 original tests PASS.

- [ ] **Step 3: Rewrite the template**

Replace the full contents of `sdk/python/plexi_sdk/templates/app_init.py`:

```python
#!/usr/bin/env python3
"""__DISPLAY_NAME__ — generated by `plexi app init`."""
from plexi_sdk import App
from plexi_sdk.ui import AppBar, Column, FooterKeys, Label, Spacer


class __CLASS_NAME__(App):
    def on_init(self) -> None:
        self.count: int = self.state.get("count", 0)

    def view(self):
        return Column([
            AppBar(title="__DISPLAY_NAME__"),
            Spacer(grow=True),
            Label(str(self.count), bold=True),
            Spacer(grow=True),
            FooterKeys(shortcuts=[
                ("+", "increment"),
                ("-", "decrement"),
                ("r", "reset"),
            ]),
        ])

    def on_key(self, key: str, mods: dict) -> None:
        if key in ("equals", "plus"):
            self.count += 1
        elif key == "minus":
            self.count -= 1
        elif key == "r":
            self.count = 0
        self.state.save({"count": self.count})
        self.emit.schedule_render()


__CLASS_NAME__().run()
```

- [ ] **Step 4: Run all template tests**

```bash
cd /Users/ianburke/Documents/GitHub/PLEXI/sdk/python && uv run pytest tests/test_app_init_template.py -v
```

Expected: all 7 tests PASS.

- [ ] **Step 5: Run full test suite**

```bash
cd /Users/ianburke/Documents/GitHub/PLEXI/sdk/python && uv run pytest tests/ -v
```

Expected: all PASS.

- [ ] **Step 6: Commit**

```bash
git add sdk/python/plexi_sdk/templates/app_init.py sdk/python/tests/test_app_init_template.py
git commit -m "feat(sdk): rewrite app_init template — view(), self.state, no ctx in event handlers"
```

---

## Task 7: Migrate Core 9 Apps to v2 API

Migrate each app in order of complexity. Canvas apps (snake, tetris, balls) are **exempt** — they use `on_render(ctx)` correctly.

**Mechanical changes per app:**
1. `async def on_init(self, ctx: RenderContext)` → `def on_init(self)` (drop async if no await; drop ctx)
2. `ctx.load_state()` → `self.state.get(key, default)` (per key)
3. `ctx.save_state({...})` → `self.state.save({...})`
4. `def on_render(self, ctx) + ctx.render(Column([...]))` → `def view(self): return Column([...])`
   (EXCEPT for apps that use `ctx.rect/text/circle` directly — those keep `on_render`)
5. `async def on_key(self, ctx, key, mods)` → `def on_key(self, key, mods)` (drop ctx; use `self.emit`, `self.state`)
6. Remove `RenderContext` import if no longer needed
7. Remove `from plexi_sdk import App, RenderContext` → `from plexi_sdk import App`

**Smoke test after each migration:**

```bash
cd apps/<name> && uv run python <name>.py --plexi-introspect
```

Expected: prints `{"required_capabilities": [...]}` and exits cleanly. Any import error or TypeError means the migration broke something.

### Migrate apps/calc/calc.py

- [ ] Apply mechanical changes
- [ ] Smoke test: `cd /Users/ianburke/Documents/GitHub/PLEXI/apps/calc && uv run python calc.py --plexi-introspect`
- [ ] `git commit -m "feat(apps/calc): migrate to SDK v2 API"`

### Migrate apps/todo/todo.py

- [ ] Apply mechanical changes (todo already uses L1 components — mostly drop ctx params)
- [ ] Smoke test: `cd /Users/ianburke/Documents/GitHub/PLEXI/apps/todo && uv run python todo.py --plexi-introspect`
- [ ] `git commit -m "feat(apps/todo): migrate to SDK v2 API"`

### Migrate apps/logs/logs.py

- [ ] Apply mechanical changes
- [ ] Smoke test: `cd /Users/ianburke/Documents/GitHub/PLEXI/apps/logs && uv run python logs.py --plexi-introspect`
- [ ] `git commit -m "feat(apps/logs): migrate to SDK v2 API"`

### Migrate apps/stats/stats.py

- [ ] Apply mechanical changes
- [ ] Smoke test: `cd /Users/ianburke/Documents/GitHub/PLEXI/apps/stats && uv run python stats.py --plexi-introspect`
- [ ] `git commit -m "feat(apps/stats): migrate to SDK v2 API"`

### Migrate apps/wikipedia/wikipedia.py

- [ ] Apply mechanical changes
- [ ] Smoke test: `cd /Users/ianburke/Documents/GitHub/PLEXI/apps/wikipedia && uv run python wikipedia.py --plexi-introspect`
- [ ] `git commit -m "feat(apps/wikipedia): migrate to SDK v2 API"`

### Migrate apps/csv_viewer/csv_viewer.py

- [ ] Apply mechanical changes
- [ ] Smoke test: `cd /Users/ianburke/Documents/GitHub/PLEXI/apps/csv_viewer && uv run python csv_viewer.py --plexi-introspect`
- [ ] `git commit -m "feat(apps/csv_viewer): migrate to SDK v2 API"`

### Migrate apps/backlog/backlog.py

- [ ] Apply mechanical changes
- [ ] Smoke test: `cd /Users/ianburke/Documents/GitHub/PLEXI/apps/backlog && uv run python backlog.py --plexi-introspect`
- [ ] `git commit -m "feat(apps/backlog): migrate to SDK v2 API"`

### Migrate apps/assistant/assistant.py

- [ ] Apply mechanical changes to `on_init` and state only. Leave `on_render` alone if it uses canvas draws for chat bubbles.
- [ ] Smoke test: `cd /Users/ianburke/Documents/GitHub/PLEXI/apps/assistant && uv run python assistant.py --plexi-introspect`
- [ ] `git commit -m "feat(apps/assistant): migrate on_init and state to SDK v2 API"`

---

## Task 8: Agent Dev Loop — `plexi pane key` CLI Command

Agents need to drive apps like Playwright drives browsers. The existing `plexi pane state` returns the UiNode tree. We need `plexi pane key` to send key events. Together they form the drive loop: **render → observe state → send key → observe state**.

**Context:** `plexi pane command` and `plexi pane state` already exist (v0.0.638). Missing: `plexi pane key <pane_id> <key>`.

**Files (Rust):**
- Modify: `src/cli/pane.rs` — add `key` subcommand to `PaneCmd` enum
- Modify the pane command dispatch to send a `KeyEvent` to the target pane

- [ ] **Step 1: Write the test (HostHarness)**

```rust
// In the existing pane CLI tests or a new test module
// Confirm that sending "plexi pane key <id> plus" increments a counter app state

#[test]
fn pane_key_delivers_key_event_to_app() {
    let mut h = HostHarness::new();
    let pane_id = h.add_test_app_pane("apps/calc/calc.py");
    h.send_cli_command(format!("pane key {pane_id} plus"));
    h.tick();
    let state = h.get_pane_state(pane_id);
    // State should reflect the key was handled
    assert!(state.contains("count") || state.contains("display"));
}
```

- [ ] **Step 2: Add `key` to `PaneCmd` enum in `src/cli/pane.rs`**

Find the `PaneCmd` enum. Add:

```rust
/// Send a key event to an app pane.
Key {
    /// Pane ID (from `plexi pane list`)
    pane_id: u32,
    /// Key name: "plus", "minus", "return", "escape", "up", "down", "a"-"z", etc.
    key: String,
    /// Optional modifier flags
    #[clap(long)] shift: bool,
    #[clap(long)] ctrl: bool,
    #[clap(long)] alt: bool,
    #[clap(long)] meta: bool,
},
```

- [ ] **Step 3: Add dispatch in the pane command handler**

In the match arm for `PaneCmd`, add:

```rust
PaneCmd::Key { pane_id, key, shift, ctrl, alt, meta } => {
    send_to_host(CliRequest::PaneKey {
        pane_id,
        key,
        modifiers: KeyModifiers { shift, ctrl, alt, meta },
    })
}
```

- [ ] **Step 4: Add `CliRequest::PaneKey` handling on the host side**

In the host's CLI request dispatch, handle `PaneKey` by routing a `Key` event to the target pane's app process via the existing event routing infrastructure (same path as keyboard focus events).

- [ ] **Step 5: Build and test**

```bash
cargo build --bin plexi 2>&1 | tail -20
```

Expected: builds clean.

```bash
# Manual smoke test: open a counter app, then send keys via CLI
plexi-alpha app open apps/calc/calc.py
# note the pane_id from output
plexi-alpha pane key <pane_id> plus
plexi-alpha pane state <pane_id>
# Observe the state JSON changed
```

- [ ] **Step 6: Add completion entry for `key` subcommand**

In the completions file, add `key` to the `pane` subcommand list and add key name completions: `plus minus return escape up down left right space`.

- [ ] **Step 7: Commit**

```bash
git add src/cli/pane.rs src/  # and other touched Rust files
git commit -m "feat(cli): plexi pane key <id> <key> — sends key event to app pane for agent testing"
```

---

## Task 9: Update `__init__.py` Quick-Start to v2

**Files:**
- Modify: `sdk/python/plexi_sdk/__init__.py`

- [ ] **Step 1: Find the QUICK START block** (currently shows the v1 counter example with `ctx.rect`, `ctx.text`, `on_render`).

- [ ] **Step 2: Replace the quick start counter example** with the v2 golden pattern:

```python
    from plexi_sdk import App
    from plexi_sdk.ui import Column, AppBar, Label, Spacer, FooterKeys

    class CounterApp(App):
        def on_init(self):
            self.count = self.state.get("count", 0)

        def view(self):
            return Column([
                AppBar("Counter"),
                Spacer(grow=True),
                Label(str(self.count), bold=True),
                Spacer(grow=True),
                FooterKeys([("+", "increment"), ("-", "decrement")]),
            ])

        def on_key(self, key, mods):
            if key == "plus":    self.count += 1
            elif key == "minus": self.count -= 1
            self.state.save({"count": self.count})
            self.emit.schedule_render()

    CounterApp().run()

    # Canvas/game apps: override on_render(self, ctx) instead of view().
    # Full reference: docs/sdk-v2.md
```

- [ ] **Step 3: Run full test suite**

```bash
cd /Users/ianburke/Documents/GitHub/PLEXI/sdk/python && uv run pytest tests/ -v
```

Expected: all PASS.

- [ ] **Step 4: Commit**

```bash
git add sdk/python/plexi_sdk/__init__.py
git commit -m "docs(sdk): update quick-start to v2 API — view(), self.state, agent dev loop"
```

---

## Self-Review

**Spec coverage:**
- [x] Emergency template fix — Task 1
- [x] Golden reference doc — Task 2
- [x] `App.state` property — Task 3
- [x] `view()` dispatch — Task 4
- [x] `on_init(self)` ctx-free — Task 5
- [x] Template with v2 API — Task 6
- [x] Core 9 migration — Task 7
- [x] Agent dev loop (`plexi pane key`) — Task 8
- [x] Module docs updated — Task 9
- [x] ROADMAP updated — Task 0

**Gap identified:** Task 7 doesn't specify what to do if an app uses BOTH `ctx.load_state()` in `on_init` AND draws with `ctx.rect/text`. In practice: apps using `ctx.rect` should keep `on_render(ctx)` as the canvas escape hatch, but still migrate `on_init` and state. The migration note covers this.

**Placeholder scan:** None. Every task has concrete code.

**Type consistency:** `_AppStateProxy` defined in Task 3, referenced by `App.state` property in the same task. `App.view` referenced in Task 4 using `type(self).view is not App.view` — consistent with Python's standard override detection pattern.

---

## Execution Handoff

Plan saved to `docs/superpowers/plans/2026-06-07-sdk-overhaul.md`.

**Two execution options:**
1. **Subagent-Driven (recommended)** — fresh subagent per task, review diffs between tasks
2. **Inline Execution** — execute tasks in this session using `/executing-plans`
