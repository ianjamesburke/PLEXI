---
title: Apps
description: Build and run Plexi apps.
order: 5
---

Plexi apps render into panes through PGAP. The Python SDK is the default authoring path.

## Execution and permissions

Python apps opened in panes run in CPython-in-WASM, inside a Wasmtime store.
The host routes `.py` and `.pyc` entries to this adapter and does not fall back
to a native subprocess when the WASM bundle is missing. Rust WASM components
use the component runtime; both paths gate host APIs through capabilities.

The Python guest receives read-only mounts for its app directory, the SDK,
and the Python standard library. Host filesystem access, network requests,
and secrets go through the host APIs and their grants. Declaring a capability
in the manifest requests access; it does not grant access by itself.

Native Python still runs during development tooling, such as type checks and
SDK tests executed with a local interpreter. Those processes have the permissions
of the user who starts them; the pane sandbox does not cover them. A terminal
pane running `python3` is also an ordinary native process.

The routing and sandbox setup are defined by
[`open_python_wasm_app_pane` and `build_python_runtime`](https://github.com/ianjamesburke/PLEXI/blob/alpha/src/pane_ops/create.rs)
and [`WasmPythonRuntime::launch`](https://github.com/ianjamesburke/PLEXI/blob/alpha/src/host/wasm_python.rs).
The [app checker](https://github.com/ianjamesburke/PLEXI/blob/alpha/src/cli/app_check.rs)
uses WASM for render checks and native tooling for static analysis.

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
  .venv/           ← local development environment
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

The app opens in a pane. Local app authoring and `plexi app` are part of the
stable v1 surface; a hosted marketplace is not.

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
