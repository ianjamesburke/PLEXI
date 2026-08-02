# Plexi WASM apps

The Rust SDK wraps the `plexi-app` world in [`wit/plexi.wit`](https://github.com/ianjamesburke/PLEXI/blob/alpha/wit/plexi.wit). See [the runtime reference](https://github.com/ianjamesburke/PLEXI/blob/alpha/docs/wasm-runtime.md) for the host architecture and security model.

## Create an app

Install `cargo-component`, then scaffold, build, and open an app:

```sh
cargo install cargo-component
plexi app init --wasm counter --global
```

Change to the exact app path printed by `app init`, then run:

```sh
cargo component build --release --target wasm32-wasip2
plexi app open .
```

The scaffold is a Cargo workspace. It carries a generated SDK snapshot under `.plexi-sdk/`, and its `wit` symlink points at the matching embedded WIT contract. This keeps the app build independent of the Plexi source checkout that produced the CLI. `cargo-component` writes the loadable component to `target/wasm32-wasip1/release/<crate>.wasm`; `manifest.toml` uses that path.

Implement `App` and export one instance:

```rust
use plexi_wasm_sdk::{export_app, ui, App, Effect, InputEvent, UiTree};

#[derive(Default)]
struct Notes;

impl App for Notes {
    fn update(&mut self, _event: InputEvent) -> Vec<Effect> { Vec::new() }

    fn view(&self) -> UiTree {
        let mut tree = ui::Tree::new();
        let root = tree.text("title", "Notes");
        tree.finish(root)
    }
}

export_app!(Notes::default());
```

## Lifecycle and state

Plexi calls `init` once, then calls `update` for input and effect results. It
calls immutable `view` whenever it needs a new tree. `view` must not call host
imports.

`InitContext` contains the initial size, launch arguments, and persisted state
snapshot. `context.get(key)` returns the initial bytes for one key. The `state`
module exposes live `get`, `set`, `delete`, `list_prefix`, and `snapshot` host
imports. State values are byte arrays; choose and document an encoding in the
app when values are not plain UTF-8.

## Effects

The `effects` module constructs every effect in the standard app world:

- `file_read(path)` and `file_write(path, bytes)` require the matching scoped
  filesystem capability. Results arrive as `FileReadResult` and
  `FileWriteResult`.
- `http_fetch(url)` performs a GET and requires a matching network capability.
  The response arrives as `HttpResponse`. Build `HttpFetchEffect` directly for
  other methods, headers, or request bodies.
- `ai_query(AiQueryEffect)`, `declare_event_streams(Vec<EventStreamDecl>)`,
  `emit_event(EmitEventEffect)`, and
  `subscribe_event_streams(SubscribeEventStreamsEffect)` expose their full WIT
  payload records. `unsubscribe_event_streams(request_id, subscription_id)`
  stops a pane-session subscription.
- `declare_tools(Vec<ToolDecl>)` exposes tools to the Assistant and same-workspace
  external MCP agents, and
  `tool_result(call_id, output_json, error)` completes a matching call.
- `set_timer(id, delay_ms, repeat)`, `cancel_timer(id)`, and
  `get_system_stats()` return timer and system-stat events through `update`.
- `set_title`, `set_status`, and `close_self` control the current pane.
- `request_capability(id)` starts the host permission flow. The result is
  `CapabilityGranted` or `CapabilityDenied`.
- `clipboard_read()` and `clipboard_write(text)` require `clipboard.read` and
  `clipboard.write`. Results arrive as `ClipboardReadResult` and
  `ClipboardWriteResult`.
- `notify(title, body)` requires `notify` and returns `NotifyResult`.
- `spawn(app_id)` requires `spawn.app` and returns `SpawnResult`. Construct
  `SpawnEffect` directly when the launch needs a layout or arguments.
- `open_file_picker(request_id, filter, multiple, mode)` requires `fs.pick`
  and shows a host file dialog (`FilePickerMode::Open` / `Folder` / `Save`).
  The result arrives as `FilePicked` — its absolute paths are registered as
  scoped fs grants for this pane, so pass them verbatim to `file_read` /
  `file_write` (a folder grant covers the subtree) — or `FilePickCancelled`
  (dismissed or capability denied; no grant is created). Grants last for the
  pane's lifetime. Headless drivers script the dialog by launching the host
  with `PLEXI_PICKER_SCRIPT` (JSON array of `{"paths": [...]}` /
  `{"cancel": true}` outcomes, consumed in order per pane).

Effect results arrive later through `App::update` as `InputEvent` variants. The SDK re-exports the generated WIT records when a helper needs the full payload, such as `AiQueryEffect` or `EmitEventEffect`.

## Event streams

Declare and emit streams owned by the current app, or subscribe to streams
declared by another app:

```rust
use plexi_wasm_sdk::{effects, Effect, EventStreamDecl, SubscribeEventStreamsEffect};

fn event_effects() -> Vec<Effect> {
    vec![
        effects::declare_event_streams(vec![EventStreamDecl {
            name: "counter.changed".into(),
            schema_json: r#"{"type":"object"}"#.into(),
            description: Some("Counter value changed".into()),
        }]),
        effects::subscribe_event_streams(SubscribeEventStreamsEffect {
            request_id: "subscribe-counter".into(),
            app_id: "counter".into(),
            event_names: vec!["counter.changed".into()],
            payload_mode: "full".into(),
            trigger_mode: "conversation".into(),
            resource_id: None,
        }),
    ]
}
```

`EventSubscriptionResult` returns the subscription ID. Matching events arrive
as `InputEvent::AppEvent`, including the publisher ID, stream name, host event
ID, resource ID, and the payload selected by `payload_mode`. Subscriptions are
session-scoped and disappear when the pane closes. The host permission broker
may ask the user before allowing a cross-app subscription.

## App tools

Declare the complete tool set during `init`, handle `InputEvent::ToolCall` in
`update`, and return exactly one result with the matching call ID:

```rust
use plexi_wasm_sdk::{effects, Effect, InputEvent, ToolDecl};

fn declare_tools() -> Effect {
    effects::declare_tools(vec![ToolDecl {
        name: "counter.read".into(),
        description: "Read the current counter value".into(),
        input_schema_json: r#"{"type":"object"}"#.into(),
        output_schema_json: r#"{"type":"object","properties":{"count":{"type":"integer"}},"required":["count"]}"#.into(),
        timeout_ms: Some(2_000),
        read_only: true,
    }])
}

fn handle_tool(event: InputEvent) -> Vec<Effect> {
    match event {
        InputEvent::ToolCall(call) if call.name == "counter.read" => vec![
            effects::tool_result(call.call_id, Some(r#"{"count":0}"#.into()), None),
        ],
        _ => vec![],
    }
}
```

Both schemas are JSON Schema encoded as strings in WIT. Invalid schema JSON is
rejected at declaration time. `read_only` must be true only when a call cannot
mutate app or workspace state. External MCP agents in the same workspace see
tools namespaced as `<app_id>__<tool>` through the config printed by
`plexi events mcp-config`. For Assistant calls specifically, read-only calls
are auto-granted unless an explicit deny applies. Mutating Assistant calls use
the normal app-connector permission prompt, and denied calls never enter the
component. The host audit logs every allowed invocation.

## UI nodes

`ui::Tree` assigns numeric arena IDs. Every builder takes a stable string key
first and returns the numeric node ID used by parent nodes.

| Builder | Remaining arguments |
|---|---|
| `empty` | none |
| `text` | text |
| `button` | label, click handler ID |
| `text_input` | value, placeholder, change handler ID, submit handler ID |
| `row`, `column` | child node IDs |
| `progress_bar` | value, maximum |
| `badge` | text |
| `list_view` | item node IDs, selected index, optional select handler ID |
| `scroll` | child node ID, horizontal flag |
| `padding` | child node ID, uniform padding |
| `canvas` | width, height, canvas commands |
| `divider` | none |
| `space` | logical pixels |
| `surface` | width, height, optional host texture handle |

Builders set common defaults. The generated node records and enums are re-exported from the crate for exact styling or payload control. Call `finish(root_id)` after adding the root node.

## Capabilities

Declare requested capabilities in `manifest.toml`. The host rejects effects that the app did not declare and asks the user before granting sensitive access.

```toml
[app.capabilities]
capabilities = ["clipboard.write", "notify", "spawn.app"]

[app.capabilities.wasm]
required = []
optional = []
```

Use scoped filesystem and network capability IDs for file and HTTP effects.
The standard effect IDs are `ai.query`, `clipboard.read`, `clipboard.write`,
`notify`, and `spawn.app`. Imported state and pipe interfaces use
`state:read-write` and `pipe.open`. GPU and audio worlds declare `gpu.render`,
`audio.playback`, or `audio:record` as required by the app.

The host asks before granting an unknown session capability and returns the
decision through `update`. Capability names and their host behavior are defined
in [the runtime reference](https://github.com/ianjamesburke/PLEXI/blob/alpha/docs/wasm-runtime.md). Keep the manifest list
limited to effects and imported interfaces the app uses.
