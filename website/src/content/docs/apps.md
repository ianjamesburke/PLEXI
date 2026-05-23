---
title: Apps
description: Build and run sandboxed apps inside Plexi.
verified_version: "0.0.496"
order: 5
---

Plexi apps are sandboxed processes that render into panes via PGAP. They can be written in any language, but the Python SDK is the primary development path.

## Prerequisites

```sh
uv add plexi-sdk
```

Or with pip:

```sh
pip install plexi-sdk
```

## Create an App

From inside any directory:

```sh
plexi app init my-app
```

This scaffolds a new app folder with:

```
my-app/
  manifest.toml    ← capabilities, entry point, metadata
  main.py          ← your app code
```

## Run an App

Pass the path to the app folder:

```sh
plexi app run ./my-app
```

The focused pane switches to app mode and starts rendering your app. Use this during development — no install step required.

## The Render Loop

Subclass `App` and override `on_render`. The host calls it on every tick:

```python
from plexi_sdk import App, RenderContext


class MyApp(App):
    def on_render(self, ctx: RenderContext) -> None:
        ctx.text("Hello from my-app", color="#f0f3f6")


MyApp().run()
```

## Capabilities

Declare what your app needs in `manifest.toml`:

```toml
[app.capabilities]
capabilities = ["secrets.read", "net.http"]
```

The host enforces these at launch. An app that didn't declare `secrets.read` cannot read any secret, even if it requests one.

Common capabilities: `secrets.read`, `net.http`, `fs.read`, `fs.write`, `ai.query`, `audio.record`, `timer`.

## Logging

Use `self.emit.info()`, `self.emit.warn()`, `self.emit.error()` from any method. Log lines are tagged `app::my-app` in the host log.

```python
from plexi_sdk import App, RenderContext


class MyApp(App):
    async def on_init(self, ctx: RenderContext) -> None:
        self.emit.info("my-app initialized")

    def on_render(self, ctx: RenderContext) -> None:
        ctx.text("Hello from my-app", color="#f0f3f6")


MyApp().run()
```

See also: [PGAP](/docs/pgap), [Secrets](/docs/secrets)
