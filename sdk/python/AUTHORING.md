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
- **Design/protocol spec** (WIT mapping, CPython-in-WASM adapter) lives in
  [`SDK_V3.md`](SDK_V3.md) — its own status note flags which sections are
  historical (native `ProcessApp`, deleted by stint 0285). For the current
  shipped runtime contract, read [`../../docs/wasm-runtime.md`](../../docs/wasm-runtime.md).
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
  component tree (`Column`/`HStack`/widgets).
- **Canvas drawing** — games, animations, visualizations. `view()` returns a
  `Canvas([...])`. For animation, return `SetSchedulerMode("continuous", fps=60)`
  from `init()` and advance simulation state from `RenderFrame` events. Do not
  build animation loops with timers.

## Effects

Effects are dataclasses returned from `init()`/`update()`. The full list with
fields is in `sdk.md` (Effects section). The common ones:

<!-- drift-check:effects -->
`SetState`, `PersistState`, `SetTitle`, `SetStatus`, `SetTimer`, `CancelTimer`,
`SetSchedulerMode`, `FileRead`, `FileWrite`, `OpenFilePicker`, `HttpFetch`,
`AiQuery`, `AiMessage`, `CloseSelf`, `RequestCapability`,
`GetSystemStats`
<!-- /drift-check:effects -->

`SetState` is process-local runtime state — view/update data, caches, game
state. `PersistState` writes the same snapshot **and** saves durable app state;
use it only when a key must survive an app restart. Both take an optional
`scope=` naming one of the app's declared state scopes (see `[state] scopes`
in the Manifest section); omitted means the app's default scope, and an
undeclared scope raises — never a silent fallback. `state.get`/`state.all`
accept the same `scope=` argument. There are no `LogInfo` /
`LogWarn` / `LogError` effects — logging goes through the `log` module (below).
Network fetches use `HttpFetch`, not `HttpRequest`.

File I/O is binary-exact: `effects.read_bytes(path)` / `effects.write_bytes(path,
content)` round-trip arbitrary bytes (WAV, PNG, NULs) through the workspace jail
under the `fs.read` / `fs.write` capabilities. The reply to a read is
`events.FileReadResult` with `content: bytes` — decode yourself for text. Both
directions are capped at `effects.MAX_FILE_IO_BYTES` per call; oversize payloads
fail with a named error, never a truncated write.

## Open and Save-As

`FileRead`/`FileWrite` paths are workspace-relative and jailed to the
workspace root. To reach a file the user chooses anywhere on disk, return
`OpenFilePicker(request_id, mode=...)` (`fs.pick` capability): `"open"` picks
existing files (`multiple=True` for several), `"folder"` picks a directory,
`"save"` picks a destination that may not exist yet. The host replies with
`FilePicked` — its absolute paths are registered as scoped fs grants for this
pane, so pass them verbatim to `FileRead`/`FileWrite` (a folder grant covers
the subtree) — or `FilePickCancelled` (dismissed or capability denied; no
grant is created). Grants last for the pane's lifetime and are never
persisted. `apps/dev/file-picker-poc` is the working example.

Production shows a native dialog, which no test can click. Headless drivers
script the picker instead: launch the host with `PLEXI_PICKER_SCRIPT`
pointing at a JSON array of `{"paths": [...]}` / `{"cancel": true}` outcomes
(consumed in order, per pane), or use a scene's `picker_script` key
(`tests/scenes/file-picker.toml`).

## App Tools

Apps can expose tools with `ExposeTools`. The Assistant can call them directly;
external MCP agents in the same workspace see them as
`<app_id>__<tool>` through the config printed by `plexi events mcp-config`.
Declare the full set in `init()`. When any caller invokes one, `update()`
receives a `ToolCall`; return one `ToolResult` with the matching `call_id`.

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
            output_schema={"type": "object", "properties": {"rows": {"type": "integer"}}},
            read_only=True,
        ),
    ])]


def update(event):
    if isinstance(event, ToolCall) and event.name == "csv.describe_table":
        return [ToolResult(event.call_id, output_json=json.dumps({"rows": 42}))]
    return []
```

`plexi_sdk.tools` writes that declaration, its schema, and its dispatch arm from
one decorator, so a tool lives in one place instead of three:

```python
from plexi_sdk import tools


@tools.tool("csv.describe_table", "Describe the current table.", returns={"rows": int})
def _describe() -> dict:
    return {"rows": 42}


@tools.tool("csv.set_title", "Rename the table.", {"title": str})
def _set_title(title: str) -> tools.Reply:
    return tools.Reply({"title": title}, [PersistState({"title": title})])


def init(size, args):
    return [tools.expose()]


def update(event):
    return tools.dispatch(event) or <the app's own handling>
```

`params`/`returns` map argument names to `str`/`int`/`float`/`bool` and become
JSON Schema objects with every key required. A mutating tool returns its effects
in a `tools.Reply` rather than writing state itself, and an exception inside a
tool is reported to the Assistant as that call's `error` — never as a crash.
Reach for the raw `AiTool`/`ExposeTools`/`ToolResult` types below only for a
schema the decorator's type map cannot express.

`input_schema` and `output_schema` are JSON Schema objects. `ToolCall.input_json`
is the caller-supplied JSON string. Return `output_json` matching the declared
output schema for success or `error` for failure, never both. A declaration
replaces the pane's previous tool set, so send every current tool when it
changes.

Set `read_only=True` only for tools that never mutate app or workspace state.
For Assistant calls specifically, read-only tools run without a write-grant
prompt while every call is still recorded. Mutating Assistant calls prompt for
an allow-once or persisted narrow grant before they run.

## Cross-app events

Use `DeclareEventStreams` and `EmitEvent` for streams owned by the current app.
Use `SubscribeEventStreams` with another app's ID and declared stream names to
receive its events. The subscription result supplies the ID needed by
`UnsubscribeEventStreams`; matching events arrive as `AppEvent` in `update()`.
Python and Rust WASM apps share the same host registry, so publishers and
subscribers can use either runtime. Subscriptions last until unsubscribe or
the subscriber pane closes. The host permission broker may ask the user before
allowing a cross-app subscription.

## UI Components

Import from `plexi_sdk.ui`. Read `sdk.md` (UI Components section) for the full
API; the widgets you reach for most:

<!-- drift-check:components -->
`AppBar`, `ActionBar`, `Column`, `HStack`, `Label`, `Text`, `Heading`,
`Spacer`, `Divider`, `FooterKeys`, `SelectList`, `Pending`, `TextEdit`, `Card`,
`Section`, `Tabs`, `Grid`, `Toggle`, `Scrollable`,
`ButtonRow`, `Button`, `Badge`, `Actions`, `FormField`, `TextInput`,
`ProgressBar`, `Canvas`, `CanvasRect`, `CanvasText`, `CanvasCircle`,
`CanvasLine`
<!-- /drift-check:components -->

Widget selection rules:

- **List + detail navigation:** use `SelectList`. It handles j/k/arrow keys,
  scrolling, and click hit-testing. Never reimplement this by hand.
- **Loading states:** wrap only the region whose fetch is in flight in
  `Pending(active=..., child=..., placeholder=Skeleton(rows=N))`. Placeholder
  is always explicit, sized to the eventual content so nothing jumps. Never
  thread a loading boolean through multiple view branches.
- **Text entry:** use `TextEdit` in the component tree. Never read raw keys for
  text.
- **Forms:** a labeled field is a `FormField`; a Save/Cancel pair is an
  `Actions` row. Buttons declared as plain `Column` children each take the
  column's full width and stack, so an action pair written that way reads as
  two unrelated bars — `Actions` is the row.
- **Focus:** the field that should hold the cursor declares
  `autofocus=True` (on `TextInput` or `FormField`). The host focuses it
  whenever the pane owns input and nothing else is focused, including the
  frame a revealed form first renders, so an app never issues a focus command
  or routes keys itself.
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

PGAP is L1-only: build declarative L1 trees — the WIT `ui-node-data` variant
set (`wit/plexi.wit`) is the single, live UI node language; there is no
separate legacy renderer for it to diverge from. L0 is deprecated and its
`_l0` fallbacks are gone; the `Raw` escape hatch stays.

### Spacing is good by default

You write no layout code to look right. The host fills in its standard spacing
wherever your tree declares none, and any explicit value you set wins:

- **Root content inset.** A declarative tree is automatically inset from the
  pane edge (horizontal `SPACE_XL`, vertical `SPACE_MD`). A leading `AppBar`
  and a trailing `FooterKeys` are the app's edges, not its content: they render
  full-bleed against the pane rect while only the body between them is inset —
  so a `Column([AppBar(...), ...body..., FooterKeys(...)])` needs no layout
  code.
- **Bottom-pinned content.** A `Spacer(grow=True)` splits a column: everything
  before it flows from the top, everything after it sits flush against the
  bottom. Children after the spacer are never pushed off the pane.
  You no longer set root padding: `Column(padding=...)` does not affect the
  declarative tree (it survives only for legacy canvas-mode layout).
- **Inter-child spacing.** Leave `gap` unset on `Column`/`HStack` and the host
  spaces children (columns get `SPACE_MD`, rows get the tighter `SPACE_SM`).
  Pass an explicit `gap=` to override; `gap=0` packs children flush.
- **Button targets.** Buttons get a comfortable minimum click/touch size, so
  single-glyph buttons (calculator keys, toolbar chips) are never cramped.

Full-bleed pixel apps opt out automatically: a grow `Canvas` or a GPU
`Surface` anywhere in the tree tells the host the app owns every pixel, so no
inset is applied (games, visualizers, video). A fixed-size `Canvas` is ordinary
flow content and stays inset.

Canvas apps bypass the host WCAG contrast check. Use `theme.fg` for canvas text
and reserve `dim()`/`theme.muted` for fills, or text fails contrast.

`Badge`/`Banner` colors are the theme's SEMANTIC roles, not literal colors:
`accent`, `success`, `warning`, `danger`, `neutral`, plus the alias roles
`red`/`green`/`yellow` (== `danger`/`success`/`warning`). There is no `blue`.
An invalid value raises `ValueError` at construction, not at host render. A
bespoke color needs `AppPalette(dark=..., light=...)` — never a raw hex or
CSS name inline.

## Keyboard Conventions

Keys arrive in `update(event)` as `KeyEvent`. Key strings are lowercase
canonical — never match `"Enter"` or `"Escape"`.
Command-key chords are host shortcuts and never reach apps. App shortcuts may
use unmodified keys and the Ctrl, Shift, or Alt modifier fields.
`event.pressed` is `True` on key-down and `False` on key-up. Track held keys by
adding on press and removing on release; do not toggle state on every event.

| Physical key | `event.key` |
|---|---|
| Enter / Return | `"enter"` |
| Escape | `"escape"` |
| Backspace | `"backspace"` |
| Space | `"space"` |
| Arrow keys | `"up"` / `"down"` / `"left"` / `"right"` |
| Tab | `"tab"` |
| Letters | `"a"`–`"z"` (lowercase) |
| Digits (main row or numpad) | `"0"`–`"9"` |
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
`[app] type = "app"` with a `.py` entry launches through the CPython-in-WASM
adapter (stint 0285). Declare capabilities under `[app.capabilities]`; see
`sdk.md` and `docs/wasm-runtime.md`'s "Capabilities" section for what each
grant allows.

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

`[state] scopes` declares which state scopes your app addresses, ordered —
the first entry is the default:

```toml
[state]
scopes = ["global", "context"]
```

Omitting `[state]` gives `["global"]`. The host owns path construction:
`global` state lives in `~/.plexi/app_states/<app_id>.<ext>` (channel-neutral,
cross-project), `context` state in `<context_root>/.plexi/app_states/`,
resolved against the pane's context root at call time — so
`plexi context set-root` immediately redirects where context-scoped state
lands. Context-scoped state is gitignored by the host (`.plexi/.gitignore`
gains `app_states/` automatically): app state is personal, single-user, local
data — never committed. An unknown or empty scope list fails install loudly.

`[state] format` selects the on-disk shape: `"json"` (default, `.json`
extension) or `"markdown"` (`.md`). Markdown keeps the host format-blind:
your state must carry the whole document as a string under the single key
`document` — `PersistState({"document": text})` writes those bytes verbatim
(a non-string `document` is a loud error and nothing is written), and reads
arrive back as `{"document": "<file text>"}`. For checklist documents use
the blessed codec in `plexi_sdk.state_format` (`ChecklistItem`,
`parse_checklist`, `render_checklist`): tolerant reader, canonical writer.
An unknown format fails install loudly.

State files are shared with the outside world — the CLI, agents, and editors
write them too. The contract is **disk wins**: every write (host and CLI) is
atomic temp+rename, and the host read-backs before writing — a `PersistState`
that loses to a concurrent external write is dropped, the on-disk state is
reloaded, and your app is told via the `events.StateChanged` event
(re-apply your change from there; never assume a persist landed). External
edits also arrive as `StateChanged`: the scope's values are replaced
wholesale before `update()` runs — deleted keys vanish, never a merge — and
`event.error` is set when the file exists but cannot be decoded (previous
values are kept and persists to that scope are blocked until the file is
fixed).

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

`plexi_sdk` is only importable inside Plexi's app runtime — the CPython-in-WASM
bridge adds it to `PYTHONPATH`. A bare `python3 -c "import plexi_sdk"` in a
terminal pane will fail or import a stale copy.
Test by opening the app in a pane. More traps: [`AGENTS.md`](AGENTS.md).

## Design Philosophy

- Obvious over clever — fight for the solution an agent would naturally assume.
- Simulate affordances, never lie about contracts — isolation, durability,
  persistence, and security boundaries stay explicit.
- Build primitives, not features — omit anything a developer's agent can
  trivially build atop the platform.
- Design for agents, not humans browsing docs — if it needs a README to be
  usable, the API is wrong.
