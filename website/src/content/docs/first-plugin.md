---
title: Build your first plugin
description: Scaffold a Python plugin, check it, open its view in a pane, and change it while it runs.
order: 3
---

A plugin is a folder with a manifest and some Python. Its view renders in a
pane next to your terminals. This page builds a counter. Every step is a CLI
command, so you can hand the whole page to your agent instead.

These commands are the 0.5.0 names. On a release that still has `plexi app`,
the steps are the same under that namespace.

## 1. Scaffold

From inside a project you have open in Plexi:

```sh
plexi plugin init counter
```

Outside a workspace, add `--global`. The scaffold writes:

```text
counter/
  manifest.toml         identity, runtime, views, capabilities
  main.py               the plugin: init, update, view
  tests/                a PluginHarness smoke test
  AGENTS.md             notes for coding agents working on the plugin
  plexi.scaffold.toml   which CLI and SDK versions wrote the scaffold
```

Always start from `plexi plugin init`. It writes a manifest with the right
schema version, and `plexi plugin check` uses `plexi.scaffold.toml` to tell you
when the plugin has drifted from the SDK you are running.

## 2. Write the plugin

A plugin is three module-level functions. Replace `main.py` with:

```python
#!/usr/bin/env python3
from __future__ import annotations

from plexi_sdk import log, state
from plexi_sdk.effects import SetState, SetTitle
from plexi_sdk.events import KeyEvent
from plexi_sdk.ui import Column, FooterKeys, SurfaceHeader, Text


def init(size, args):
    log.info("counter: init")
    return [SetTitle("Counter"), SetState({"count": 0})]


def update(event):
    if isinstance(event, KeyEvent) and event.key == "+" and event.pressed:
        return [SetState({"count": state.get("count", 0) + 1})]
    return []


def view():
    return Column([
        SurfaceHeader("Counter"),
        Text(str(state.get("count", 0)), bold=True, align="center"),
        FooterKeys([("+", "increment")]),
    ], grow=True)
```

`init` runs once and returns startup effects. `update` runs for every input
event and returns effects. `view` returns the component tree and must be pure:
calling `state.set()` inside it raises `RuntimeError`. You describe structure
and the host draws it, so the plugin picks up your theme and fonts without any
pixel math.

## 3. Check it

```sh
plexi plugin check counter --png-dir /tmp/counter-shots
```

The check validates the manifest and its capability IDs, inspects your SDK
usage without importing the code, and renders the view at small and normal pane
sizes. The PNGs land in the directory you named, so you (or your agent) can
look at the result before opening anything.

## 4. Open the view

```sh
plexi view open ./counter
```

The view opens in a split to the right of the pane you ran it from. Press `+`.

`plexi view open` takes a path or a plugin ID. Direction flags (`--down`,
`--left`, `--up`), `--tab` and `--window` change where it lands.

## 5. Change it while it runs

New plugins set `watch = true` in their manifest. Edit `main.py`, save, and the
same pane updates without reopening.

Run the scaffolded tests whenever you change behavior:

```sh
plexi plugin test counter
```

## 6. Ask for a capability

The counter touches nothing outside itself, so its capability list is empty.
A plugin that needs the network or your files has to say so in `manifest.toml`:

```toml
[plugin.capabilities]
capabilities = ["net.http"]
```

Declaring a capability requests it. It grants nothing until you approve it.
[Plugins and views](/docs/plugins-and-views) explains the model, and the
[plugin manifest](/docs/plugin-manifest#capability-registry) lists every ID.

## Where next

- [Plugin guide](/docs/plugins): the sandbox, capabilities and logging in more depth.
- [Python SDK](/docs/sdk): every effect, event and UI component.
- The plugins that ship with Plexi are built on this same public API. Their source is in the [`apps/` directory](https://github.com/ianjamesburke/PLEXI/tree/alpha/apps) of the repository.
