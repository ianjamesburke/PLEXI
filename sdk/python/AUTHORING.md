# Authoring Plexi Plugins

Canonical guide for building a Plexi plugin on the Python SDK v3. Every other
authoring surface (`README.md`, both `AGENTS.md` files, the scaffolded plugin
contract, the `plexi plugin init --help` block) points here; this file is the one
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
plexi plugin init myapp          # or plexi-pr-<N> plugin init myapp on a PR build
plexi plugin init --adopt <dir>  # backfill/refresh plexi.scaffold.toml for an existing plugin
plexi plugin check myapp --png-dir /tmp/myapp-shots
plexi view open myapp
```

`plexi plugin init` writes `main.py`, `manifest.toml`, `tests/test_plugin.py`,
`fixtures/state.json`, `AGENTS.md`, `.gitignore`, and `plexi.scaffold.toml`.
The scaffolded `AGENTS.md` says what each file is for. Never author a
`manifest.toml` by hand: the scaffold sets the correct `schema_version` and a
valid shape.

`main.py` is a skeleton to extend: one state value, one button, one key, and a
`Resize` handler. Its manifest declares no capabilities and it exposes no
tools. `tests/test_plugin.py` drives the button and the key, so it fails when
the handler breaks.

For pre-existing or migrated plugins that predate scaffold metadata, run
`plexi plugin init --adopt <dir>` (or `plexi plugin init --adopt` inside the plugin
directory) to write or refresh `plexi.scaffold.toml` without creating or
modifying any source files. `plexi plugin check` uses `plexi.scaffold.toml` to
verify drift across `plexi_cli_version`, `sdk_version`,
`manifest_schema_version`, `python_runtime_version`, `template_version`, and
the active `channel`/`profile_dir`.

## App Contract

A plugin is three module-level functions, with no class and no inheritance:

```python
#!/usr/bin/env python3
from __future__ import annotations

from plexi_sdk import log, state
from plexi_sdk.effects import SetState, SetTitle
from plexi_sdk.events import KeyEvent
from plexi_sdk.ui import SurfaceHeader, Column, FooterKeys, Text


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

- `init(size, args)` runs once on launch; return startup effects. `size` is
  `(width, height)`, the pane's content area in logical points. The host sends
  `Resize(width, height)` when either changes by more than half a point. A
  pane that starts before it is first painted gets `(800, 600)` and then a
  `Resize` with the real size. `plexi plugin check` and `plexi view render`
  pass the size they render at. `PluginHarness` sends no size, so `init` sees
  `(0, 0)` there.
- Persisted and seeded state is already in `plexi_sdk.state` when `init` runs.
  A `SetState` returned from `init` is merged on top, so it replaces a restored
  or seeded value for the same key. Read with `state.get("key", default)`
  instead of writing a default from `init`.
- `update(event)` runs for every input event; return effects. Never mutate in
  place — effects are data the adapter executes after `update()` returns.
- `view()` must be **pure**. Calling `state.set()` inside `view()` raises
  `RuntimeError`. All mutation flows through effects returned from `update()`.

## Keyed Component State

Reusable components own state through an explicit, stable key. Declare it once
with `component("invoice-total")`, read its value in `view()`, and return its
setter from `update()`. The SDK stores this under a reserved part of the normal
state snapshot, so restore and `persist=True` continue to use the existing
state machinery.

```python
from plexi_sdk import component

total = component("invoice-total")

def update(event):
    if event.handler_id == "invoice:add":
        return [total.set("count", total.get("count", 0) + 1)]
    return []

def view():
    count, _set_count = total.use_state("count", 0)
    return Keyed(Button(str(count), on_click="invoice:add"), key="invoice-total")
```

Use `Keyed(child, key="...")` for children generated from data. Its UI key and
the `component(...)` key are independent strings: the former identifies a child
in the rebuilt UI tree, while the latter names local state. `Keyed` does not
send its key to the host yet. Do not call a component setter in `view()`; it
returns the same deferred state effect as `state.set()` and belongs in `update()`.

## Two Rendering Modes

Pick one per plugin:

- **Declarative UI trees** — forms, lists, dashboards. `view()` returns a
  component tree (`Column`/`HStack`/widgets).
- **Canvas drawing** — games, animations, visualizations. `view()` returns a
  `Canvas([...])`. For animation, return `SetSchedulerMode("continuous", fps=60)`
  from `init()` and advance simulation state from `RenderFrame` events. Do not
  build animation loops with timers. `RenderFrame.elapsed` is the seconds since
  the previous frame, and `0.0` on the first. `SetSchedulerMode` needs no
  capability.

### Tree components and canvas commands

Tree mode lays out native host nodes and handles widget input. Use `Column`,
`HStack`, `Text`, `Button`, and `TextInput` for ordinary interfaces. Some SDK
components are sugar that compose native nodes: for example, `Actions` builds a
row of buttons and `FormField` combines a label and input. They do not add new
wire node types. Inspect `to_node()` in `plexi_sdk.ui` or the node variants in
`wit/plexi.wit` when you need to know what reaches the host.

`Canvas` sends drawing commands instead of individual interactive widgets. Use
`CanvasRect`, `CanvasCircle`, `CanvasLine`, and `CanvasText` for custom drawing;
handle interaction in the app. A canvas can also be a child in a tree. See
`apps/dev/component-gallery` for examples of tree components and their use.

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
state. `PersistState` writes the same snapshot **and** saves durable plugin state;
use it only when a key must survive a plugin restart. Both take an optional
`scope=` naming one of the plugin's declared state scopes (see `[state] scopes`
in the Manifest section); omitted means the plugin's default scope, and an
undeclared scope raises — never a silent fallback. `state.get`/`state.all`/`state.load`/`state.patch`
accept the same `scope=` argument. There are no `LogInfo` /
`LogWarn` / `LogError` effects — logging goes through the `log` module (below).
Network fetches use `HttpFetch`, not `HttpRequest`. `HttpFetch` accepts optional
`params={...}` (query parameters), `tag="..."`, and `request_id="..."`; the reply
is `events.HttpResponse` with `.status`, `.headers`, `.body`, `.error`,
`.request_id`, `.tag`, `.truncated`, `.text()`, and `.json()`.

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

Plugins can expose tools with `ExposeTools`. The Assistant can call them directly;
external MCP agents in the same workspace see them as
`<plugin_id>__<tool>` through the config printed by `plexi events mcp-config`.
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
    return tools.dispatch(event) or <the plugin's own handling>
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

Set `read_only=True` only for tools that never mutate plugin or workspace state.
For Assistant calls specifically, read-only tools run without a write-grant
prompt while every call is still recorded. Mutating Assistant calls prompt for
an allow-once or persisted narrow grant before they run.

## Cross-plugin events

Use `DeclareEventStreams` and `EmitEvent` for streams owned by the current app.
Use `SubscribeEventStreams` with another plugin's ID and declared stream names to
receive its events. The subscription result supplies the ID needed by
`UnsubscribeEventStreams`; matching events arrive as `PluginEvent` in `update()`.
Python and Rust WASM plugins share the same host registry, so publishers and
subscribers can use either runtime. Subscriptions last until unsubscribe or
the subscriber pane closes. The host permission broker may ask the user before
allowing a cross-plugin subscription. Declaring and emitting streams needs no
capability ID in the manifest.

## Focus and Visibility

The host tells each Python plugin about its own pane through four events,
delivered to `update()` in transition order, once per change:

| Event | Sent when |
|---|---|
| `PaneShown` | the pane became visible: its context is active, the host window is not minimized or occluded, no other pane is zoomed over it, and the tree atlas is closed |
| `FocusGained` | the pane became the focused pane of the active context while the host window is visible and OS-focused |
| `FocusLost` | the pane stopped being focused (focus moved, the context changed, the window lost OS focus or was hidden) |
| `PaneHidden` | the pane stopped being visible |

A focused pane is always visible, so `PaneShown` precedes `FocusGained` and
`FocusLost` precedes `PaneHidden`. A new plugin starts unshown and unfocused, so a
pane opened in view receives `PaneShown` then `FocusGained` right after `init`.
Modal overlays (command palette, approvals) do not count as focus changes.
Refresh on `FocusGained`; do not poll while hidden. Delivery continues while
the host window is hidden. `FocusChanged` is a separate host-level time-tracking
event, not a substitute. Test handlers with `PluginHarness.set_focus(bool)` and
`PluginHarness.set_visible(bool)`.

## UI Components

Import from `plexi_sdk.ui`. Read `sdk.md` (UI Components section) for the full
API; the widgets you reach for most:

<!-- drift-check:components -->
`SurfaceHeader`, `ActionBar`, `Column`, `HStack`, `Row`, `Label`, `Text`, `Heading`,
`Spacer`, `Divider`, `FooterKeys`, `SelectList`, `Pending`, `TextEdit`, `Card`,
`Section`, `Tabs`, `TabBar`, `Grid`, `Toggle`, `Switch`, `Scrollable`,
`Button`, `Badge`, `Banner`, `Actions`, `FormField`, `TextInput`,
`Checkbox`, `Radio`, `Slider`, `Select`, `DateTimePicker`, `Breadcrumb`,
`Pagination`, `DataTable`, `Table`, `KeyValue`, `CodeBlock`, `Avatar`, `Icon`,
`Tooltip`, `Accordion`, `EmptyState`, `Modal`, `Spinner`, `Skeleton`,
`ProgressBar`, `Progress`, `Canvas`, `CanvasRect`, `CanvasText`, `CanvasCircle`,
`CanvasLine`
<!-- /drift-check:components -->

Widget selection rules:

- **List + detail navigation:** use `SelectList` (and `SelectList.move_cursor(event, len(items), selected)` in `update()`). Set `on_select="pick"` to receive `UiAction("pick:<idx>")` when a row is clicked. Never reimplement list scrolling or hit-testing by hand.
- **Tables:** use `DataTable(columns=[...], rows=[...], selected=idx, on_select="row")` (`Table` is an alias). Clicking row `i` dispatches `UiAction("row:<i>")`.
- **Loading states:** wrap only the region whose fetch is in flight in
  `Pending(active=..., child=..., placeholder=Skeleton(rows=N))`. Placeholder
  is always explicit, sized to the eventual content so nothing jumps. Never
  thread a loading boolean through multiple view branches.
- **Text entry:** use `TextInput` (`TextEdit` alias) in the component tree. Never read raw keys for
  text. It is a single-line field; there is no multi-line input yet. Every edit
  arrives as `UiValueChange(handler_id, value)` with the field's `on_change`
  id. Enter arrives as `UiAction(handler_id)` with its `on_submit` id and
  carries no text, so keep the latest `UiValueChange.value` in state and read
  it on submit. Both ids default to the field's `id`; tell the two events apart
  by type. `FormField` behaves the same. In tests, `h.text_submit(id, value)`
  delivers the `UiValueChange` and `h.ui_action(id)` delivers the Enter.
- **Long content:** `Scrollable(child)` takes one child, so wrap a list in a
  `Column`: `Column([SurfaceHeader(...), Scrollable(Column(rows)),
  FooterKeys(...)], grow=True)`. A body taller than the pane then scrolls
  between the header and the footer. `apps/logs` is a working example.
- **Size-aware views:** store the size from `init` and `Resize` in state and
  branch on it in `view()`, as the scaffolded `main.py` does.
- **Forms:** a labeled field is a `FormField`; a Save/Cancel pair is an
  `Actions` (`ActionBar` alias) row. Buttons declared as plain `Column` children each take the
  column's full width and stack, so an action pair written that way reads as
  two unrelated bars — `Actions` is the row.
- **Focus:** the field that should hold the cursor declares
  `autofocus=True` (on `TextInput` or `FormField`). The host focuses it
  whenever the pane owns input and nothing else is focused, including the
  frame a revealed form first renders, so a plugin never issues a focus command
  or routes keys itself.
- **Raw drawing / games:** return `Canvas(...)` and drive state from
  `RenderFrame`.

`Canvas` defaults to `fit="fill"`, the existing SDK behavior: its coordinate
space fills the allocated pane and may scale differently on each axis. Existing
plugins need no change. Use `fit="contain"` when geometry must keep its source
aspect ratio; the host centers the canvas and leaves unused space on two sides.
This is appropriate for square game cells and circles that must remain round.

Timers remain supported for ordinary delayed and periodic work. Animation plugins
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
  pane edge (horizontal `SPACE_XL`, vertical `SPACE_MD`). A leading `SurfaceHeader`
  and a trailing `FooterKeys` are the plugin's edges, not its content: they render
  full-bleed against the pane rect while only the body between them is inset —
  so a `Column([SurfaceHeader(...), ...body..., FooterKeys(...)])` needs no layout
  code.
- **Bottom-pinned content.** A `Spacer(grow=True)` splits a column: everything
  before it flows from the top, everything after it sits flush against the
  bottom. Children after the spacer are never pushed off the pane.
  Grow spacers share a column's spare height: `[Spacer(grow=True), child,
  Spacer(grow=True)]` centers `child`.
  You no longer set root padding: `Column(padding=...)` does not affect the
  declarative tree (it survives only for legacy canvas-mode layout).
- **Inter-child spacing.** Leave `gap` unset on `Column`/`HStack` and the host
  spaces children (columns get `SPACE_MD`, rows get the tighter `SPACE_SM`).
  Pass an explicit `gap=` to override; `gap=0` packs children flush.
- **Cross-axis `align`.** On both `Column` and `Row` (`HStack`), `align` is the
  cross axis: a column places children at its left, center or right; a row
  places them at its top, center or bottom. The values are `start`, `center`
  and `end`. A `Row` defaults to `center`, a `Column` to `start`.
- **Narrow panes.** `Actions`, `Select` and `Tabs` wrap whole buttons onto
  further lines when the pane is narrow.
- **Button targets.** Buttons get a comfortable minimum click/touch size, so
  single-glyph buttons (calculator keys, toolbar chips) are never cramped.

Full-bleed pixel plugins opt out automatically: a grow `Canvas` or a GPU
`Surface` anywhere in the tree tells the host the plugin owns every pixel, so no
inset is applied (games, visualizers, video). A fixed-size `Canvas` is ordinary
flow content and stays inset.

Canvas plugins bypass the host WCAG contrast check. Use `theme.fg` for canvas text
and reserve `dim()`/`theme.muted` for fills, or text fails contrast.

`Badge`/`Banner` colors are the theme's SEMANTIC roles, not literal colors:
`accent`, `success`, `warning`, `danger`, `neutral`, plus the alias roles
`red`/`green`/`yellow` (== `danger`/`success`/`warning`). `Text`/`Label`/`Heading`
accept those semantic roles plus `"muted"` and `theme.*` hex tokens (e.g.
`theme.text_muted`, `theme.red`). There is no `blue`.
An invalid value raises `ValueError` at construction, not at host render. A
bespoke color needs `PluginPalette(dark=..., light=...)` — never a raw hex or
CSS name inline.

## Keyboard Conventions

Keys arrive in `update(event)` as `KeyEvent`. Key strings are lowercase
canonical — never match `"Enter"`, `"Escape"`, `"plus"`, `"minus"`, `"equals"`,
`"slash"`, or `"asterisk"`.
Command-key chords are host shortcuts and never reach plugins. App shortcuts may
use unmodified keys and the Ctrl, Shift, or Alt modifier fields.
`event.pressed` is `True` on key-down and `False` on key-up. Track held keys by
adding on press and removing on release; do not toggle state on every event.

| Physical key | `event.key` |
|---|---|
| Enter / Return | `"enter"` |
| Escape | `"escape"` |
| Backspace | `"backspace"` |
| Delete | `"delete"` |
| Space | `"space"` |
| Arrow keys | `"up"` / `"down"` / `"left"` / `"right"` |
| Home / End | `"home"` / `"end"` |
| Page Up / Page Down | `"pageup"` / `"pagedown"` |
| Insert | `"insert"` |
| Tab | `"tab"` |
| Letters | `"a"`–`"z"` (lowercase) |
| Digits (main row or numpad) | `"0"`–`"9"` |
| Plus / Minus / Multiply / Divide / Equals | `"+"` / `"-"` / `"*"` / `"/"` / `"="` |
| Function keys | `"f1"`–`"f12"` |

Convention: Enter = open/confirm. Escape (+ optional Backspace) = exit/cancel.
Every focused sub-view must be escapable.

## State

Keep state in `plexi_sdk.state` and update it by returning `SetState` /
`state.set(...)` / `state.patch(...)` from `update(event)`:

```python
state.get("key", default)         # read runtime state (also valid inside view())
state.load({"count": 0})          # merge defaults with stored state
state.set("key", value)           # returns a SetState effect — does NOT mutate now
state.patch(count=1)              # returns SetState({"count": 1}) (or PersistState with persist=True)
state.all()                       # every key, decoded
```

`view()` reads state but must never write it.

State restored from disk, or seeded by `plexi view render --state` and
`fixtures/state.json`, is loaded before `init` runs (see App Contract).

## Logging

Log through the frame with `plexi_sdk.log`. App logs forward into the host log
tagged `plugin::<plugin_id>`.

- `log.debug` — detailed state/render/event diagnostics.
- `log.info` — init, user actions, state transitions.
- `log.warn` — recoverable fallbacks or ignored input.
- `log.error` — unrecoverable failures, before returning or surfacing them.

Each takes one string: `log.info(f"saved {count} rows")`. There are no format
arguments or keyword fields.

Always log at init and key state transitions; log escapes and errors from
`update(event)`.

## Manifest

The scaffold writes a valid `manifest.toml`; the shape lives in the template at
[`plexi_sdk/templates/manifest.toml`](plexi_sdk/templates/manifest.toml). A
`[plugin] type = "plugin"` with a `.py` entry launches through the CPython-in-WASM
adapter (stint 0285). Declare capabilities under `[plugin.capabilities]`. The
accepted IDs and what each allows are the "Capability registry" table in
[`plugin-manifest.md`](../../website/src/content/docs/plugin-manifest.md),
generated from `Capability::ALL` in `src/app/permissions.rs`. An ID that is
not in the table, or that the table marks Reserved, fails `plexi plugin check`
and install.

`[[contributes.views]]` declares the views a plugin offers:

```toml
[[contributes.views]]
id = "myapp.main"
default = true

[[contributes.views]]
id = "myapp.settings"
```

Each `id` starts with the plugin id and a dot, and is unique in the manifest.
At most one view is `default`. Opening a view id opens that view. Opening the
plugin id opens its only view, or the default one; with several views and no
default it fails and lists the declared ids. All views run the same entry
file. The SDK has no accessor for which view a pane shows, and
`plexi view render <path>` and `plexi plugin check` render the default view
only.

`[launch] on_launch` declares what the host does when your plugin is launched
while an instance already exists: `focus_existing` (one instance globally —
relaunch focuses it, jumping context if needed), `focus_existing_in_context`
(one instance per context), or `always_new` (default — every launch spawns a
fresh instance). An unknown value fails install loudly. Under `always_new`,
duplicate instances are fully independent: each receives events through its
own event-bus subscriptions, and instance identity is the host-stamped pane
id — never an id your plugin assigns itself. Don't try to self-dedup or key
shared state on a self-chosen id; declare `on_launch` and let the host resolve
launches. Note that when a relaunch is deduped to an existing instance, any
`args` (or cwd) that relaunch carried are **not** delivered to the running
instance — the host focuses it as-is (delivering relaunch args to a live
instance via a bus event is future work).

`[state] scopes` declares which state scopes your plugin addresses, ordered —
the first entry is the default:

```toml
[state]
scopes = ["global", "context"]
```

Omitting `[state]` gives `["global"]`. The host owns path construction:
`global` state lives in `~/.plexi/app_states/<plugin_id>.<ext>` (channel-neutral,
cross-project), `context` state in `<context_root>/.plexi/app_states/`,
resolved against the pane's context root at call time — so
`plexi context set-root` immediately redirects where context-scoped state
lands. Context-scoped state is gitignored by the host (`.plexi/.gitignore`
gains `app_states/` automatically): plugin state is personal, single-user, local
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
reloaded, and your plugin is told via the `events.StateChanged` event
(re-apply your change from there; never assume a persist landed). External
edits also arrive as `StateChanged`: the scope's values are replaced
wholesale before `update()` runs — deleted keys vanish, never a merge — and
`event.error` is set when the file exists but cannot be decoded (previous
values are kept and persists to that scope are blocked until the file is
fixed).

## Approved CLI Operations

A plugin can run a known command-line program on the person's computer, but only
through operations its package declares and the person approves. This is the
`cli_operations` experiment (`plexi experiments enable cli_operations`) and
needs the `cli.operations` capability. It is not a shell: the plugin names an
operation and supplies typed values, nothing else.

Declare each operation in `manifest.toml`:

```toml
[[cli_operation]]
id = "task.set_priority"
executable = "stint"                     # bare name or absolute path, never plugin-chosen
argv = ["set", "--priority={priority}", "--", "{task}"]
timeout_ms = 5000
[cli_operation.output]
format = "json"                          # or "text"
max_bytes = 65536
[cli_operation.authority]                # what the program is meant to reach
mode = "mutate"                          # "read" or "mutate"; separate approvals
fs_write = ["workspace"]
[cli_operation.slots.priority]
type = "enum"
values = ["p0", "p1", "p2", "p3", "p4"]
[cli_operation.slots.task]
type = "string"
max_len = 64
```

Each `argv` element is a literal, a whole-element `{slot}`, or an option-joined
`--name={slot}`. A slot in a bare position (before any `--`) can never render a
leading `-`; put free text after a `--` element or join it to its option.
`plexi plugin check` and install reject a definition whose mapping the host cannot
guarantee. Slot types are `string` (`max_len`), `int` (`min`, `max`), `enum`
(`values`) and `bool`.

Request one with `CliOperation(request_id, operation_id, values)` and read the
answer in `events.CliOperationResult`, matched on `request_id`. A request can wait
for the person's decision, so keep it outstanding; `CliOperationCancel` withdraws
it. Failures are typed (`events.CliOperationResult.error_code`): only
`nonzero_exit`, `timeout`, `output_too_large` and `cancelled` mean the program
may have run.

The host picks the program, working directory (the workspace) and a minimal
fixed environment. Every call is checked against an approval bound to your
package's exact content, the workspace, the operation definition, and the
program's path and bytes: change any of them and the person is asked again. The
approval says the program runs with their full account authority and is not
confined; `authority` is a declaration for review, not a sandbox. The program
must be a self-contained native executable. Scripts are refused because their
interpreter would be unbound.

## Dev / Test Loop

Test-first. Define done by the test, not the code.

- Add or update `tests/test_plugin.py` (PluginHarness) for every behavior change
  before touching plugin code, then run `plexi plugin test .`.
- Exercise behavior through actions and keys, not only by reading code:
  `plexi pane action <pane-id> <handler-id>` and `plexi pane key <pane-id> <key>`.
- Hot reload is part of the loop. New plugins set `watch = true`; after
  `plexi view open .`, edit source and confirm the same pane id updates without
  reopening (check `plexi pane state <pane-id>` and the host log's `hot_reload`
  lines).
- Final gate: `plexi plugin check .` renders the size matrix and validates the SDK
  v3 shape. There is no `plexi plugin build`.
- `plexi plugin check` also reads `fixtures/state.json` when it exists. The file
  must be one JSON object. The check seeds it, renders once, and fails unless
  at least one string or number from the fixture shows up in the frame, then
  clicks the handlers it finds. Keep the fixture's keys the ones `view()`
  reads, and do not overwrite them from `init`. With no fixture the seeded
  probe is skipped.
- `--state` on `plexi view render` is read relative to the current directory,
  not the plugin directory. The examples below run from inside the plugin
  directory.

Make the channel explicit when validating — `PLEXI_CHANNEL` leaks into plugin
tooling:

```sh
PLEXI_CHANNEL=alpha plexi plugin check . --png-dir render-output/check
PLEXI_CHANNEL=pr-123 plexi plugin check . --png-dir render-output/check   # or: plexi-pr-123 plugin check .
PLEXI_CHANNEL=alpha plexi view render . --state fixtures/state.json      # JSON frame tree
PLEXI_CHANNEL=alpha plexi view render . --png --output render-output/shot.png
```

When probing a running host from outside its Plexi pane, set the matching
socket explicitly:
`PLEXI_SOCKET=$HOME/.plexi-alpha/notify.sock PLEXI_CHANNEL=alpha plexi ...`.
Inspect `~/.plexi-alpha/plexi.log` (or `~/.plexi-pr-<N>/plexi.log`) for plugin logs.

## Traps

`plexi_sdk` is only importable inside Plexi's plugin runtime — the CPython-in-WASM
bridge adds it to `PYTHONPATH`. A bare `python3 -c "import plexi_sdk"` in a
terminal pane will fail or import a stale copy.
Test by opening the plugin in a pane. More traps: [`AGENTS.md`](AGENTS.md).

## Design Philosophy

- Obvious over clever — fight for the solution an agent would naturally assume.
- Simulate affordances, never lie about contracts — isolation, durability,
  persistence, and security boundaries stay explicit.
- Build primitives, not features — omit anything a developer's agent can
  trivially build atop the platform.
- Design for agents, not humans browsing docs — if it needs a README to be
  usable, the API is wrong.
