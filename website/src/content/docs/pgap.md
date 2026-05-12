---
title: PGAP
description: The Plexi General App Protocol — how apps communicate with the host.
verified_version: "3.6.19"
order: 3
---

PGAP (Plexi General App Protocol) is the binary protocol that connects sandboxed apps to the Plexi host. Every Plexi app is a separate process that communicates exclusively through PGAP — it has no direct access to the host's state, other panes, or the filesystem beyond what it's given.

## How It Works

When you run a Plexi app, the host spawns it as a child process and establishes a bidirectional frame channel over stdio. The app sends **draw frames** describing what to render; the host sends **input frames** when the user interacts with the pane.

```
Host ──draw request──► App
Host ◄──draw frame──── App
Host ──input event──► App
```

The host owns the render loop. Each frame tick, the host asks each app pane for a fresh draw frame and composites it into the UI.

## Frame Types

| Frame | Direction | Purpose |
|-------|-----------|---------|
| `Draw` | App → Host | Render instructions for the current tick |
| `Input` | Host → App | Keyboard/mouse events |
| `Notify` | App → Host | Push a notification to the user |
| `Context` | Host → App | Workspace context (cwd, secrets, etc.) |
| `Log` | App → Host | Emit a log line into the host's log |

## Capabilities

Every app declares its capabilities in its `manifest.toml`. The host enforces these at runtime — an app that didn't declare `secrets` access cannot read secrets, even if it tries.

```toml
[capabilities]
secrets = true
notifications = true
context = true
```

## The Python SDK

The recommended way to write a Plexi app is with the Python SDK:

```sh
pip install plexi-sdk
```

The SDK wraps PGAP into idiomatic Python. You write a render function; the SDK handles the frame loop.

```python
from plexi import App

app = App()

@app.on_draw
def draw(ctx):
    ctx.text("Hello from PGAP")
    ctx.info("draw called")

app.run()
```

See [Apps](/docs/apps) for the full development workflow.
