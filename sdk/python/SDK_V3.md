# Plexi Python SDK v3 — Design Specification

**Status:** authoritative design doc for task 0285. Implement exactly this. No design decisions during implementation.

Source of truth for all types: `wit/plexi.wit`. Every Python type here maps to a named WIT type. Where a mapping decision was made, it is stated explicitly.

---

## 1. App Contract

A Python app is three module-level functions. No class, no inheritance.

```python
from plexi_sdk import state, log
from plexi_sdk.effects import SetTitle, SetTimer
from plexi_sdk.events import KeyEvent, TimerFired
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
- `state` in `plexi_sdk` is a module-level proxy that reads `plexi_sdk._state`.
- The adapter flattens the `UINode` tree returned by `view()` into a WIT `ui-tree` arena.

**Naming:** functions must be named exactly `init`, `update`, `view` at module level. The adapter looks them up by name via `getattr(module, "init")` etc.

---

## 2. State Access

```python
# plexi_sdk/__init__.py exposes:
from plexi_sdk import state   # StateProxy instance

state.get(key: str, default=None) -> any       # JSON-decode value, return default if absent
state.set(key: str, value: any) -> SetState    # returns a SetState effect (does NOT mutate immediately)
state.all() -> dict[str, any]                  # all keys decoded
state.raw(key: str) -> bytes | None            # raw bytes, no decode
```

**`state.set()` returns an effect, it does not mutate.** The app returns `[state.set("count", 5)]` from `update()`. The adapter executes it via `host-state::set`, then rebuilds the snapshot and calls `view()`.

**`SetState` effect:** convenience shorthand. Equivalent to returning a list of `HostStateSet` effects. Implemented as:

```python
@dataclass
class SetState:
    """Set one or more state keys. Values are JSON-encoded by the adapter."""
    data: dict  # {key: value} — values must be JSON-serializable
```

The adapter serializes each value as JSON bytes and calls `host-state::set(key, json_bytes)` for each entry.

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
    data: dict  # {key: any} — JSON-encoded per key by adapter

# ── File I/O ──────────────────────────────────────────────────────────────────

@dataclass
class FileRead:
    path: str   # → file-read-effect { path }

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
class TextInput:
    value: str
    placeholder: str = ""
    on_change: str = ""     # handler_id — matched by UiValueChange event
    on_submit: str = ""     # handler_id — matched by UiAction event
    password: bool = False
    # → ui-node-data::text-input(text-input-node)

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

## 7. Adapter Bridge Protocol

The CPython-in-WASM adapter (`src/host/wasm_python.rs`) drives Python via the CPython C API exposed inside WASM. The bridge between Rust and Python uses JSON over the WASM linear memory. This is the exact protocol — implement exactly this.

### Bootstrap sequence

1. Adapter loads `app/main.py` (or the entry file from `manifest.toml`) by calling CPython's `PyRun_SimpleFile`.
2. Adapter imports `plexi_sdk._adapter` (internal module pre-loaded into CPython).
3. All lifecycle calls go through `plexi_sdk._adapter.call_lifecycle(fn_name, json_arg) -> str`.

### `_adapter.py` interface (internal, not app-facing)

```python
# plexi_sdk/_adapter.py — called by Rust adapter, not by app code

import json
import sys

_module = None  # the loaded app module

def load_app(module_name: str):
    global _module
    _module = __import__(module_name)

def call_lifecycle(fn_name: str, json_arg: str) -> str:
    """
    fn_name: "init" | "update" | "view"
    json_arg: JSON-encoded argument (see below)
    returns: JSON-encoded return value
    """
    fn = getattr(_module, fn_name)
    arg = json.loads(json_arg) if json_arg else None

    if fn_name == "init":
        # arg: {"state": {key: value_b64}, "size": [w, h], "args": [...]}
        import plexi_sdk as sdk
        sdk._state = _decode_state(arg["state"])
        sdk._in_view = False
        result = fn((arg["size"][0], arg["size"][1]), arg["args"])
        return json.dumps([_encode_effect(e) for e in result])

    elif fn_name == "update":
        # arg: {"type": "KeyEvent", "key": "a", ...}
        import plexi_sdk as sdk
        sdk._state = _decode_state(arg["state"])
        sdk._in_view = False
        event = _decode_event(arg["event"])
        result = fn(event)
        return json.dumps([_encode_effect(e) for e in result])

    elif fn_name == "view":
        # arg: {"state": {key: value_b64}}
        import plexi_sdk as sdk
        sdk._state = _decode_state(arg["state"])
        sdk._in_view = True
        tree = fn()
        sdk._in_view = False
        return json.dumps(_encode_uitree(tree))
```

**JSON encoding for state values:** values are base64-encoded JSON bytes. `_decode_state` calls `json.loads(base64.b64decode(v))` for each entry. State keys that are not valid JSON (raw bytes) are passed through as base64 strings with a `"b64:"` prefix.

**JSON encoding for effects:** each effect encodes as `{"type": "ClassName", ...fields}`. Example: `SetTitle(title="hello")` → `{"type": "SetTitle", "title": "hello"}`.

**JSON encoding for UINode:** the flattened arena as `{"root": 0, "nodes": [{"id": 0, "key": "0", "data": {"type": "Column", "children": [1,2], "gap": 0.0, ...}}, ...]}`.

**JSON encoding for events:** each event decodes from `{"type": "ClassName", ...fields}`. The Rust adapter serializes the WIT `input-event` to this JSON before calling Python.

The Rust side reads the returned JSON string from WASM linear memory via a shared buffer. Implementation detail for `wasm_python.rs`: pass JSON in/out via a pre-allocated 4MB linear memory buffer. Write JSON to offset 0, call Python function, read JSON response from offset 0 of the response buffer.

---

## 8. Manifest Schema

`manifest.toml` for Python apps:

```toml
schema_version = "2"
id = "com.publisher.app-name"
version = "1.0.0"
name = "App Name"
description = "One sentence."
publisher = "publisher-name"

[runtime]
entry = "main.py"            # entry file; adapter does __import__(stem)
python_compat = true         # required for Python apps

[capabilities]
required = []
optional = []
```

`[runtime] python_compat = true` is the sole flag that routes to `WasmPythonAdapter` instead of `LiveWasmPane`. No other manifest change needed.

`entry` field: defaults to `"main.py"` if absent. The adapter strips the `.py` suffix and calls `__import__` on the stem. Multi-module apps: the entry module imports helpers from the same directory; WASI preopens mount the app dir as `"."`.

---

## 9. CPython WASM Bundle

**Source:** `https://github.com/brettcannon/cpython-wasi-build/releases` — use the latest `cpython-3.12.x-wasm32-wasip1` release asset named `python-3.12.x-wasm32-wasip1.zip`. If that release is unavailable, fall back to building CPython from source: `./configure --host=wasm32-wasip1 CC=clang --disable-test-modules --prefix=/opt/wasm` then `make install`.

**Version pin:** `CPYTHON_BUNDLE_VERSION = "3.12.3"` as a constant in `src/host/wasm_python.rs`. Hash is SHA256 of the `.wasm` file, hardcoded alongside the version.

**Cache path:** `~/.plexi/wasm-bundles/cpython-3.12.wasm`. The adapter checks this path on first use. If absent or SHA256 mismatch: download from the releases URL to a temp file, verify SHA256, move to cache path. If download fails: panic with `"CPython WASM bundle unavailable — run: just fetch-cpython-bundle"`.

**Bundle wrapping:** if the upstream asset is a raw WASM module (not a Component), wrap it: `wasm-tools component new python.wasm --adapt wasi_snapshot_preview1=wasi_preview1_component_adapter.wasm -o cpython-3.12.wasm`. The `wasi_preview1_component_adapter.wasm` is from `bytecodealliance/wasmtime` releases. Both the adapted bundle and the adapter binary are cached locally.

**`just fetch-cpython-bundle`:** shell recipe that downloads, verifies, and wraps. Not called at build time — called explicitly by the developer or CI.

---

## 10. Hot Reload

Dev mode only (`plexi app dev <path>`). Not active for registry-installed apps.

1. Host watches `<app-dir>/**/*.py` for `inotify`/`kqueue` events.
2. On any change: capture the current state snapshot from the `WasmPythonAdapter` (`adapter.snapshot()`).
3. Tear down the existing `wasmtime::Store` (drop it — microseconds).
4. Create a new `Store`, re-mount the app dir via WASI preopens.
5. Call `init(snapshot, last_size, last_args)` — passes previous state so in-progress work is preserved.
6. `view()` called → host repaints.
7. User sees the update. Total latency: WASM instance reset (~1ms) + `init` + `view` (~10ms typical).

`file-watch` effect (WIT stub, implement in `src/host/wasm_app.rs`): WASM apps can also self-declare a file-watch. For Python dev mode, the host initiates the watch without the app asking — the app does not need to emit a `file-watch` effect.

---

## 11. Scaffold Template (`plexi app init <name>`)

`src/cli/app.rs` — `init` subcommand for Python apps generates exactly:

**`<name>/manifest.toml`:**
```toml
schema_version = "2"
id = "com.youname.{name}"
version = "0.1.0"
name = "{Name}"
description = "A Plexi app."
publisher = "yourname"

[runtime]
entry = "main.py"
python_compat = true

[capabilities]
required = []
optional = []
```

**`<name>/main.py`:**
```python
from plexi_sdk import state, log
from plexi_sdk.effects import SetTitle, SetState
from plexi_sdk.events import KeyEvent
from plexi_sdk.ui import Column, AppBar, Text, FooterKeys


def init(size, args):
    return [
        SetTitle("{Name}"),
        SetState({"count": 0}),
    ]


def update(event):
    if isinstance(event, KeyEvent) and event.key == "plus" and event.pressed:
        return [SetState({"count": state.get("count", 0) + 1})]
    if isinstance(event, KeyEvent) and event.key == "minus" and event.pressed:
        return [SetState({"count": state.get("count", 0) - 1})]
    return []


def view():
    count = state.get("count", 0)
    return Column([
        AppBar("{Name}"),
        Text(str(count), bold=True, align="center"),
        FooterKeys([("+", "increment"), ("-", "decrement")]),
    ], grow=True)
```

**`<name>/pyproject.toml`:**
```toml
[project]
name = "{name}"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = ["plexi-sdk>=3.0"]
```

No other files. No `__init__.py`. No `requirements.txt`.

---

## 12. SDK Package Layout

`sdk/python/plexi_sdk/` replaces the existing package entirely:

```
plexi_sdk/
  __init__.py        # exports: state (StateProxy), log (LogProxy), _state, _in_view
  effects.py         # all Effect dataclasses (section 4)
  events.py          # all Event dataclasses (section 5)
  ui/
    __init__.py      # all UINode dataclasses + AppBar/FooterKeys/Section helpers (section 6)
  _adapter.py        # internal: call_lifecycle, load_app, encode/decode helpers
  _state.py          # StateProxy, StateSnapshot implementation
  _log.py            # LogProxy implementation (calls host-log via ctypes/WASM import)
```

**Files to delete** (not rename, not keep — delete entirely):
- `plexi_sdk/_pipe.py`
- `plexi_sdk/_protocol.py`
- `plexi_sdk/_emitter.py`
- `plexi_sdk/_render_context.py`
- `plexi_sdk/_constants.py` (constants move inline to `ui/__init__.py` where needed)
- `plexi_sdk/_types.py` (replaced by `effects.py` + `events.py`)

**`pyproject.toml` version:** bump to `3.0.0`. Add `python_requires = ">=3.12"`.

---

## 13. PGAP Deletion

After all Core apps pass under the new runtime, delete:

- `src/process_app/` — entire directory
- All `AppRuntime::Process` / `AppRuntime::ProcessApp` enum variants and match arms
- `src/host/pgap.rs` or equivalent PGAP parser
- Subprocess spawn code (search for `std::process::Command` in `src/` — anything spawning a Python process)
- The `PackageRuntime::Native` variant if it was only used to gate subprocess Python execution (it maps to `NativeUnreviewed` trust label — if no native runtime exists post-deletion, remove the variant and the label)

Run `cargo build` after deletion. All dead code errors are additional deletions. Do not use `#[allow(dead_code)]`.

---

## 14. Tests Required

### SDK unit tests (`sdk/python/tests/`)

- `test_effects.py`: instantiate every Effect class, call `_encode_effect(e)`, assert JSON shape matches expected.
- `test_events.py`: for each Event class, call `_decode_event(json_dict)`, assert correct Python type and field values.
- `test_ui.py`: instantiate `Column([Text("hi"), Button("x", "click")])`, call `_encode_uitree(node)`, assert arena has 3 nodes with correct IDs and types.
- `test_state.py`: `StateProxy.get()` with present key, absent key, default value. `state.set()` returns `SetState`. `state.set()` inside `view()` context raises `RuntimeError`.

### Host unit tests (`src/host/wasm_python.rs #[cfg(test)]`)

Load `tests/fixtures/apps/hello_wasm_python/main.py`:
```python
from plexi_sdk.effects import SetTitle
from plexi_sdk.events import KeyEvent
from plexi_sdk.ui import Text

def init(size, args):
    return [SetTitle("hello")]

def update(event):
    return []

def view():
    return Text("ok")
```

Test: `init` returns WIT `list<effect>` with one `set-title("hello")`. `update(key-event{key:"q"})` returns empty list. `view()` returns `ui-tree` with one `text-node{text:"ok"}`.

### G8 gate

`apps/stats/stats.py` rewritten against v3 SDK, launches under `WasmPythonAdapter`, screenshot of the activity clock matches reference. Test is a `#[test]` in `src/ui_tests.rs` that calls `PlexiUiHarness`, opens the stats app, waits for `TimerFired`, screenshots, asserts non-blank canvas region.
