---
title: Plugins
description: Build and run Plexi plugins.
order: 5
---

Plexi plugins render into panes through PGAP. The Python SDK is the default authoring path.

## Execution and permissions

Python plugins opened in panes run in CPython-in-WASM, inside a Wasmtime store.
The host routes `.py` and `.pyc` entries to this adapter and does not fall back
to a native subprocess when the WASM bundle is missing. Rust WASM components
use the component runtime; both paths gate host APIs through capabilities.

The Python guest receives read-only mounts for its plugin directory, the SDK,
and the Python standard library. Host filesystem access, network requests,
and secrets go through the host APIs and their grants. Declaring a capability
in the manifest requests access; it does not grant access by itself.

Native Python still runs during development tooling, such as type checks and
SDK tests executed with a local interpreter. Those processes have the permissions
of the user who starts them; the pane sandbox does not cover them. A terminal
pane running `python3` is also an ordinary native process.

The routing and sandbox setup are defined by
[`open_python_wasm_view_pane` and `build_python_runtime`](https://github.com/ianjamesburke/PLEXI/blob/alpha/src/pane_ops/create.rs)
and [`WasmPythonRuntime::launch`](https://github.com/ianjamesburke/PLEXI/blob/alpha/src/host/wasm_python.rs).
The [plugin checker](https://github.com/ianjamesburke/PLEXI/blob/alpha/src/cli/plugin_check.rs)
uses WASM for render checks and native tooling for static analysis.

## Create a plugin

From inside a Plexi workspace:

```sh
plexi plugin init my-plugin
```

Outside a workspace, use `--global`.

This scaffolds a new plugin folder with:

```text
my-plugin/
  manifest.toml    ← capabilities, entry point, metadata
  main.py          ← entry point
  plugin/          ← your plugin callbacks
  tests/test_plugin.py ← PluginHarness smoke tests
  .venv/           ← local development environment
```

Validate the plugin and write screenshots:

```sh
plexi plugin check ./my-plugin --png-dir /tmp/my-plugin-shots
```

## Open a view

Pass the path to the plugin folder:

```sh
plexi view open ./my-plugin
```

The view opens in a pane. `plexi view list --json` lists declared views. A plugin ID opens its only view or its explicitly declared default; multiple unmarked views return the available choices. A plugin with no views cannot be opened.

The retained visualizers open with `plexi view open cli-renderer -- git` and `plexi view open mcp-renderer -- filesystem`; `--cli git` and `--mcp filesystem` select those views directly. Their existing experiment gates still apply. `plexi agents-dashboard` opens the Agents dashboard when its experiment is enabled. Agents remain separate from plugins.

These names describe the 0.5.0 checkpoint source, not a deployed marketplace.

## The plugin pattern

SDK v3 plugins are module-level `init`, `update`, and `view` functions. `view()` returns a component tree and must stay pure:

```python
#!/usr/bin/env python3
from __future__ import annotations

from plexi_sdk import state
from plexi_sdk.effects import SetState, SetTitle
from plexi_sdk.events import KeyEvent
from plexi_sdk.ui import SurfaceHeader, Column, FooterKeys, Text


def init(size, args):
    return [SetTitle("My plugin"), SetState({"count": 0})]


def update(event):
    if isinstance(event, KeyEvent) and event.key == "return" and event.pressed:
        return [SetState({"count": state.get("count", 0) + 1})]
    return []


def view():
    return Column([
        SurfaceHeader("My plugin"),
        Text(str(state.get("count", 0)), bold=True),
        FooterKeys([("return", "increment")]),
    ], grow=True)
```

Canvas plugins also return from `view()`: use `Canvas(...)` components and `RenderFrame` events for animation.

## Capabilities

Declare what your plugin needs in `manifest.toml`:

```toml
[plugin.capabilities]
capabilities = ["secrets.get", "net.http"]
```

Common capabilities: `secrets.get`, `net.http`, `fs.read`, `fs.write`, `ai.query`, `audio.record`, `timer`.

## Logging

Use `plexi_sdk.log` from `init()` or `update()`:

```python
from plexi_sdk import log


def init(size, args):
    log.info("my-plugin initialized")
    return []
```

On macOS and Linux the channel log is `~/.plexi-<channel>/plexi.log`; stable uses `~/.plexi/plexi.log`. On Windows it is `plexi.log` in the corresponding `.plexi-<channel>` directory under the user's home directory. Plexi rolls the live file by day or size. Dated archives are named `plexi-YYYY-MM-DD.log` (with a numeric suffix when needed), and `[log] retention_days` defaults to 30.

Set `[log] level` in the channel's `config.toml` to `error`, `warn`, `info`, `debug`, or `trace`. A config reload applies the level to the running host. Third-party targets stay at warning level. Logs can contain anything a plugin explicitly writes, so never put secrets or user input in a plugin log or stderr.

SDK log messages appear with an `plugin::<id>:` message prefix and the host runtime's log target. Guest stderr lines carry an `plugin::<id> CPython WASM stderr` prefix; traceback lines are recorded at error level. Open the Logs plugin to tail, filter, and search the channel log. Its `logs.query` assistant tool returns matching lines, and `logs.set_filter` changes the displayed level.

`plexi debug session list` lists recorded sessions; `plexi debug session show latest` reads the latest session, even after the host exits. Use `--pane <id>` to narrow the view and `--cli` for CLI calls. Session records live under the channel profile's `sessions/` directory. CLI content arguments are replaced by length and digest placeholders, and secret values and environment assignments are redacted. Tool inputs and outputs and plugin event content are recorded as lengths, while routing and session provenance remain available. `--join-agents` also reads external agent transcripts, which have their own content and privacy rules.

To record CLI content deliberately, set `PLEXI_TRACE_CAPTURE_CONTENT=1` in both the host and the calling CLI process. Secret-named options and environment assignments remain redacted, but other content may contain sensitive text. Invalid CLI calls stay redacted. Existing trace files are not rewritten when this setting changes.

See also: [PGAP](/docs/pgap), [Secrets](/docs/secrets)
