---
title: PGAP
description: The Plexi Generic App Protocol — how apps communicate with the host.
verified_version: "0.0.505"
order: 3
---

PGAP (Plexi Generic App Protocol) is the communication layer between sandboxed apps and the Plexi host. It runs as newline-delimited JSON over a child process's stdin/stdout — every Plexi app is a separate process with no direct access to host state, other panes, or the filesystem beyond what it's granted.

## How It Works

When you run a Plexi app, the host spawns it as a child process and establishes a bidirectional message channel over stdio. The app sends draw commands describing what to render; the host sends events when the user interacts with the pane.

```text
Host ──render request──► App
Host ◄──draw commands─── App
Host ──input event──────► App
```

The host owns the render loop. Each frame tick, the host asks each app pane for a fresh set of draw commands and composites them into the UI.

## Message Types

Messages flow in both directions. Key app→host types: `Render` (draw commands for the current tick), `Notify` (push a notification), `Log` (emit a log line), `SecretGet` (request a secret value). Key host→app types: `Init` (startup context including granted capabilities), `Key`/`Mouse` (input events), `Rect` (pane dimensions), `SecretValue` (secret response).

This is a non-exhaustive summary. The full schema is in `sdk/protocol/pgap.schema.json`.

## Capabilities

Every app declares its capabilities in `manifest.toml`. The host enforces these at runtime:

```toml
[app.capabilities]
capabilities = ["secrets.read", "net.http"]
```

An app that didn't declare `secrets.read` cannot read any secret, even if it requests one.

## The Python SDK

The recommended way to write a Plexi app is with the Python SDK. It's bundled with Plexi — no separate install needed.

The SDK wraps PGAP into idiomatic Python. Subclass `App` and override `on_render`; the SDK handles the message loop.

```python
from plexi_sdk import App, RenderContext


class MyApp(App):
    def on_render(self, ctx: RenderContext) -> None:
        ctx.text("Hello from PGAP")
        self.emit.info("render called")


MyApp().run()
```

See [Apps](/docs/apps) for the full development workflow.
