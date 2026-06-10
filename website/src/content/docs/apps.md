---
title: Apps
description: Build and run Plexi apps.
verified_version: "0.0.689"
order: 5
---

Plexi apps render into panes through PGAP. The Python SDK is the default authoring path.

Python apps are native subprocesses. Capabilities gate host APIs such as `net.http`, `secrets.get`, and `ai.query`; they are not a Python process sandbox.

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
```

## Open an App

Pass the path to the app folder:

```sh
plexi app open ./my-app
```

The app opens in a pane. Use this during development; no marketplace install is required.

## The App Pattern

Normal apps implement `view()` and return a component tree:

```python
from plexi_sdk import App
from plexi_sdk.ui import AppBar, Column, Label


class MyApp(App):
    def view(self):
        return Column([
            AppBar("My App"),
            Label("Hello from my-app"),
        ])


MyApp().run()
```

Use `on_render(ctx)` only for games, animations, realtime visualizations, or other pixel-control apps.

## Capabilities

Declare what your app needs in `manifest.toml`:

```toml
[app.capabilities]
capabilities = ["secrets.get", "net.http"]
```

Common capabilities: `secrets.get`, `net.http`, `fs.read`, `fs.write`, `ai.query`, `audio.record`, `timer`.

## Logging

Use `self.emit.info()`, `self.emit.warn()`, `self.emit.error()` from any method. Log lines are tagged `app::my-app` in the host log.

```python
from plexi_sdk import App


class MyApp(App):
    def on_init(self) -> None:
        self.emit.info("my-app initialized")

    def view(self):
        ...


MyApp().run()
```

See also: [PGAP](/docs/pgap), [Secrets](/docs/secrets)
