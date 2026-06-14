# SDK Quickstart: Your First Plexi App

Audience: a coding agent building its first PGAP app.
Goal: a running counter app using SDK v2.
Deeper reference: [`docs/sdk-v2.md`](sdk-v2.md) and [`docs/PGAP_REFERENCE.md`](PGAP_REFERENCE.md).

Naming: SDK v2 is the Python authoring API. PGAP v3 (`pgap/3`) is the host/app protocol it speaks.

## 1. Create The App

Run this inside a Plexi workspace:

```bash
plexi app init counter
```

If you are outside a workspace, use `--global`:

```bash
plexi app init --global counter
```

The scaffold creates an app directory with `manifest.toml` and `main.py`. Do not hand-write the manifest for a new app. Start from the scaffold and edit only the fields you need.
It does not open the app by default; run `plexi app init counter --open` if you want the new app launched immediately.

## 2. The App Pattern

Normal apps use `view()` and return a component tree. The SDK sends that tree to the host, and the host handles layout, spacing, theme colors, hit testing, and rendering.

```python
#!/usr/bin/env python3
from plexi_sdk import App
from plexi_sdk.ui import AppBar, Column, FooterKeys, Label, Spacer


class CounterApp(App):
    def on_init(self) -> None:
        self.count = self.state.get("count", 0)

    def view(self):
        return Column([
            AppBar("Counter"),
            Spacer(grow=True),
            Label(str(self.count), bold=True),
            Spacer(grow=True),
            FooterKeys([
                ("+", "increment"),
                ("-", "decrement"),
                ("r", "reset"),
            ]),
        ])

    def on_key(self, key: str, mods: dict) -> None:
        if key in ("plus", "equals"):
            self.count += 1
        elif key == "minus":
            self.count -= 1
        elif key == "r":
            self.count = 0
        self.state.save({"count": self.count})
        self.emit.schedule_render()


CounterApp().run()
```

Use `on_render(ctx)` only for games, animations, realtime visualizations, or other pixel-control apps. Never override both `view()` and `on_render(ctx)`.

## 3. Manifest

The generated manifest has the required fields:

```toml
schema_version = 1

[app]
id = "counter"
type = "app"
name = "Counter"
entry = "main.py"
version = "0.1.0"
description = "A Plexi app"
watch = true

[app.capabilities]
capabilities = []

[launch]
```

Add capabilities only when the app needs host-brokered powers such as `net.http`, `fs.read`, `fs.write`, `secrets.get`, `ai.query`, `panes.spawn`, or `terminal.bindings`.

Python apps are native subprocesses. Capabilities gate PGAP host APIs; they are not a Python process sandbox.

## 4. Open And Test

```bash
plexi app check ./counter
plexi app open ./counter
plexi app render ./counter --state '{"count": 5}'
```

`plexi app check` validates the manifest, inspects Python SDK usage without importing app code, and renders the app at small and normal pane sizes.

For a live app pane, agents can drive the UI with:

```bash
plexi pane state <pane_id>
plexi pane key <pane_id> plus
plexi pane key <pane_id> minus
```

The loop is: render, inspect the UiNode tree, send an action or key, inspect again.

## 5. The Dev Loop

The two commands every app author (human or agent) needs:

```bash
plexi app open .       # open the app in a pane with hot reload, then edit main.py
plexi app check .      # validate manifest + SDK shape + render-size matrix (no pane)
```

The app's manifest sets `watch = true`, so saving `main.py` reloads the
pane automatically.

### The agent drive loop (render → inspect → act)

An agent iterates on an app without reading host internals by repeating one
cycle. Concretely, for the counter app:

```bash
# 1. RENDER — get the current UI tree as JSON (no pane needed)
plexi app render ./counter
#    → { "type": "Column", "children": [ ... { "type": "Label", "text": "0" } ] }

# 2. INSPECT — confirm the Label shows "0", decide the next action is "+"

# 3. ACT — open a live pane, then send the key
plexi app open ./counter
plexi pane key <pane_id> plus

# 4. RE-INSPECT — read the pane state and confirm the count advanced
plexi pane state <pane_id>
#    → Label now reads "1"
```

Edit `main.py`, save, and `watch = true` reloads the pane; re-run
`plexi app render` (or `plexi app check .`) to verify the change before moving on.
Each cycle is one observable diff — keep changes small.

## 6. Verify And Publish

The full authoring flow is **init → dev → verify → publish**. Each step has one
command:

```bash
plexi app check ./counter      # render-inspect: validates the manifest, inspects
                               # SDK usage without importing app code, and renders
                               # the app across the small/normal pane size matrix.
                               # This is the non-interactive verification an agent
                               # runs each cycle — green means it renders.
plexi app validate ./counter   # fail-closed package check: descriptor, content
                               # hashes, manifest, entry point, capability strings.
                               # Run before packaging or publishing.
plexi app package ./counter    # build a distributable <id>-<version>.plexipkg
plexi app publish ./counter    # submit to the marketplace (needs the [marketplace]
                               # section — uncomment it in manifest.toml first)
```

`plexi app check` is the verification harness an agent loops on while building;
`plexi app validate` / `package` / `publish` are the release path. The scaffolded
`manifest.toml` already includes a commented `[marketplace]` block, so publishing
is one uncomment (and a `publisher` value) away.

> **This doc vs. [`docs/sdk-v2.md`](sdk-v2.md):** this Quickstart is the
> end-to-end *tutorial* (the path from empty dir to published app).
> `sdk-v2.md` is the *reference* (component tables, the canvas API, the full
> protocol surface). Start here; reach for `sdk-v2.md` when you need a specific
> API.

## 7. Next Steps

- SDK v2 reference: [`docs/sdk-v2.md`](sdk-v2.md)
- PGAP wire reference: [`docs/PGAP_REFERENCE.md`](PGAP_REFERENCE.md)
- Security model: [`docs/SECURITY_MODEL.md`](SECURITY_MODEL.md)
- App framework roadmap: [`docs/prm/app-framework-marketplace.md`](prm/app-framework-marketplace.md)
