# Authoring Plexi Apps

Canonical guide for building a Plexi app on the Python SDK v3. Every other
authoring surface (`README.md`, both `AGENTS.md` files, the scaffolded app
contract, the `plexi app init --help` block) points here; this file is the one
place the how-to-build knowledge lives.

- **Full API reference** (every effect, event, and UI component with signatures)
  is generated from the SDK source into
  [`website/src/content/docs/sdk.md`](../../website/src/content/docs/sdk.md).
  That file is the exhaustive, always-fresh surface. This guide teaches the
  shape; `sdk.md` is the dictionary.
- **Design/protocol spec** (adapter contract, native ProcessApp bridge, WIT
  mapping) lives in [`SDK_V3.md`](SDK_V3.md).
- **Traps** (non-obvious failure modes) live in [`AGENTS.md`](AGENTS.md).

## Scaffold, Never Hand-Write

```sh
plexi app init myapp          # or plexi-pr-<N> app init myapp on a PR build
plexi app check myapp --png-dir /tmp/myapp-shots
plexi app open myapp
```

`plexi app init` writes `main.py`, `manifest.toml`, `tests/test_app.py`,
`AGENTS.md`, `.gitignore`, and `plexi.scaffold.toml`. Never author a
`manifest.toml` by hand — the scaffold sets the correct `schema_version` and a
valid shape. Prune the unused SDK imports the template ships before finishing.

## App Contract

An app is three module-level functions — no class, no inheritance:

```python
#!/usr/bin/env python3
from __future__ import annotations

from plexi_sdk import log, state
from plexi_sdk.effects import SetState, SetTitle
from plexi_sdk.events import KeyEvent
from plexi_sdk.ui import AppBar, Column, FooterKeys, Text


def init(size, args):
    log.info("counter: init")
    return [SetTitle("Counter"), SetState({"count": 0})]


def update(event):
    if isinstance(event, KeyEvent) and event.key == "plus" and event.pressed:
        return [SetState({"count": state.get("count", 0) + 1})]
    return []


def view():
    return Column([
        AppBar("Counter"),
        Text(str(state.get("count", 0)), bold=True, align="center"),
        FooterKeys([("+", "increment")]),
    ], grow=True)
```

- `init(size, args)` runs once on launch; return startup effects.
- `update(event)` runs for every input event; return effects. Never mutate in
  place — effects are data the adapter executes after `update()` returns.
- `view()` must be **pure**. Calling `state.set()` inside `view()` raises
  `RuntimeError`. All mutation flows through effects returned from `update()`.

## Two Rendering Modes

Pick one per app:

- **Declarative UI trees** — forms, lists, dashboards. `view()` returns a
  component tree (`Column`/`Row`/widgets).
- **Canvas drawing** — games, animations, visualizations. `view()` returns a
  `Canvas([...])`. For animation, return `SetSchedulerMode("continuous", fps=60)`
  from `init()` and advance simulation state from `RenderFrame` events. Do not
  build animation loops with timers.

## Effects

Effects are dataclasses returned from `init()`/`update()`. The full list with
fields is in `sdk.md` (Effects section). The common ones:

<!-- drift-check:effects -->
`SetState`, `PersistState`, `SetTitle`, `SetStatus`, `SetTimer`, `CancelTimer`,
`SetSchedulerMode`, `FileRead`, `FileWrite`, `HttpFetch`, `AiQuery`, `AiMessage`,
`CloseSelf`, `RequestCapability`,
`GetSystemStats`
<!-- /drift-check:effects -->

`SetState` is process-local runtime state — view/update data, caches, game
state. `PersistState` writes the same snapshot **and** saves durable app state;
use it only when a key must survive an app restart. There are no `LogInfo` /
`LogWarn` / `LogError` effects — logging goes through the `log` module (below).
Network fetches use `HttpFetch`, not `HttpRequest`.

## App Tools

Apps can expose tools to the Assistant with `ExposeTools`. Declare the full
set in `init()`. When the Assistant calls one, `update()` receives a
`ToolCall`; return one `ToolResult` with the matching `call_id`.

```python
import json

from plexi_sdk.effects import AiTool, ExposeTools, ToolResult
from plexi_sdk.events import ToolCall


def init(size, args):
    return [ExposeTools([
        AiTool(
            name="csv.describe_table",
            description="Describe the current table.",
            input_schema={"type": "object", "properties": {}},
            read_only=True,
        ),
    ])]


def update(event):
    if isinstance(event, ToolCall) and event.name == "csv.describe_table":
        return [ToolResult(event.call_id, output_json=json.dumps({"rows": 42}))]
    return []
```

`input_schema` is a JSON Schema object. `ToolCall.input_json` is the JSON
string supplied by the Assistant. Return `output_json` for success or `error`
for failure, never both. A declaration replaces the pane's previous tool set,
so send every current tool when it changes.

Set `read_only=True` only for tools that never mutate app or workspace state.
The Assistant runs read-only tools without a write-grant prompt, while still
recording every call. Mutating tools prompt for an allow-once or persisted
narrow grant before they run.

## UI Components

Import from `plexi_sdk.ui`. Read `sdk.md` (UI Components section) for the full
API; the widgets you reach for most:

<!-- drift-check:components -->
`AppBar`, `ActionBar`, `Column`, `Row`, `HStack`, `Label`, `Text`, `Heading`,
`Spacer`, `Divider`, `FooterKeys`, `Footer`, `SelectList`, `TextEdit`, `Card`,
`Section`, `Tabs`, `Grid`, `Toggle`, `ScrollLog`, `Scrollable`, `ChatBubble`,
`Markdown`, `InfoTable`, `FormField`, `ButtonRow`, `Button`, `Badge`,
`ProgressBar`, `Clickable`, `Canvas`, `CanvasRect`, `CanvasText`, `CanvasCircle`,
`CanvasLine`
<!-- /drift-check:components -->

Widget selection rules:

- **List + detail navigation:** use `SelectList`. It handles j/k/arrow keys,
  scrolling, and click hit-testing. Never reimplement this by hand.
- **Text entry:** use `TextEdit` in the component tree. Never read raw keys for
  text.
- **Raw drawing / games:** return `Canvas(...)` and drive state from
  `RenderFrame`.

`Canvas` defaults to `fit="fill"`, the existing SDK behavior: its coordinate
space fills the allocated pane and may scale differently on each axis. Existing
apps need no change. Use `fit="contain"` when geometry must keep its source
aspect ratio; the host centers the canvas and leaves unused space on two sides.
This is appropriate for square game cells and circles that must remain round.

Timers remain supported for ordinary delayed and periodic work. Animation apps
should use `SetSchedulerMode("continuous", fps=...)` plus `RenderFrame`; this
lets the host request a new paint only when the guest commits a frame.

PGAP is L1-only: build declarative L1 trees. L0 is deprecated and its `_l0`
fallbacks are gone; the `Raw` escape hatch stays.

Keep the root shell padded: `Column([...], grow=True, padding=SPACE_MD)` or
larger. Never `padding=0` on the root — app bars and footers render full-bleed
while body content stays inset.

Canvas apps bypass the host WCAG contrast check. Use `theme.fg` for canvas text
and reserve `dim()`/`theme.muted` for fills, or text fails contrast.

## Keyboard Conventions

Keys arrive in `update(event)` as `KeyEvent`. Key strings are lowercase
canonical — never match `"Enter"` or `"Escape"`.
`event.pressed` is `True` on key-down and `False` on key-up. Track held keys by
adding on press and removing on release; do not toggle state on every event.

| Physical key | `event.key` |
|---|---|
| Enter / Return | `"return"` |
| Escape | `"escape"` |
| Backspace | `"backspace"` |
| Space | `"space"` |
| Arrow keys | `"up"` / `"down"` / `"left"` / `"right"` |
| Tab | `"tab"` |
| Letters | `"a"`–`"z"` (lowercase) |
| Plus / Minus / Equals | `"plus"` / `"minus"` / `"equals"` |
| Function keys | `"f1"`–`"f12"` |

Convention: Enter = open/confirm. Escape (+ optional Backspace) = exit/cancel.
Every focused sub-view must be escapable.

## State

Keep state in `plexi_sdk.state` and update it by returning `SetState` /
`state.set(...)` from `update(event)`:

```python
state.get("key", default)   # read runtime state (also valid inside view())
state.set("key", value)     # returns a SetState effect — does NOT mutate now
state.all()                 # every key, decoded
```

`view()` reads state but must never write it.

## Logging

Log through the frame with `plexi_sdk.log`. App logs forward into the host log
tagged `app::<app_id>`.

- `log.debug` — detailed state/render/event diagnostics.
- `log.info` — init, user actions, state transitions.
- `log.warn` — recoverable fallbacks or ignored input.
- `log.error` — unrecoverable failures, before returning or surfacing them.

Always log at init and key state transitions; log escapes and errors from
`update(event)`.

## Manifest

The scaffold writes a valid `manifest.toml`; the shape lives in the template at
[`plexi_sdk/templates/manifest.toml`](plexi_sdk/templates/manifest.toml). A
`[app] type = "app"` with a `.py` entry launches through native `ProcessApp`.
Declare capabilities under `[app.capabilities]`; see `sdk.md` and the PGAP
capability reference for what each grant allows.

`[launch] on_launch` declares what the host does when your app is launched
while an instance already exists: `focus_existing` (one instance globally —
relaunch focuses it, jumping context if needed), `focus_existing_in_context`
(one instance per context), or `always_new` (default — every launch spawns a
fresh instance). An unknown value fails install loudly. Under `always_new`,
duplicate instances are fully independent: each receives events through its
own event-bus subscriptions, and instance identity is the host-stamped pane
id — never an id your app assigns itself. Don't try to self-dedup or key
shared state on a self-chosen id; declare `on_launch` and let the host resolve
launches. Note that when a relaunch is deduped to an existing instance, any
`args` (or cwd) that relaunch carried are **not** delivered to the running
instance — the host focuses it as-is (delivering relaunch args to a live
instance via a bus event is future work).

## Dev / Test Loop

Test-first. Define done by the test, not the code.

- Add or update `tests/test_app.py` (AppHarness) for every behavior change
  before touching app code, then run `plexi app test .`.
- Exercise behavior through actions and keys, not only by reading code:
  `plexi app action <pane-id> <handler-id>` and `plexi pane key <pane-id> <key>`.
- Hot reload is part of the loop. New apps set `watch = true`; after
  `plexi app open .`, edit source and confirm the same pane id updates without
  reopening (check `plexi pane state <pane-id>` and the host log's `hot_reload`
  lines).
- Final gate: `plexi app check .` renders the size matrix and validates the SDK
  v3 shape. There is no `plexi app build`.

Make the channel explicit when validating — `PLEXI_CHANNEL` leaks into app
tooling:

```sh
PLEXI_CHANNEL=alpha plexi app check . --png-dir render-output/check
PLEXI_CHANNEL=pr-123 plexi app check . --png-dir render-output/check   # or: plexi-pr-123 app check .
PLEXI_CHANNEL=alpha plexi app render . --state fixtures/state.json      # JSON frame tree
PLEXI_CHANNEL=alpha plexi app render . --png --output render-output/shot.png
```

When probing a running host from outside its Plexi pane, set the matching
socket explicitly:
`PLEXI_SOCKET=$HOME/.plexi-alpha/notify.sock PLEXI_CHANNEL=alpha plexi ...`.
Inspect `~/.plexi-alpha/plexi.log` (or `~/.plexi-pr-<N>/plexi.log`) for app logs.

## Traps

`plexi_sdk` is only importable inside Plexi-spawned processes — it is on
`PYTHONPATH` only for apps Plexi launches through native `ProcessApp`. A bare
`python3 -c "import plexi_sdk"` in a terminal will fail or import a stale copy.
Test by opening the app in a pane. More traps: [`AGENTS.md`](AGENTS.md).

## Design Philosophy

- Obvious over clever — fight for the solution an agent would naturally assume.
- Simulate affordances, never lie about contracts — isolation, durability,
  persistence, and security boundaries stay explicit.
- Build primitives, not features — omit anything a developer's agent can
  trivially build atop the platform.
- Design for agents, not humans browsing docs — if it needs a README to be
  usable, the API is wrong.
