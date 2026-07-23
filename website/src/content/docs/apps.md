---
title: Apps
description: Build and run Plexi apps.
order: 5
---

Plexi apps render into panes through PGAP. The Python SDK is the default authoring path.

Python apps are sandboxed, not native subprocesses: they run through the CPython-in-WASM adapter inside their own `wasmtime::Store`, the same component boundary a Rust WASM app gets. Capabilities gate host APIs such as `net.http`, `secrets.get`, and `ai.query` on top of that sandbox.

## Create an App

From inside a Plexi workspace:

```sh
plexi app init my-app
```

Outside a workspace, use `--global`.

This scaffolds a new app folder with:

```text
my-app/
  manifest.toml    ← capabilities, entry point, metadata
  main.py          ← your app code
  tests/test_app.py ← AppHarness smoke tests
  .venv/           ← isolated Python runtime
```

Validate the app and write screenshots:

```sh
plexi app check ./my-app --png-dir /tmp/my-app-shots
```

## Open an App

Pass the path to the app folder:

```sh
plexi app open ./my-app
```

The app opens in a pane. Use this during development; no marketplace install is required.

## The App Pattern

SDK v3 apps are module-level `init`, `update`, and `view` functions. `view()` returns a component tree and must stay pure:

```python
#!/usr/bin/env python3
from __future__ import annotations

from plexi_sdk import state
from plexi_sdk.effects import SetState, SetTitle
from plexi_sdk.events import KeyEvent
from plexi_sdk.ui import AppBar, Column, FooterKeys, Text


def init(size, args):
    return [SetTitle("My App"), SetState({"count": 0})]


def update(event):
    if isinstance(event, KeyEvent) and event.key == "return" and event.pressed:
        return [SetState({"count": state.get("count", 0) + 1})]
    return []


def view():
    return Column([
        AppBar("My App"),
        Text(str(state.get("count", 0)), bold=True),
        FooterKeys([("return", "increment")]),
    ], grow=True)
```

Canvas apps also return from `view()`: use `Canvas(...)` components and `RenderFrame` events for animation.

## Capabilities

Declare what your app needs in `manifest.toml`:

```toml
[app.capabilities]
capabilities = ["secrets.get", "net.http"]
```

Common capabilities: `secrets.get`, `net.http`, `fs.read`, `fs.write`, `ai.query`, `audio.record`, `timer`.

## Logging

Use `plexi_sdk.log` from `init()` or `update()`. Log lines are tagged with the app process in the host log.

```python
from plexi_sdk import log


def init(size, args):
    log.info("my-app initialized")
    return []
```

See also: [PGAP](/docs/pgap), [Secrets](/docs/secrets)
