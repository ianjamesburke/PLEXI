# Plexi Python SDK v3 — Design Specification

**Status:** authoritative design doc for task 0285. Implement exactly this. No design decisions during implementation.

Source of truth for all types: `wit/plexi.wit`. Every Python type here maps to a named WIT type. Where a mapping decision was made, it is stated explicitly.

---

## 1. App Contract

A Python app is three module-level functions. No class, no inheritance.

```python
from plexi_sdk import state, log
from plexi_sdk.effects import SetTitle
from plexi_sdk.events import KeyEvent
from plexi_sdk.ui import Column, Text, Button, AppBar, FooterKeys

def init(size: tuple[float, float], args: list[str]) -> list:
    """Called once on launch. Return initial effects."""
    return [SetTitle("My App")]

def update(event) -> list:
    """Called for every input event. Return effects."""
    if isinstance(event, KeyEvent) and event.key == "q":
        from plexi_sdk.effects import CloseSelf
        return [CloseSelf()]
    return []

def view():
    """Called after any state mutation. Must be pure."""
    count = state.get("count", 0)
    return Column([
        AppBar("My App"),
        Text(str(count), bold=True),
        FooterKeys([("q", "quit")]),
    ])
```

**Adapter contract:**
- The adapter calls `init(size, args)` on launch, passing the pane size and any args from `plexi run`.
- Before every `update(event)` call, the adapter sets `plexi_sdk._state` to the current `StateSnapshot`.
- Before every `view()` call, the adapter sets `plexi_sdk._state` to the current `StateSnapshot`.
- Before every `view()` call, the adapter dispatches `RenderFrame` through `update(event)` without auto-scheduling another render. Continuous apps opt into host-paced animation with `SetSchedulerMode("continuous", fps=60)`.
- `state` in `plexi_sdk` is a module-level proxy that reads `plexi_sdk._state`.
- The adapter flattens the `UINode` tree returned by `view()` into a WIT `ui-tree` arena.

**Naming:** functions must be named exactly `init`, `update`, `view` at module level. The adapter looks them up by name via `getattr(module, "init")` etc.

---

## 2. State Access

```python
# plexi_sdk/__init__.py exposes:
from plexi_sdk import state   # StateProxy instance

state.get(key: str, default=None) -> any       # JSON-decode value, return default if absent
state.set(key: str, value: any) -> SetState    # returns a runtime-state effect (does NOT mutate immediately)
state.all() -> dict[str, any]                  # all keys decoded
state.raw(key: str) -> bytes | None            # raw bytes, no decode
```

**`state.set()` returns an effect, it does not mutate immediately.** The app returns `[state.set("count", 5)]` from `update()`. The adapter applies the update to the process-local SDK snapshot, then calls `view()`.

**`SetState` effect:** process-local runtime state. Use it for view/update data, game state, caches, and animation state. It never writes host app-state files.

```python
@dataclass
class SetState:
    data: dict  # {key: value} — values must be JSON-serializable
```

**`PersistState` effect:** explicit durable state. It updates the same runtime snapshot and writes the full app-state snapshot through the host.

```python
@dataclass
class PersistState:
    data: dict
```

**State in `view()`:** `state.get()` reads the snapshot set by the adapter before the call. It is read-only inside `view()` — calling `state.set()` inside `view()` raises `RuntimeError("state.set() called inside view() — return SetState from update() instead")`.

**`_state` internal:** `plexi_sdk._state: StateSnapshot | None`. Set by adapter. `StateProxy.get()` raises `RuntimeError` if `_state` is None (called outside lifecycle).

---

## 3. Logging

```python
from plexi_sdk import log

log.debug(msg: str)   # → host-log::debug
log.info(msg: str)    # → host-log::info
log.warn(msg: str)    # → host-log::warn
log.error(msg: str)   # → host-log::error
```

Backed by `host-log` WIT interface. String formatting is Python's responsibility: `log.info(f"count={x}")`.

---

## 4. Effects

All effects are dataclasses in `plexi_sdk/effects.py`. Every WIT `effect` variant has a 1:1 Python class. Snake_case field names match WIT record field names (hyphens → underscores).

```python
from __future__ import annotations
from dataclasses import dataclass, field
from typing import Optional

# ── State ─────────────────────────────────────────────────────────────────────

@dataclass
class SetState:
    data: dict  # process-local runtime state

@dataclass
class PersistState:
    data: dict  # runtime state plus explicit durable app-state save

# ── File I/O ──────────────────────────────────────────────────────────────────

@dataclass
class FileRead:
    path: str   # → file-read-effect { path }

@dataclass
class FileList:
    path: str
    extensions: list = field(default_factory=list)  # host filters files by extension

@dataclass
class FileWrite:
    path: str
    content: bytes  # → file-write-effect { path, content: list<u8> }

# ── Network ───────────────────────────────────────────────────────────────────

@dataclass
class HttpFetch:
    url: str
    method: str = "GET"
    headers: dict = field(default_factory=dict)  # {name: value}
    body: Optional[bytes] = None
    # → http-fetch-effect { url, method, headers: list<tuple<string,string>>, body: option<list<u8>> }
    # adapter converts headers dict to list[tuple[str,str]]

@dataclass
class OpenUrl:
    url: str  # host opens an HTTP(S) URL in the default browser after allowlist checks

# ── AI ────────────────────────────────────────────────────────────────────────

@dataclass
class AiMessage:
    role: str   # "user" | "assistant" | "system"
    content: str

@dataclass
class AiQuery:
    request_id: str  # app-assigned, echoed back in AiResponse event
    model_tier: str  # "low" | "medium" | "high"
    system: str
    messages: list  # list[AiMessage]
    # → ai-query-effect

# ── Timers ────────────────────────────────────────────────────────────────────

@dataclass
class SetTimer:
    id: int         # app-assigned u32; echoed in TimerFired event
    delay_ms: int
    repeat: bool = False
    # → set-timer(timer-effect { id, delay-ms, repeat })

@dataclass
class CancelTimer:
    id: int         # → cancel-timer(u32)

# ── Rendering ────────────────────────────────────────────────────────────────

@dataclass
class SetSchedulerMode:
    mode: str       # "idle" | "scheduled" | "continuous"
    fps: int | None = None
    # continuous is for games/animations; it drives RenderFrame events.

# ── System ────────────────────────────────────────────────────────────────────

@dataclass
class GetSystemStats:
    pass    # → get-system-stats (no payload)

# ── Pane chrome ───────────────────────────────────────────────────────────────

@dataclass
class SetTitle:
    title: str      # → set-title(string)

@dataclass
class SetStatus:
    text: str       # → set-status(string)

@dataclass
class CloseSelf:
    pass            # → close-self

@dataclass
class RequestCapability:
    name: str       # → request-capability(string)

# ── Events ────────────────────────────────────────────────────────────────────

@dataclass
class EventStreamDecl:
    name: str
    schema_json: str
    description: Optional[str] = None

@dataclass
class DeclareEventStreams:
    streams: list   # list[EventStreamDecl]
    # → declare-event-streams-effect

@dataclass
class EmitEvent:
    event: str
    actor: str
    summary: str
    resource_id: str
    revision_after: str
    actor_id: Optional[str] = None
    caused_by: Optional[str] = None
    resource_scope: Optional[str] = None
    payload_json: Optional[str] = None
    state_ref: Optional[str] = None
    revision_before: Optional[str] = None
    rollback_token: Optional[str] = None
    changed_resources: list = field(default_factory=list)
    suggested_trigger: Optional[str] = None
    # → emit-event-effect (field names: hyphens → underscores in Python)
```

**Adapter marshalling:** the adapter walks the returned `list`, checks `type(effect).__name__`, and dispatches to the appropriate WIT effect variant. Unknown types raise `TypeError(f"Unknown effect type: {type(effect).__name__}")`.

---

## 5. Events

All events are dataclasses in `plexi_sdk/events.py`. Every WIT `input-event` variant has a 1:1 Python class. The adapter deserializes the WIT `input-event` into the correct Python class before calling `update(event)`.

```python
from __future__ import annotations
from dataclasses import dataclass
from typing import Optional

@dataclass
class Modifiers:
    ctrl: bool = False
    shift: bool = False
    alt: bool = False
    meta: bool = False

@dataclass
class KeyEvent:
    key: str            # WIT key string e.g. "a", "enter", "escape", "space"
    modifiers: Modifiers = field(default_factory=Modifiers)
    pressed: bool = True
    # → input-event::key(key-event)

@dataclass
class MouseEvent:
    x: float
    y: float
    button: Optional[str] = None  # "left" | "right" | "middle" | None
    pressed: bool = False
    scroll_x: float = 0.0
    scroll_y: float = 0.0
    # → input-event::mouse(mouse-event)
    # adapter: position.x/y, button from enum, scroll_delta.x/y

@dataclass
class UiAction:
    handler_id: str     # → input-event::ui-action(ui-action-event)

@dataclass
class UiValueChange:
    handler_id: str
    value: str          # → input-event::ui-value-change(ui-value-change-event)

@dataclass
class Resize:
    width: float
    height: float       # → input-event::resize(size)

@dataclass
class FocusGained:
    pass                # → input-event::focus-gained

@dataclass
class FocusLost:
    pass                # → input-event::focus-lost

@dataclass
class TimerFired:
    id: int             # → input-event::timer-fired(u32)

@dataclass
class RenderFrame:
    frame_id: int       # process-local monotonically increasing frame id
    elapsed: float      # seconds since previous render frame

@dataclass
class SystemStats:
    cpu_usage_pct: float
    memory_used_bytes: int
    memory_total_bytes: int
    disk_read_bps: int
    disk_write_bps: int
    net_rx_bps: int
    net_tx_bps: int
    uptime_secs: int
    load_avg_one_min: float

@dataclass
class SystemStatsResult:
    stats: SystemStats  # → input-event::system-stats-result(system-stats)

@dataclass
class FileReadResult:
    content: Optional[bytes]    # None on error
    error: Optional[str]        # None on success
    # → input-event::file-read-result(result<list<u8>, string>)

@dataclass
class FileWriteResult:
    error: Optional[str]        # None on success
    # → input-event::file-write-result(result<_, string>)

@dataclass
class HttpResponse:
    status: int
    headers: list               # list[tuple[str, str]]
    body: bytes
    # → input-event::http-response(http-response)

@dataclass
class AiStreamChunk:
    request_id: str
    delta: str
    reasoning: Optional[str]
    done: bool
    # → input-event::ai-stream-chunk(ai-stream-chunk-event)

@dataclass
class AiResponse:
    request_id: str
    content: Optional[str]
    tokens_in: int
    tokens_out: int
    error: Optional[str]
    # → input-event::ai-response(ai-response-event)

@dataclass
class DeclareEventStreamsResult:
    streams: Optional[list]     # list[str] on success, None on error
    error: Optional[str]
    # → input-event::declare-event-streams-result

@dataclass
class EmitEventResult:
    sequence: Optional[int]     # u64 on success, None on error
    error: Optional[str]
    # → input-event::emit-event-result

@dataclass
class SurfaceReady:
    texture_handle: int
    width: int
    height: int
    # → input-event::surface-ready(surface-event) — GPU apps only

@dataclass
class SurfaceResized:
    texture_handle: int
    width: int
    height: int
    # → input-event::surface-resized(surface-event) — GPU apps only

@dataclass
class PipePayload:
    binary: Optional[bytes] = None
    json: Optional[str] = None

@dataclass
class PipeMessage:
    handle: int
    payload: PipePayload
    # → input-event::pipe-message(pipe-message-event)

@dataclass
class PipePeerConnected:
    handle: int             # → input-event::pipe-peer-connected(u32)

@dataclass
class PipeClosed:
    handle: int             # → input-event::pipe-closed(u32)

@dataclass
class PipeError:
    handle: int
    error: str              # → input-event::pipe-error(tuple<u32, string>)

@dataclass
class CapabilityGranted:
    name: str               # → input-event::capability-granted(string)

@dataclass
class CapabilityDenied:
    name: str               # → input-event::capability-denied(string)

@dataclass
class PaymentComplete:
    pass                    # → input-event::payment-complete

@dataclass
class PaymentFailed:
    reason: str             # → input-event::payment-failed(string)
```

**Adapter dispatch:** the adapter receives a WIT `input-event`, reads the variant tag, constructs the corresponding Python dataclass, and passes it to `update(event)`.

---

## 6. UINode

All UINode types are dataclasses in `plexi_sdk/ui/__init__.py`. They map to WIT `ui-node-data` variants. The adapter flattens the nested tree into a WIT `ui-tree` arena after `view()` returns.

```python
from __future__ import annotations
from dataclasses import dataclass, field
from typing import Optional

@dataclass
class Color:
    r: int; g: int; b: int; a: int = 255
    # → types::color { r, g, b, a }

# Alignment values map to WIT enum alignment { start, center, end, stretch }
# Python: use string literals "start" | "center" | "end" | "stretch"

# ── Leaf nodes ────────────────────────────────────────────────────────────────

@dataclass
class Empty:
    pass    # → ui-node-data::empty

@dataclass
class Text:
    text: str
    size: Optional[float] = None
    bold: bool = False
    color: Optional[Color] = None
    truncate: bool = False
    align: str = "start"    # "start" | "center" | "end" | "stretch"
    # → ui-node-data::text(text-node)

@dataclass
class Button:
    label: str
    on_click: str           # handler_id — matched by UiAction event
    style: str = "secondary"  # "primary" | "secondary" | "danger" | "ghost"
    disabled: bool = False
    # → ui-node-data::button(button-node)

@dataclass
class TextEdit:
    node_id: str
    value: str = ""
    placeholder: str = ""
    multiline: bool = False
    max_length: int = 0
    # → ui-node-data::text-edit(text-edit-node)

@dataclass
class ProgressBar:
    value: float
    max: float = 1.0
    color: Optional[Color] = None
    label: Optional[str] = None
    # → ui-node-data::progress-bar(progress-bar-node)

@dataclass
class Badge:
    text: str
    color: str = "neutral"  # "accent" | "success" | "warning" | "danger" | "neutral"
    # → ui-node-data::badge(badge-node)

@dataclass
class Divider:
    pass    # → ui-node-data::divider

@dataclass
class Space:
    size: float     # → ui-node-data::space(f32)

@dataclass
class Surface:
    width: int
    height: int
    # → ui-node-data::surface(surface-node { width, height, texture-handle: None })
    # GPU apps only. texture_handle filled in by host after surface-ready event.

# ── Container nodes ───────────────────────────────────────────────────────────

@dataclass
class Row:
    children: list      # list[UINode]
    gap: float = 0.0
    align: str = "start"
    grow: bool = False
    # → ui-node-data::row(row-node)

@dataclass
class Column:
    children: list      # list[UINode]
    gap: float = 0.0
    align: str = "start"
    grow: bool = False
    # → ui-node-data::column(column-node)

@dataclass
class ListView:
    items: list             # list[UINode] — each item is a UINode, usually a Row or Text
    selected: Optional[int] = None
    on_select: Optional[str] = None  # handler_id — matched by UiAction event
    # → ui-node-data::list-view(list-node)

@dataclass
class Scroll:
    child: object           # UINode
    horizontal: bool = False
    # → ui-node-data::scroll(scroll-node)

@dataclass
class Padding:
    child: object           # UINode
    top: float = 0.0
    right: float = 0.0
    bottom: float = 0.0
    left: float = 0.0
    # → ui-node-data::padding(padding-node)

# ── Convenience composites (Python-only, no WIT equivalent) ──────────────────
# These build from primitive nodes. Adapter never sees these as distinct types —
# they return a UINode tree built from the primitives above.

def AppBar(title: str, subtitle: str = "") -> Column:
    """Standard app top bar. Renders title + optional subtitle in a Column."""
    items = [Text(title, bold=True, size=15.0)]
    if subtitle:
        items.append(Text(subtitle, size=11.0))
    return Column(items, gap=2.0)

def FooterKeys(pairs: list) -> Row:
    """Standard footer key hints. pairs: list[tuple[str, str]] = [(key, label), ...]"""
    children = []
    for key, label in pairs:
        children.append(Badge(key, color="neutral"))
        children.append(Text(label, size=11.0))
        children.append(Space(8.0))
    return Row(children, gap=4.0, align="center")

def Section(title: str, children: list) -> Column:
    """Titled section group."""
    return Column([
        Text(title, bold=True, size=11.0),
        Divider(),
        *children,
    ], gap=4.0)
```

### Tree flattening (adapter responsibility)

The adapter calls `view()`, receives a root UINode, and flattens it:

```
_next_id = 0
arena = []  # list[indexed-node]

def flatten(node, key_prefix="") -> node-id:
    id = _next_id++
    key = key_prefix or str(id)
    match type(node):
        case Column:
            child_ids = [flatten(c, f"{key}/{i}") for i, c in enumerate(node.children)]
            arena.append(indexed-node(id, key, column-node(child_ids, node.gap, node.align, node.grow)))
        case Text:
            arena.append(indexed-node(id, key, text-node(node.text, node.size, node.bold, ...)))
        ...
    return id

root_id = flatten(root_node)
return ui-tree { root: root_id, nodes: arena }
```

Reset `_next_id` to 0 before each `view()` call. Keys are positional by default; apps can set a `key` attribute on any node to override (add optional `key: str = ""` to every UINode dataclass — empty string = use positional).

---

## 7. Native ProcessApp Bridge Protocol

SDK v3 Python apps run through native `ProcessApp`: the host starts the system Python interpreter as a subprocess and invokes `python -m plexi_sdk._v3_process <entry.py>`. PGAP remains the process transport. WASM-contained Python is not part of this contract.

### Bootstrap sequence

1. `ProcessApp::launch` resolves the `.py` entry from the manifest and launches it with the SDK path on `PYTHONPATH`.
2. `plexi_sdk._v3_process` loads the app module from that entry file.
3. The runtime reads PGAP JSON events from stdin and emits PGAP JSON commands on stdout.
4. Lifecycle calls are ordinary Python function calls into module-level `init(size, args)`, `update(event)`, and `view()`.

### Runtime protocol

- `PlexiEvent::Init` calls `init((width, height), args)` and applies returned effects.
- `PlexiEvent::Render` dispatches `RenderFrame` through `update(event)`, then calls `view()` and emits a `component_tree` draw command.
- Input events decode to typed SDK events such as `KeyEvent`, `UiAction`, `UiValueChange`, `FileListResult`, and `FileReadResult`.
- Effects encode back to existing PGAP host/control commands, including `set_state`, `save_app_state`, `set_title`, `file_list`, `file_read`, `http_request`, `open_url`, and `set_scheduler_mode`.

### File effects

`FileList(path, extensions=[])` and `FileRead(path)` are native ProcessApp host requests. The host requires `fs.read`, resolves the requested path inside `workspace_root`, and returns `FileListResult(entries, error)` or `FileReadResult(content, error)`. Directory listings are sorted with directories first, then files by name.

### URL effects

`OpenUrl(url)` is a native ProcessApp host request for opening HTTP(S) URLs in the user's default browser. The host requires `net.http`, rejects non-HTTP(S) URLs, and applies the manifest `allowed_hosts` matcher before spawning the platform opener.

### Deferred WASM Python boundary

CPython-in-WASM remains deferred G8 runtime work. Do not add `[runtime] python_compat = true` for SDK v3 apps, do not route Python app manifests to a `WasmPythonAdapter`, and do not require CPython bundle or shim fixtures for this SDK contract.

---

## 8. Manifest Schema

Current Python SDK v3 apps use the existing app manifest shape:

```toml
schema_version = 1

[app]
id = "my_app"
type = "app"
name = "My App"
version = "0.1.0"
description = "One sentence."
entry = "main.py"

[app.capabilities]
capabilities = []

[launch]
```

`[app] type = "app"` plus a `.py` entry launches through native `ProcessApp`. `[app] type = "wasm"` is the separate component-model WASM runtime. SDK v3 is the current Python app API; it is not a WASM execution mode.

---

## 9. WASM-Contained Python Status

Deferred. A future G8 may add a CPython-in-WASM compatibility layer, bundle management, and manifest routing. That work is outside this SDK v3 native landing and must not be advertised as shipped.

---

## 10. Hot Reload

Dev mode is the existing watched ProcessApp subprocess flow. On Python source changes, the host restarts the app subprocess and reinjects persisted/runtime state where supported by the ProcessApp lifecycle. There is no wasmtime store reset or CPython bundle reload in the current SDK v3 path.

---

## 11. Scaffold Template (`plexi app init <name>`)

`src/cli/app.rs` — `init` subcommand for Python apps generates a normal native app manifest and SDK v3 entry point.

**`<name>/manifest.toml`:**
```toml
schema_version = 1

[app]
id = "{name}"
type = "app"
name = "{Name}"
entry = "main.py"
version = "0.1.0"
description = "A Plexi app"
watch = true

[app.capabilities]
capabilities = []

[launch]
```

**`<name>/main.py`:**
```python
from plexi_sdk import state, log
from plexi_sdk.effects import SetTitle, SetState
from plexi_sdk.events import KeyEvent
from plexi_sdk.ui import Column, AppBar, Text, FooterKeys


def init(size, args):
    log.info("app initialized")
    return [SetTitle("{Name}"), SetState({"count": 0})]


def update(event):
    if isinstance(event, KeyEvent) and event.key == "return" and event.pressed:
        return [state.set("count", state.get("count", 0) + 1)]
    return []


def view():
    return Column([
        AppBar("{Name}"),
        Text(str(state.get("count", 0)), bold=True),
        FooterKeys([("return", "increment")]),
    ], grow=True)
```

---

## 12. SDK Package Layout

`sdk/python/plexi_sdk/` is the SDK v3 package used by native ProcessApp apps:

```
plexi_sdk/
  __init__.py        # exports state, log, sizing helpers, and public SDK symbols
  effects.py         # Effect dataclasses (section 4)
  events.py          # Event dataclasses (section 5)
  ui.py              # UINode dataclasses and layout primitives (section 6)
  _v3_process.py     # native ProcessApp entry point
  _v3_runtime.py     # PGAP event/effect runtime
  _v3_state.py       # StateProxy and StateSnapshot implementation
  _adapter.py        # test/helper encode/decode surface
```

**Superseded legacy files to delete** (not rename, not keep):
- `plexi_sdk/_pipe.py`
- `plexi_sdk/_emitter.py`
- `plexi_sdk/_render_context.py`
- `plexi_sdk/_app.py`
- `plexi_sdk/_state.py`
- legacy widget modules replaced by `ui.py` primitives

**`pyproject.toml` version:** `3.0.0`. Python requirement remains repo-standard `>=3.11`.

---

## 13. Runtime Boundary

Do not delete `src/process_app/`. SDK v3 Python apps still run through native `ProcessApp`; PGAP is the current transport for Python apps.

The WASM runtime remains available for `[app] type = "wasm"` component-model apps. CPython-in-WASM remains deferred G8 work and must not be described as shipped by SDK v3.

---

## 14. Tests Required

### SDK unit tests (`sdk/python/tests/`)

- `test_v3_adapter.py`: instantiate effects/events/UI nodes and assert encoded wire shape.
- `test_v3_runtime_regression.py`: run runtime event/effect regressions, including state, key normalization, sizing, canvas, and file results.

### Host unit tests

- `src/process_app/routing.rs`: native `FileList` / `FileRead` enforce `fs.read` and workspace scoping.
- `src/process_app/mod.rs`: regression coverage proves Python SDK v3 apps launch through `ProcessApp`, not a WASM adapter.
- `src/protocol/events.rs` and `src/protocol/commands.rs`: serde round-trips for new event/request fields.

### App tests

Each touched core app has a focused SDK v3 test under `apps/<app>/tests/`. Canvas/game apps must prove size-aware canvas behavior; file apps must prove `FileList` / `FileRead` effects; keyboard apps must use normalized lowercase key strings.
