---
title: PGAP
description: The Plexi Generic App Protocol — how apps communicate with the host.
verified_version: "0.0.669"
order: 3
---

PGAP (Plexi Generic App Protocol) is the communication layer between app processes and the Plexi host. It runs as newline-delimited JSON over a child process's stdin/stdout. PGAP is the host API boundary; native Python apps are not process-sandboxed.

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
capabilities = ["secrets.get", "net.http"]
```

Capabilities gate PGAP host APIs; they do not restrict what a native Python process can do outside PGAP.

## The Python SDK

The recommended way to write a Plexi app is with the Python SDK. It's bundled with Plexi — no separate install needed.

The SDK wraps PGAP into idiomatic Python. Subclass `App`, implement `view()`, and return a component tree.

```python
from plexi_sdk import App
from plexi_sdk.ui import AppBar, Column, Label


class MyApp(App):
    def view(self):
        return Column([
            AppBar("PGAP App"),
            Label("Hello from PGAP"),
        ])


MyApp().run()
```

See [Apps](/docs/apps) for the full development workflow.
