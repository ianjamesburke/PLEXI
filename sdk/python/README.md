# plexi-sdk

Python SDK v3 for building Plexi apps running inside the CPython-in-WASM runtime.

Full design spec: [`SDK_V3.md`](SDK_V3.md)

## Install For SDK Development

```sh
uv pip install -e ./sdk/python
```

Changes to `sdk/python/plexi_sdk/` take effect immediately.

## App Pattern

An app is three module-level functions:

```python
from plexi_sdk import state
from plexi_sdk.effects import SetTitle, SetState
from plexi_sdk.events import KeyEvent
from plexi_sdk.ui import Column, AppBar, Text, FooterKeys


def init(size, args):
    return [SetTitle("Counter"), SetState({"count": 0})]


def update(event):
    if isinstance(event, KeyEvent) and event.key == "plus" and event.pressed:
        return [SetState({"count": state.get("count", 0) + 1})]
    return []


def view():
    return Column([
        AppBar("Counter"),
        Text(str(state.get("count", 0)), bold=True, align="center"),
        FooterKeys([("+", "increment")]),
    ], grow=True)
```

## Keyboard Conventions

Keys arrive in `update(event)` as `KeyEvent`. Key strings are lowercase canonical.

| Physical key | `event.key` |
|---|---|
| Enter / Return | `"return"` |
| Escape | `"escape"` |
| Backspace | `"backspace"` |
| Space | `"space"` |
| Arrow keys | `"up"` / `"down"` / `"left"` / `"right"` |

## New App

```sh
plexi app init myapp
```

Generates `myapp/main.py` + `myapp/manifest.toml` with `python_compat = true`.
