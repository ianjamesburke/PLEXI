---
title: Plugin guide
description: How a Plexi plugin is written, where it runs, what it may touch, and how to read its logs.
order: 5
---

A plugin renders its views into panes through [PGAP](/docs/pgap), the protocol
between a plugin and the host. The Python SDK is the default way to write one.
If you have not built one yet, start with
[Build your first plugin](/docs/first-plugin).

## The plugin pattern

An SDK v3 plugin is three module-level functions: `init`, `update` and `view`.
There is no class to inherit from.

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

- `init(size, args)` runs once on launch and returns startup effects.
- `update(event)` runs for every input event and returns effects. Effects are data; the host applies them after `update` returns.
- `view()` returns what to draw and must stay pure. All mutation goes through effects.

Pick one rendering mode per plugin. Forms, lists and dashboards return a
component tree (`Column`, `HStack`, `Text`, `Button`, `TextInput`). Games and
visualizations return a `Canvas([...])` and advance their state from
`RenderFrame` events. The [Python SDK reference](/docs/sdk) lists every
component, effect and event.

## The working loop

```sh
plexi plugin init my-plugin                              # scaffold
plexi plugin check ./my-plugin --png-dir /tmp/shots      # validate and render
plexi view open ./my-plugin                              # open the view in a pane
plexi plugin test ./my-plugin                            # run its PluginHarness tests
```

`plexi plugin check` is the gate: it validates the manifest, inspects SDK usage
without importing your code, and renders the view at small and normal pane
sizes. There is no build step.

Saving a source file reloads the view in the same pane. This is always on; the
manifest `watch` key has no effect.

`plexi view render ./my-plugin --png --output shot.png` renders a view without
a host window, which is how an agent looks at its own work.

`plexi_sdk` is importable only inside Plexi's plugin runtime. A bare
`python3 -c "import plexi_sdk"` in a terminal fails or finds a stale copy. Test
by opening the view or running `plexi plugin test`.

## Execution and permissions

Python plugins opened in panes run in CPython-in-WASM, inside a Wasmtime store.
The host routes `.py` and `.pyc` entries to this adapter and does not fall back
to a native subprocess when the WASM bundle is missing. Rust WASM components
use the component runtime. Both paths gate host APIs through capabilities.

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

## Capabilities

Declare what your plugin needs in `manifest.toml`:

```toml
[plugin.capabilities]
capabilities = ["secrets.get", "net.http"]
```

Unknown IDs fail before the plugin starts. The
[capability registry](/docs/plugin-manifest#capability-registry) lists every
accepted ID and whether the host serves it yet. `plexi plugin inspect ./my-plugin`
prints the trust sheet a person sees before installing your plugin.

## Wrapping a CLI or an MCP server

Two built-in views wrap tools that have no Plexi UI of their own:

```sh
plexi view open --cli git
plexi view open --mcp filesystem
```

Both are behind experiments. `plexi experiments list` shows their state.
`plexi agents-dashboard` opens the Agents dashboard when its experiment is
enabled. Agents are separate from plugins.

## Logging

Use `plexi_sdk.log` from `init()` or `update()`:

```python
from plexi_sdk import log


def init(size, args):
    log.info("my-plugin initialized")
    return []
```

On macOS the channel log is `~/.plexi-<channel>/plexi.log`; stable uses
`~/.plexi/plexi.log`. Plexi rolls the live file by day or size. Dated archives
are named `plexi-YYYY-MM-DD.log` (with a numeric suffix when needed), and
`[log] retention_days` defaults to 30.

Set `[log] level` in the channel's `config.toml` to `error`, `warn`, `info`,
`debug`, or `trace`. A config reload applies the level to the running host.
Third-party targets stay at warning level. Logs can contain anything a plugin
explicitly writes, so never put secrets or user input in a plugin log or stderr.

SDK log messages appear with a `plugin::<id>:` message prefix and the host
runtime's log target. Guest stderr lines carry a `plugin::<id> CPython WASM stderr`
prefix; traceback lines are recorded at error level. Open the Logs plugin
(`plexi view open logs`) to tail, filter, and search the channel log.

## Session records

`plexi debug session list` lists recorded sessions, and
`plexi debug session show latest` reads the latest one, even after the host
exits. Use `--pane <id>` to narrow the view and `--cli` for CLI calls.

CLI content arguments are replaced by length and digest placeholders, and
secret values and environment assignments are redacted. Tool inputs and
outputs and plugin event content are recorded as lengths, while routing and
session provenance remain available. `--join-agents` also reads external agent
transcripts, which have their own content and privacy rules.

To record CLI content deliberately, set `PLEXI_TRACE_CAPTURE_CONTENT=1` in both
the host and the calling CLI process. Secret-named options and environment
assignments remain redacted, but other content may contain sensitive text.
Invalid CLI calls stay redacted. Existing trace files are not rewritten when
this setting changes.
