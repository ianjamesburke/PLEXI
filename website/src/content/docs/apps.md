---
title: Apps
description: Build and run sandboxed apps inside Plexi.
verified_version: "3.6.19"
order: 5
---

Plexi apps are sandboxed processes that render into panes via PGAP. They can be written in any language, but the Python SDK is the primary development path.

## Prerequisites

```sh
pip install plexi-sdk
```

## Workspace Init

Apps live in a **workspace** — a project directory you've initialized with Plexi. From inside your project:

```sh
cd your-project/
plexi workspace init
```

This creates a `.plexi/` directory with `workspace.toml`. Apps you create here are scoped to this workspace.

## Create an App

```sh
plexi app init my-app
```

This scaffolds a new app under `.plexi/apps/my-app/`:

```
.plexi/apps/my-app/
  manifest.toml    ← capabilities, entry point, metadata
  main.py          ← your app code
```

## Run an App

From inside a Plexi pane, with your workspace directory as CWD:

```sh
plexi app run my-app
```

The focused pane switches to app mode and starts rendering your app.

## The Render Loop

Your `main.py` receives draw requests from the host on each tick. Return a draw frame describing the UI:

```python
from plexi import App

app = App()

@app.on_draw
def draw(ctx):
    ctx.text("Hello from my-app", color="#f0f3f6")

app.run()
```

## Capabilities

Declare what your app needs in `manifest.toml`:

```toml
[capabilities]
secrets = true        # read secrets from the host
notifications = true  # push notifications
context = true        # workspace context (cwd, git branch, etc.)
```

The host enforces these at launch. An app without `secrets = true` cannot read any secret, even if it requests one.

## Logging

Use `ctx.info()`, `ctx.warn()`, `ctx.error()` inside draw handlers. Log lines are tagged `app::my-app` in the host log.

```python
@app.on_draw
def draw(ctx):
    ctx.info("draw tick")
```

See also: [PGAP](/docs/pgap), [Secrets](/docs/secrets)
