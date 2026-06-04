# PGAP Protocol Reference

Complete reference for the Plexi Graphics & Application Protocol (PGAP v3).
Every type on the wire is documented here so an agent can build a working app
without reading Rust or Python source.

**Wire format:** newline-delimited JSON over stdin/stdout. Binary data travels
on typed pipes, not stdio.

**Serde convention:** all `type` tags are `snake_case`. Unknown fields are
ignored by the host. Missing required fields fail deserialization loudly.

---

## Table of Contents

1. [Protocol Handshake](#protocol-handshake)
2. [PlexiEvent (host to app)](#plexievent-host-to-app)
3. [DrawCommand (app to host)](#drawcommand-app-to-host)
   - [RenderCommand (draw primitives)](#rendercommand)
   - [AppRequest (host requests)](#apprequest)
   - [ControlCommand (inline commands)](#controlcommand)
4. [UiNode (component tree)](#uinode-component-tree)
   - [L0 Primitives](#l0-primitives)
   - [L1 Sugar](#l1-sugar)
   - [L1 Layout Components](#l1-layout-components)
5. [Capabilities](#capabilities)
6. [Supporting Types](#supporting-types)
7. [Python SDK Components](#python-sdk-components)
8. [Common Patterns](#common-patterns)

---

## Protocol Handshake

1. Host spawns the app binary.
2. Host sends exactly one `Init` event.
3. App sends `DrawCommand::Ready` once after receiving `Init`.
4. Each frame: host sends `Render`; app replies with draw commands + `FrameDone`.
5. Input events (`Key`, `Click`, `Command`) arrive between frames.
6. Out-of-frame commands (`Notify`, `SecretGet`, etc.) may arrive any time.
7. On close: host sends `Shutdown`; app must exit within a short timeout.

---

## PlexiEvent (host to app)

Events are tagged JSON objects: `{"type": "<snake_case>", ...}`.

### Init

Sent exactly once on startup. App must reply with `Ready`.

| Field | Type | Required | Description |
|---|---|---|---|
| `protocol` | string | yes | Protocol version, e.g. `"pgap/3"` |
| `app_id` | string | yes | Stable identifier for this app instance |
| `workspace_root` | string | yes | Workspace root path; all SecretGet calls scoped here |
| `capabilities` | string[] | yes | Granted capabilities, e.g. `["audio.record", "fs.read"]` |
| `feature_flags` | string[] | yes | Additive feature flags; unknown flags ignored |
| `compact_threshold` | float | no | Default 280.0 |
| `regular_threshold` | float | no | Default 480.0 |
| `theme` | object | no | Map of `role -> "#rrggbb"` for host theme colors |

### Render

Request a new frame. App replies with draw commands terminated by `FrameDone`.

| Field | Type | Required | Description |
|---|---|---|---|
| `frame_id` | u64 | yes | Must be echoed back in `FrameDone` |
| `rect` | Rect | yes | Current surface rect `{x, y, w, h}` |

### Resize

| Field | Type | Required | Description |
|---|---|---|---|
| `width` | float | yes | New surface width |
| `height` | float | yes | New surface height |

### Key

| Field | Type | Required | Description |
|---|---|---|---|
| `key` | string | yes | Key name (e.g. `"a"`, `"Enter"`, `"ArrowUp"`) |
| `modifiers` | Modifiers | yes | `{shift, ctrl, alt, cmd}` booleans |

### Click

| Field | Type | Required | Description |
|---|---|---|---|
| `x` | float | yes | Logical X coordinate within app surface |
| `y` | float | yes | Logical Y coordinate within app surface |
| `button` | string | yes | `"primary"` or `"secondary"` |

### MouseDown / MouseUp

| Field | Type | Required | Description |
|---|---|---|---|
| `x` | float | yes | Logical X |
| `y` | float | yes | Logical Y |
| `button` | string | yes | `"primary"` or `"secondary"` |
| `modifiers` | Modifiers | yes | `{shift, ctrl, alt, cmd}` booleans |

### MouseMove

Only fires when app opts in via `SetMouseTracking { enabled: true }`.

| Field | Type | Required | Description |
|---|---|---|---|
| `x` | float | yes | Logical X |
| `y` | float | yes | Logical Y |
| `buttons` | string[] | yes | Currently held buttons |
| `modifiers` | Modifiers | yes | `{shift, ctrl, alt, cmd}` booleans |

### Command

| Field | Type | Required | Description |
|---|---|---|---|
| `text` | string | yes | User-submitted command bar text |

### CapabilityDecision

Response to a runtime `CapabilityRequest`.

| Field | Type | Required | Description |
|---|---|---|---|
| `request_id` | string | yes | Matches the request |
| `granted` | bool | yes | Whether the capability was granted |

### SecretValue

| Field | Type | Required | Description |
|---|---|---|---|
| `key` | string | yes | Secret key name |
| `value` | string? | yes | Secret value, or null when denied |

### RunUpdate

| Field | Type | Required | Description |
|---|---|---|---|
| `run_id` | string | yes | Run identifier |
| `status` | string | yes | One of: `"pending"`, `"running"`, `"blocked_on_user"`, `"completed"`, `"failed"` |
| `payload` | JSON | yes | Arbitrary payload |

### PipeMessage

| Field | Type | Required | Description |
|---|---|---|---|
| `pipe_id` | string | yes | Pipe identifier |
| `payload` | JSON | yes | JSON payload |

### PathChanged

| Field | Type | Required | Description |
|---|---|---|---|
| `cwd` | string | yes | New CWD from a pane group member |

### Suspend / Resume / Shutdown

No fields. `Suspend` fires when backgrounded, `Resume` when foregrounded, `Shutdown` when closing.

### Theme

| Field | Type | Required | Description |
|---|---|---|---|
| `colors` | object | yes | Updated `role -> "#rrggbb"` map |

### AppSpawned

| Field | Type | Required | Description |
|---|---|---|---|
| `pane_id` | u64 | yes | Pane ID of the new app |
| `type_id` | string | yes | App type identifier |

### PaneSpawned

| Field | Type | Required | Description |
|---|---|---|---|
| `pane_id` | u64 | yes | Pane ID of the new pane |
| `request_id` | string? | no | Matches SpawnPane request_id |

### PaneSpawnError

| Field | Type | Required | Description |
|---|---|---|---|
| `reason` | string | yes | Human-readable error |
| `request_id` | string? | no | Matches SpawnPane request_id |

### InjectState

| Field | Type | Required | Description |
|---|---|---|---|
| `payload` | JSON | yes | Persisted app state payload |

### HttpResponse

| Field | Type | Required | Description |
|---|---|---|---|
| `request_id` | string | yes | Matches the HttpRequest |
| `status` | u16 | yes | HTTP status code |
| `body` | string | yes | Response body |
| `error` | string? | no | Error message if request failed |

### NotifyAction

| Field | Type | Required | Description |
|---|---|---|---|
| `notify_id` | string | yes | Matches the notification |
| `action_label` | string | yes | `"acknowledge"`, option label, `"submit"`, or `"cancel"` |
| `value` | string? | no | Option value, typed text, or absent |

### Timer

| Field | Type | Required | Description |
|---|---|---|---|
| `timer_id` | string | yes | Matches the SetTimer request |

### ImageLoaded

| Field | Type | Required | Description |
|---|---|---|---|
| `handle` | string | yes | Image handle from LoadImage |
| `status` | string | yes | `"ok"` or `"error"` |
| `message` | string? | no | Error detail on failure |

### TextMeasured

| Field | Type | Required | Description |
|---|---|---|---|
| `request_id` | string | yes | Matches the MeasureText request |
| `width` | float | yes | Measured width in logical pixels |
| `height` | float | yes | Measured height in logical pixels |

### TextWrappedMeasured

| Field | Type | Required | Description |
|---|---|---|---|
| `request_id` | string | yes | Matches the MeasureTextWrapped request |
| `height` | float | yes | Wrapped text height in logical pixels |

### TextSubmitted

| Field | Type | Required | Description |
|---|---|---|---|
| `id` | string | yes | Matches the TextInput `id` |
| `value` | string | yes | Submitted text. Host clears the buffer after this. |

### Paste

| Field | Type | Required | Description |
|---|---|---|---|
| `text` | string | yes | OS clipboard contents (UTF-8) |

### AiResponse

| Field | Type | Required | Description |
|---|---|---|---|
| `request_id` | string | yes | Matches the AiQuery |
| `content` | string? | yes | Response text (null on error) |
| `tokens_in` | u32 | yes | Input tokens (0 on error) |
| `tokens_out` | u32 | yes | Output tokens (0 on error) |
| `error` | string? | yes | Error message (null on success) |

### AiStreamChunk

| Field | Type | Required | Description |
|---|---|---|---|
| `request_id` | string | yes | Matches the AiQuery |
| `delta` | string | yes | Incremental text chunk |
| `done` | bool | no | Default false. True on last chunk before AiResponse. |

### ToolCall

| Field | Type | Required | Description |
|---|---|---|---|
| `call_id` | string | yes | Must be echoed in ToolResult |
| `name` | string | yes | Tool name |
| `input_json` | string | yes | JSON-encoded tool input |

### McpToolCall

| Field | Type | Required | Description |
|---|---|---|---|
| `call_id` | string | yes | Must be echoed in McpToolResult |
| `tool_name` | string | yes | Tool name |
| `arguments` | JSON | yes | Tool arguments |

### AudioDevicesListed

| Field | Type | Required | Description |
|---|---|---|---|
| `request_id` | string | yes | Matches ListAudioDevices |
| `inputs` | AudioDevice[] | yes | Input devices |
| `outputs` | AudioDevice[] | yes | Output devices |
| `error` | string? | no | Enumeration error |

AudioDevice: `{id: string, name: string, default: bool}`

### AudioCaptureStarted

| Field | Type | Required | Description |
|---|---|---|---|
| `pipe_id` | string | yes | Binary pipe for PCM frames |
| `sample_rate` | u32 | yes | Negotiated sample rate |
| `channels` | u16 | yes | Negotiated channel count |
| `buffer_size` | u32 | yes | Negotiated buffer size |
| `device_name` | string | yes | Opened device name |

### AudioCaptureError

| Field | Type | Required | Description |
|---|---|---|---|
| `pipe_id` | string | yes | Pipe that failed |
| `error` | string | yes | Error message |

### MidiDevicesListed

| Field | Type | Required | Description |
|---|---|---|---|
| `request_id` | string | yes | Matches ListMidiDevices |
| `inputs` | MidiPort[] | yes | Input ports |
| `outputs` | MidiPort[] | yes | Output ports |
| `error` | string? | no | Enumeration error |

MidiPort: `{id: string, name: string, default: bool}`

### MidiInputOpened / MidiInputError / MidiSendError

MidiInputOpened: `{pipe_id, port_id, port_name}`
MidiInputError: `{pipe_id, error}`
MidiSendError: `{port_id, error}`

### VideoOpenAck

| Field | Type | Required | Description |
|---|---|---|---|
| `request_id` | string | yes | Matches OpenVideo |
| `handle_id` | u64 | yes | Opaque handle for SetVideoState/CloseVideo |
| `width` | u32 | yes | Video width |
| `height` | u32 | yes | Video height |
| `fps` | float | yes | Frame rate |
| `duration_ms` | u64 | yes | Total duration in ms |

### VideoOpenError

| Field | Type | Required | Description |
|---|---|---|---|
| `request_id` | string | yes | Matches OpenVideo |
| `error` | string | yes | Error message |

### LinkedTerminalReady

| Field | Type | Required | Description |
|---|---|---|---|
| `request_id` | string | yes | Matches RequestLinkedTerminal |
| `terminal_pane_id` | u64 | yes | Pane ID of the linked terminal |

### CommandPreview

| Field | Type | Required | Description |
|---|---|---|---|
| `request_id` | string | yes | Matches RequestCommandPreview |
| `command` | string | yes | Command string |
| `would_run_in_cwd` | string | yes | Terminal's CWD at request time |

### NavBack

| Field | Type | Required | Description |
|---|---|---|---|
| `view_id` | string | yes | The view being popped |

### FilePicked / FilePickCancelled

FilePicked: `{request_id: string, paths: string[]}`
FilePickCancelled: `{request_id: string}`

### StreamChunk

| Field | Type | Required | Description |
|---|---|---|---|
| `correlation_id` | string | yes | Matches StreamProcess |
| `channel` | string | yes | `"stdout"`, `"stderr"`, or `"structured"` |
| `bytes` | u8[] | yes | Raw byte array (JSON array of 0-255) |

### StreamEnd

| Field | Type | Required | Description |
|---|---|---|---|
| `correlation_id` | string | yes | Matches StreamProcess |
| `exit_code` | i32 | yes | Process exit code |

### ScrollOffset

| Field | Type | Required | Description |
|---|---|---|---|
| `id` | string | yes | Matches BeginScroll id |
| `offset_y` | float | yes | Current vertical offset (>= 0) |

### Scroll

| Field | Type | Required | Description |
|---|---|---|---|
| `delta_y` | float | yes | Raw scroll delta (positive = scroll up) |

### ListSelect / ListActivate

| Field | Type | Required | Description |
|---|---|---|---|
| `id` | string | yes | Matches the list_view id |
| `index` | usize | yes | Selected/activated item index |

### ComponentEvent

| Field | Type | Required | Description |
|---|---|---|---|
| `node_id` | string | yes | Matches the Interactive/Button/Input node_id |
| `event_type` | string | yes | `"click"`, `"hover_enter"`, `"hover_exit"`, `"submit"`, `"change"` |
| `payload` | JSON? | no | e.g. `{"value": "text"}` for Input change events |

### PipeOpened / PipeOverrun

PipeOpened: `{pipe_id: string, socket_path: string}`
PipeOverrun: `{pipe_id: string, dropped_frames: u64}`

### ContextStateResponse

| Field | Type | Required | Description |
|---|---|---|---|
| `state` | ContextState | yes | Rolled-up context state |

---

## DrawCommand (app to host)

`DrawCommand` is the top-level wire type. It is an untagged union of three
inner enums, all using `{"type": "<snake_case>", ...}`. The `type` field is
globally unique across all three enums.

```
DrawCommand = RenderCommand | AppRequest | ControlCommand
```

---

### RenderCommand

Draw primitives that go to the pending frame buffer.

#### Rect

Fill a rectangle.

```json
{"type": "rect", "x": 0, "y": 0, "w": 800, "h": 600, "fill": "#1e1e2e", "radius": 0.0}
```

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `x` | float | yes | | Left edge |
| `y` | float | yes | | Top edge |
| `w` | float | yes | | Width |
| `h` | float | yes | | Height |
| `fill` | string | yes | | Hex color (e.g. `"#1e1e2e"`) |
| `radius` | float | no | 0.0 | Corner radius |

#### Text

Draw text at a position.

```json
{"type": "text", "x": 20, "y": 20, "text": "Hello", "size": 14.0, "color": "#cdd6f4", "monospace": false, "bold": false, "align": "top_left", "max_width": null, "elide": false, "selectable": false}
```

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `x` | float | yes | | X position |
| `y` | float | yes | | Y position |
| `text` | string | yes | | Text content |
| `size` | float | yes | | Font size in pt |
| `color` | string | yes | | Hex color |
| `monospace` | bool | no | false | Use monospace font |
| `bold` | bool | no | false | Bold weight |
| `align` | string | no | `"top_left"` | `"top_left"` or `"center"` |
| `max_width` | float? | yes | | Clip width (null = no clip) |
| `elide` | bool | yes | | Append `...` at clip point |
| `selectable` | bool | yes | | Allow text selection (required, no default) |
| `max_lines` | u32? | no | null | Max line count |

#### Line

```json
{"type": "line", "x1": 0, "y1": 0, "x2": 100, "y2": 100, "color": "#cdd6f4", "width": 1.0}
```

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `x1`, `y1` | float | yes | | Start point |
| `x2`, `y2` | float | yes | | End point |
| `color` | string | yes | | Hex color |
| `width` | float | no | 1.0 | Stroke width |

#### Circle

```json
{"type": "circle", "cx": 50, "cy": 50, "r": 20, "fill": "#cdd6f4"}
```

Supports alpha via 8-digit hex: `#rrggbbaa`.

#### Arc

Filled pie slice. Angles in radians, clockwise from east.

```json
{"type": "arc", "cx": 50, "cy": 50, "r": 20, "start_angle": 0.0, "end_angle": 3.14, "fill": "#cdd6f4"}
```

#### Image

```json
{"type": "image", "src": "handle-uuid-or-path", "x": 0, "y": 0, "w": 200, "h": 150, "fit": "contain"}
```

`fit`: `"contain"` (default), `"cover"`, or `"fill"`.

#### Avatar

Circular clipped image.

```json
{"type": "avatar", "src": "handle-uuid", "cx": 50, "cy": 50, "radius": 20}
```

#### Skeleton

Animated shimmer placeholder.

```json
{"type": "skeleton", "x": 0, "y": 0, "w": 200, "h": 20, "radius": 4.0}
```

#### Markdown

Host-rendered markdown.

```json
{"type": "markdown", "x": 0, "y": 0, "w": 400, "text": "# Hello\n\nWorld", "base_size": 14.0, "color": "#cdd6f4"}
```

#### TextInput

Host-owned text input field. Host sends `TextSubmitted` on Enter.

```json
{"type": "text_input", "id": "search", "x": 0, "y": 0, "w": 300, "h": 24, "placeholder": "Search...", "multiline": false}
```

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `id` | string | yes | | Stable identifier for the input buffer |
| `x`, `y` | float | yes | | Position |
| `w` | float | yes | | Width |
| `h` | float | no | 24.0 | Height |
| `placeholder` | string | yes | | Placeholder text |
| `multiline` | bool | no | false | Multi-line mode (Shift+Enter = newline) |

#### Badge

Host-measured pill badge.

```json
{"type": "badge", "x": 0, "y": 20, "label": "NEW", "fill": "#89b4fa", "fg": "#1e1e2e", "font_size": 11.0, "radius": 6.0}
```

#### KeyChip

Host-measured keycap chip.

```json
{"type": "key_chip", "x": 0, "y": 0, "label": "Enter", "font_size": 11.0}
```

#### KeyChipRow

Horizontal row of keycap chips with optional description.

```json
{"type": "key_chip_row", "x": 0, "y": 0, "keys": ["Cmd", "K"], "description": "Command palette", "font_size": 11.0}
```

#### Shortcuts

Multi-group shortcut row with wrap.

```json
{"type": "shortcuts", "x": 0, "y": 0, "max_width": 400, "pairs": [{"keys": ["j"], "description": "down"}, {"keys": ["k"], "description": "up"}], "font_size": 11.0}
```

#### TextRow

Multiple text segments with host-measured layout.

```json
{"type": "text_row", "x": 0, "y": 0, "items": [{"text": "Name:", "color": "#a6adc8", "size": 14, "monospace": false}, {"text": "Value", "color": "#cdd6f4", "size": 14, "monospace": true}], "gap": 8.0, "align": "left_center"}
```

#### AudioMeter

```json
{"type": "audio_meter", "rect": {"x": 0, "y": 0, "w": 200, "h": 30}, "pipe_id": "mic-pipe"}
```

#### List (legacy)

```json
{"type": "list", "x": 0, "y": 0, "w": 400, "h": 300, "items": [{"label": "Item 1"}], "selected": 0, "item_height": 32}
```

#### ListView

Host-native scrollable list with j/k navigation and typed row slots.

```json
{"type": "list_view", "id": "my-list", "x": 0, "y": 40, "w": 0, "h": 0, "items": [{"type": "row", "id": "r1", "primary": "Title", "secondary": "Subtitle", "leading": {"variant": "badge", "label": "#1", "color": "accent"}, "chips": [{"label": "tag", "color": "accent"}], "trailing": "3m ago"}], "selected": 0, "loading": false, "error": null}
```

`w: 0` = full pane width. `h: 0` = remaining height below y.
Host emits `ListSelect` and `ListActivate` events.

ListViewItem variants:
- `Row`: `{type: "row", id, leading?, primary, secondary?, chips[], trailing?}`
- `CustomCell`: `{type: "custom_cell", id, height_hint?}`

ListViewLeading variants: `badge`, `avatar`, `icon`, `none`.

#### PushClip / PopClip

Clip stack management. All draws between push/pop are clipped to the intersection.

```json
{"type": "push_clip", "x": 0, "y": 0, "w": 200, "h": 100}
{"type": "pop_clip"}
```

#### BeginScroll / EndScroll

Host-managed vertical scroll region.

```json
{"type": "begin_scroll", "id": "log-scroll", "x": 0, "y": 40, "w": 400, "h": 300, "content_height": 1200}
...draw commands...
{"type": "end_scroll"}
```

Host emits `ScrollOffset` events when the user scrolls.

#### Layout

Declarative flex layout tree. Host resolves positions via taffy + egui font metrics.

```json
{"type": "layout", "x": 0, "y": 0, "direction": "column", "children": [{"type": "leaf", "command": {"type": "badge", "x": 0, "y": 0, "label": "Hi", "fill": "#89b4fa", "fg": "#1e1e2e", "font_size": 11, "radius": 6}}, {"type": "node", "direction": "row", "children": [], "gap": 4}], "gap": 8}
```

LayoutDirection: `"row"`, `"column"`, `"stack"`.
LayoutChild: `{"type": "node", ...}` or `{"type": "leaf", "command": {...}}`.

#### Responsive

Host picks a layout tier based on aspect ratio.

```json
{"type": "responsive", "x": 0, "y": 0, "tiers": [{"aspect": "landscape", "direction": "row", "children": [], "gap": 8}, {"aspect": "portrait", "direction": "column", "children": [], "gap": 8}]}
```

Aspect values: `"landscape"` (w > h), `"portrait"` (h > w), `"square"` (fallback).

#### ComponentTree

Emit a declarative UiNode tree rendered by the host. See [UiNode section](#uinode-component-tree).

```json
{"type": "component_tree", "root": {"type": "stack", "direction": "vertical", "children": [{"type": "text", "text": "Hello"}], "gap": 8, "padding": {"top": 0, "right": 0, "bottom": 0, "left": 0}}}
```

---

### AppRequest

App-to-host requests routed through the command dispatcher.

#### CapabilityRequest

```json
{"type": "capability_request", "request_id": "req-1", "capability": "net.http"}
```

Host shows a modal; responds with `CapabilityDecision`.

#### SecretGet

```json
{"type": "secret_get", "key": "GITHUB_TOKEN"}
```

Host responds with `SecretValue`. Scoped to `Init.workspace_root`.

#### SaveAppState

```json
{"type": "save_app_state", "payload": {"last_tab": 2}}
```

Host writes to workspace or global JSON file. Restored via `InjectState` on next launch.

#### Notify

```json
{"type": "notify", "level": "info", "title": "Done", "body": "Task complete", "kind": "message", "priority": 50}
```

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `level` | string | yes | | `"info"`, `"warn"`, or `"error"` |
| `title` | string | yes | | Notification title |
| `body` | string | yes | | Notification body |
| `kind` | string | no | `"message"` | `"message"`, `"choice"`, or `"input"` |
| `options` | NotifyOption[] | no | [] | Choice options (for `kind: "choice"`) |
| `input_prompt` | string? | no | null | Placeholder for `kind: "input"` |
| `required` | bool | no | false | Block Esc dismiss |
| `notify_id` | string? | no | null | Set to receive `NotifyAction` response |
| `priority` | u32 | yes | | Higher = more urgent. Required, no default. |
| `timeout_secs` | u64? | no | null | Auto-dismiss after N seconds |
| `on_dismiss` | string? | no | null | Value sent on timeout/dismiss |
| `image_inline` | object? | no | null | `{mime: string, base64: string}` (max 50KB decoded) |
| `image_pipe_id` | string? | no | null | Binary pipe reference for image |

NotifyOption: `{label: string, value?: string, shortcut?: string, host_action?: string}`

#### StatusSummary

```json
{"type": "status_summary", "text": "3 items loaded"}
```

Updates the status text in the pane chrome.

#### HttpRequest

Requires `net.http` capability. Host responds with `HttpResponse`.

```json
{"type": "http_request", "request_id": "req-1", "url": "https://api.example.com/data", "method": "GET", "headers": {}, "body": null}
```

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `request_id` | string | yes | | Correlation ID |
| `url` | string | yes | | Request URL |
| `method` | string | no | `"GET"` | HTTP method |
| `headers` | object | no | {} | Header map |
| `body` | string? | no | null | Request body |

#### AiQuery

Requires `ai.query` capability. Host responds with `AiResponse` (and optionally `AiStreamChunk`s).

```json
{"type": "ai_query", "request_id": "req-1", "model_tier": "medium", "system": "You are helpful.", "messages": [{"role": "user", "content": "Hello"}], "tools": []}
```

| Field | Type | Required | Description |
|---|---|---|---|
| `request_id` | string | yes | Correlation ID |
| `model_tier` | string | yes | `"low"` (Haiku), `"medium"` (Sonnet), `"high"` (Opus) |
| `system` | string | yes | System prompt |
| `messages` | AiMessage[] | yes | Conversation messages `[{role, content}]` |
| `tools` | AiTool[] | yes | Tool definitions (may be empty) |

AiTool: `{name: string, description: string, input_schema: JSON, timeout_ms?: u64}`

#### ExposeTools

Declare callable tools for the AI broker.

```json
{"type": "expose_tools", "tools": [{"name": "search", "description": "Search items", "input_schema": {"type": "object", "properties": {"query": {"type": "string"}}}}]}
```

#### ToolResult

```json
{"type": "tool_result", "call_id": "call-1", "output_json": "{\"result\": 42}", "error": null}
```

#### McpToolResult

```json
{"type": "mcp_tool_result", "call_id": "call-1", "result": {"items": []}, "error": null}
```

#### PipeOpen

```json
{"type": "pipe_open", "pipe_id": "data-pipe", "mode": "json", "direction": "duplex"}
```

Mode: `"json"` or `"binary"`. Direction: `"in"`, `"out"`, or `"duplex"`.

#### PipeOpenDirected

Open a directed JSON pipe to a specific target pane.

```json
{"type": "pipe_open_directed", "pipe_id": "coord-to-worker", "target_pane_id": 42}
```

#### PipeSend

```json
{"type": "pipe_send", "pipe_id": "data-pipe", "payload": {"action": "update"}}
```

#### SpawnApp

Requires `spawn.app` capability.

```json
{"type": "spawn_app", "type_id": "my-app", "layout": "split_v", "args": ["--flag"]}
```

Layout: `"split_h"`, `"split_v"` (default), `"overlay"`.

#### SpawnPane

Requires `panes.spawn` capability. Supersedes SpawnApp.

```json
{"type": "spawn_pane", "type_id": "terminal", "layout": "split_v", "args": [], "pipe_id": null, "no_focus": false, "name": "Build"}
```

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `type_id` | string | yes | | App type or `"terminal"` |
| `layout` | string? | no | null | `"split_h"`, `"split_v"`, `"overlay"`, `"new_window"`, `"tab"`, etc. |
| `args` | string[] | no | [] | CLI arguments |
| `pipe_id` | string? | no | null | Auto-append `--pipe=<id>` to args |
| `from_pane_id` | u64? | no | null | Source pane for split |
| `request_id` | string? | no | null | Correlation ID |
| `ephemeral` | bool | no | false | Ephemeral pane (auto-close) |
| `cwd` | string? | no | null | Working directory |
| `no_focus` | bool | no | false | Don't focus the new pane |
| `path` | string? | no | null | Launch from filesystem path |
| `workspace_root` | string? | no | null | Override workspace root |
| `target_context` | u64? | no | null | Spawn into a specific context |
| `name` | string? | no | null | Inline pane name |

#### AudioPlay

```json
{"type": "audio_play", "source": "/path/to/file.mp3", "pipe_id": null, "volume": 1.0, "state": "playing"}
```

State: `"playing"`, `"paused"`, `"stopped"`.

#### AudioCapture

Requires `audio.record`. Host responds with `AudioCaptureStarted` or `AudioCaptureError`.

```json
{"type": "audio_capture", "pipe_id": "mic", "device_id": null, "sample_rate": 48000, "buffer_size": 512}
```

#### ListAudioDevices

```json
{"type": "list_audio_devices", "request_id": "req-1"}
```

#### ListMidiDevices

```json
{"type": "list_midi_devices", "request_id": "req-1"}
```

#### OpenMidiInput

Requires `midi.in`.

```json
{"type": "open_midi_input", "port_id": "123", "pipe_id": "midi-in"}
```

#### CloseMidiInput

```json
{"type": "close_midi_input", "port_id": "123"}
```

#### SendMidi

Requires `midi.out`. Fire-and-forget (errors via `MidiSendError`).

```json
{"type": "send_midi", "port_id": "123", "bytes": [144, 60, 100]}
```

#### OpenVideo

Requires `video.playback`. Host responds with `VideoOpenAck` or `VideoOpenError`.

```json
{"type": "open_video", "request_id": "req-1", "source": "file:///path/video.mp4", "pipe_id": "video-stream"}
```

#### SetVideoState

```json
{"type": "set_video_state", "handle_id": 7, "state": {"kind": "play"}}
```

State kinds: `{"kind": "play"}`, `{"kind": "pause"}`, `{"kind": "seek", "position_ms": 1500}`.

#### CloseVideo

```json
{"type": "close_video", "handle_id": 7}
```

#### SetTimer / CancelTimer

Requires `timer`.

```json
{"type": "set_timer", "timer_id": "refresh", "after_ms": 5000}
{"type": "cancel_timer", "timer_id": "refresh"}
```

#### LoadImage

Requires `net.http`. Host responds with `ImageLoaded`.

```json
{"type": "load_image", "handle": "avatar-uuid", "src": "https://example.com/photo.png"}
```

#### RequestLinkedTerminal

Requires `terminal.bindings`.

```json
{"type": "request_linked_terminal", "request_id": "req-1", "cwd": "/tmp/project", "label": "Build Terminal"}
```

#### RunInLinkedTerminal

```json
{"type": "run_in_linked_terminal", "terminal_pane_id": 42, "command": "npm test", "echo": true}
```

#### InsertPathToken

```json
{"type": "insert_path_token", "terminal_pane_id": 42, "path": "/tmp/file.txt", "mode": "replace"}
```

Mode: `"replace"` (Ctrl-W then path) or `"append"` (path verbatim).

#### RequestCommandPreview

```json
{"type": "request_command_preview", "request_id": "req-1", "terminal_pane_id": 42, "command": "rm -rf .git"}
```

#### OpenArtifact

```json
{"type": "open_artifact", "path": "/tmp/project", "mode": "open_in_pane"}
```

Mode: `"open_in_pane"`, `"reveal_in_finder"`, `"open_with_default"`.

#### StreamProcess

Requires `terminal.bindings`.

```json
{"type": "stream_process", "correlation_id": "cid-1", "terminal_pane_id": 42, "command": "ls -la", "channel": "stdout"}
```

Channel: `"stdout"`, `"stderr"`, `"structured"` (v1: same as stdout).

#### CancelProcess

```json
{"type": "cancel_process", "correlation_id": "cid-1"}
```

#### OpenFilePicker

Requires `fs.pick`.

```json
{"type": "open_file_picker", "request_id": "req-1", "filter": ["mp4", "mov"], "multiple": false}
```

#### PushNav / PopNav

Navigation stack management. Host shows back arrow while stack is non-empty.

```json
{"type": "push_nav", "view_id": "detail", "title": "Item Detail"}
{"type": "pop_nav"}
```

#### SetMouseTracking

```json
{"type": "set_mouse_tracking", "enabled": true}
```

#### CdRequest

```json
{"type": "cd_request", "cwd": "/path/to/dir"}
```

#### Context Commands

```json
{"type": "create_context", "root": "/path", "name": "My Context"}
{"type": "focus_context", "root": "/path"}
{"type": "set_context_root", "root": "/path"}
{"type": "set_context_description", "description": "Working on X"}
{"type": "zoom_into_context", "context_id": 42}
{"type": "zoom_out_of_context"}
{"type": "push_pane_to_subcontext", "name": "Sub"}
{"type": "query_context_state", "context_id": 42}
```

#### Pane Management Commands

```json
{"type": "set_pane_title", "pane_id": 42, "name": "Build"}
{"type": "list_panes", "response_file": "/tmp/panes.json"}
{"type": "get_pane_info", "pane_id": 42, "response_file": "/tmp/info.json"}
{"type": "focus_pane", "pane_id": 42}
{"type": "close_pane", "pane_id": 42}
{"type": "send_to_pane", "pane_id": 42, "text": "ls\n"}
{"type": "key_pane", "pane_id": 42, "key": "enter"}
{"type": "capture_pane", "pane_id": 42, "lines": 50, "response_file": "/tmp/capture.json"}
```

---

### ControlCommand

Inline-handled commands processed directly by the host.

#### Ready

SDK handshake. Sent once after receiving Init.

```json
{"type": "ready", "sdk": "plexi-sdk-python/0.1.0", "features_used": []}
```

#### FrameDone

End of frame. Host renders everything queued since the matching Render event.

```json
{"type": "frame_done", "frame_id": 42}
```

#### Log

Forward a log message to the host logger.

```json
{"type": "log", "level": "info", "message": "App initialized"}
```

Level: `"error"`, `"warn"`, `"info"`, `"debug"`.

#### ScheduleRender

Request the host to send a new Render event after a delay.

```json
{"type": "schedule_render", "after_ms": 16}
```

#### CopyToClipboard

```json
{"type": "copy_to_clipboard", "text": "copied text"}
```

#### MeasureText

```json
{"type": "measure_text", "request_id": "req-1", "text": "Hello", "font_size": 14.0, "monospace": false}
```

Host responds with `TextMeasured`.

#### MeasureTextWrapped

```json
{"type": "measure_text_wrapped", "request_id": "req-1", "text": "Long text...", "font_size": 14.0, "max_width": 200.0, "max_lines": 3}
```

Host responds with `TextWrappedMeasured`.

#### SetMinSize

Override manifest-declared minimum pane size at runtime.

```json
{"type": "set_min_size", "width": 300, "height": 200}
```

#### CloseSelf

Gracefully close this app's pane (preferred over `sys.exit()`).

```json
{"type": "close_self"}
```

---

## UiNode (Component Tree)

The component tree is PGAP v3.5's declarative UI system. Apps emit a single
`ComponentTree` draw command with a `UiNode` root; the host renders the
entire tree with consistent theming.

Wire format: `{"type": "<snake_case>", ...fields}`.

### L0 Primitives

#### Stack

Flex container.

```json
{"type": "stack", "direction": "vertical", "children": [...], "gap": 8.0, "padding": {"top": 0, "right": 0, "bottom": 0, "left": 0}}
```

| Field | Type | Default | Description |
|---|---|---|---|
| `direction` | string | `"vertical"` | `"vertical"` or `"horizontal"` |
| `children` | UiNode[] | required | Child nodes |
| `gap` | float | 0.0 | Space between children |
| `padding` | UiPadding | all 0 | Per-side padding `{top, right, bottom, left}` |

#### Scroll

Scrollable single-child container.

```json
{"type": "scroll", "child": {...}, "horizontal": false}
```

#### Layer

Z-stack overlay. Children rendered back-to-front.

```json
{"type": "layer", "children": [...]}
```

#### Text

Inline text node.

```json
{"type": "text", "text": "Hello", "size": 14.0, "color": "#cdd6f4", "bold": false, "monospace": false}
```

| Field | Type | Default | Description |
|---|---|---|---|
| `text` | string | required | Text content |
| `size` | float | 0.0 | Font size (0 = inherit from context) |
| `color` | string | "" | Hex color (empty = inherit) |
| `bold` | bool | false | Bold weight |
| `monospace` | bool | false | Monospace font |

#### Interactive

Interaction wrapper. Host fires `ComponentEvent` for click/hover.

```json
{"type": "interactive", "node_id": "btn-1", "child": {"type": "text", "text": "Click me"}, "on_click": true, "on_hover": false}
```

| Field | Type | Default | Description |
|---|---|---|---|
| `node_id` | string | required | Event target ID |
| `child` | UiNode | required | Wrapped child |
| `on_click` | bool | false | Fire click events |
| `on_hover` | bool | false | Fire hover events |

#### Raw

Escape hatch: embed a flat RenderCommand inside the tree.

```json
{"type": "raw", "command": {"type": "rect", "x": 0, "y": 0, "w": 100, "h": 50, "fill": "#ff0000", "radius": 4}}
```

#### Surface

Future GPU surface placeholder. Currently a no-op.

```json
{"type": "surface", "id": "canvas-1"}
```

### L1 Sugar

Host-rendered widgets with consistent styling.

#### Button

```json
{"type": "button", "node_id": "submit", "label": "Submit", "disabled": false}
```

Fires `ComponentEvent` with `event_type: "click"` when pressed.

#### Input

```json
{"type": "input", "node_id": "name-field", "placeholder": "Enter name", "value": ""}
```

Fires `ComponentEvent` with `event_type: "change"` (payload: `{"value": "..."}`)
and `event_type: "submit"` (payload: `{"value": "..."}`) on Enter.

#### Badge

Pill badge.

```json
{"type": "badge", "label": "NEW", "fill": "#89b4fa", "fg": "#1e1e2e"}
```

| Field | Type | Default | Description |
|---|---|---|---|
| `label` | string | required | Badge text |
| `fill` | string | "" | Background color (empty = theme accent) |
| `fg` | string | "" | Text color (empty = theme text) |

#### Dot

Colored dot indicator.

```json
{"type": "dot", "color": "#a6e3a1", "size": 8.0}
```

### L1 Layout Components

#### AppBar

Title bar with optional subtitle.

```json
{"type": "app_bar", "title": "My App", "subtitle": "v1.0"}
```

#### FooterKeys

Keyboard shortcut hints row.

```json
{"type": "footer_keys", "entries": [{"keys": ["j"], "description": "down"}, {"keys": ["k"], "description": "up"}], "divider": true}
```

#### Footer

Single-line status footer.

```json
{"type": "footer", "text": "3 items", "color": ""}
```

#### Section

Section header with uppercase label and rule below.

```json
{"type": "section", "title": "Settings"}
```

#### Label

Themed text label with semantic tone.

```json
{"type": "label", "text": "Hello", "size": 14.0, "color": "", "tone": "body", "bold": false, "monospace": false, "max_lines": 0}
```

| Field | Type | Default | Description |
|---|---|---|---|
| `text` | string | required | Label text |
| `size` | float | 0.0 | Font size (0 = TEXT_BODY) |
| `color` | string | "" | Override color (empty = use tone) |
| `tone` | string | "" | Semantic tone: `""`, `"hint"`, `"dim"`, `"muted"`, `"danger"`, `"error"`, `"success"`, `"warning"`, `"accent"`, `"section"` |
| `bold` | bool | false | Bold weight |
| `monospace` | bool | false | Monospace font |
| `max_lines` | usize | 0 | 0 = wrap freely; >0 = truncate |

#### Spacer

Flexible space.

```json
{"type": "spacer", "size": 12.0, "grow": false}
```

`grow: true` expands to fill remaining space.

#### Divider

Horizontal 1px rule.

```json
{"type": "divider", "color": ""}
```

#### Card

Bordered container.

```json
{"type": "card", "children": [...], "padding": 12.0}
```

#### SelectList

Keyboard-navigable scrollable list.

```json
{"type": "select_list", "items": [{"name": "Item 1", "description": "Details", "leading": "", "trailing": ""}], "selected_idx": 0}
```

SelectListItem: `{name: string, description?: string, leading?: string, trailing?: string}`

---

## Capabilities

Declared in `manifest.toml` under `[capabilities]`. Required for specific
AppRequest commands to be accepted by the host.

| Capability | Wire String | Description |
|---|---|---|
| FsRead | `fs.read` | Read files within workspace_root |
| FsWrite | `fs.write` | Write files within workspace_root |
| NetHttp | `net.http` | Outbound HTTP(S) requests, LoadImage |
| SecretsGet | `secrets.get` | SecretGet (scoped to workspace_root) |
| PipeOpen | `pipe.open` | Open typed pipes (JSON or binary) |
| SpawnApp | `spawn.app` | Launch another app via SpawnApp |
| PanesSpawn | `panes.spawn` | Spawn panes via SpawnPane |
| AudioRecord | `audio.record` | Microphone capture via AudioCapture |
| AudioPlayback | `audio.playback` | Audio playback via AudioPlay |
| VideoPlayback | `video.playback` | Video decode via OpenVideo |
| Timer | `timer` | SetTimer / CancelTimer |
| AiQuery | `ai.query` | LLM calls through the Plexi AI broker |
| MidiIn | `midi.in` | Receive MIDI via OpenMidiInput |
| MidiOut | `midi.out` | Send MIDI via SendMidi |
| TerminalBindings | `terminal.bindings` | Drive a linked terminal (RequestLinkedTerminal, RunInLinkedTerminal, InsertPathToken, RequestCommandPreview, OpenArtifact, StreamProcess, CancelProcess) |
| FsPick | `fs.pick` | Native file picker dialog |

No capability required: ListAudioDevices, ListMidiDevices, CopyToClipboard,
MeasureText, MeasureTextWrapped, StatusSummary, SaveAppState, Log,
ScheduleRender, SetMinSize, CloseSelf, PushNav, PopNav, SetMouseTracking.

---

## Supporting Types

### Rect

```json
{"x": 0.0, "y": 0.0, "w": 800.0, "h": 600.0}
```

### Modifiers

```json
{"shift": false, "ctrl": false, "alt": false, "cmd": false}
```

### MouseButton

`"primary"` or `"secondary"`.

### StackDirection

`"vertical"` or `"horizontal"`.

### UiPadding

```json
{"top": 0.0, "right": 0.0, "bottom": 0.0, "left": 0.0}
```

### FooterKeyEntry

```json
{"keys": ["Cmd", "K"], "description": "Command palette"}
```

### SelectListItem

```json
{"name": "Item", "description": "", "leading": "", "trailing": ""}
```

### ShortcutPair

```json
{"keys": ["j"], "description": "down"}
```

### TextRowItem

```json
{"text": "label", "color": "#cdd6f4", "size": 14.0, "monospace": false}
```

### AiMessage

```json
{"role": "user", "content": "Hello"}
```

### ModelTier

`"low"` (Haiku), `"medium"` (Sonnet), `"high"` (Opus).

### PathTokenMode

`"replace"` or `"append"`.

### ArtifactOpenMode

`"open_in_pane"`, `"reveal_in_finder"`, `"open_with_default"`.

### StreamChannel

`"stdout"`, `"stderr"`, `"structured"`.

### NotifyKind

`"message"` (default), `"choice"`, `"input"`.

### NotifyScope

`"window"`, `"context"`, `"global"`. Set by manifest, not by app code.

---

## Python SDK Components

The Python SDK (`plexi_sdk.ui`) provides high-level components that emit
either `ComponentTree` (L1) or raw draw commands (L0 fallback).

### Container Components

| Component | Description | to_node() |
|---|---|---|
| `Column(children, padding, gap)` | Root vertical container with grow-spacer support | `stack` (vertical) |
| `Card(children, padding, gap)` | Surface-colored bordered container | `card` |
| `Scrollable(child)` | Clip-bounded scrollable container | `scroll` |

### Layout Components

| Component | Description | to_node() |
|---|---|---|
| `AppBar(title, subtitle)` | Top-of-pane title bar | `app_bar` |
| `Section(title)` | Uppercase section header with rule | `section` |
| `Footer(text, color)` | Bottom status line | `footer` |
| `FooterKeys(shortcuts, divider)` | Keyboard shortcut hints row | `footer_keys` |
| `Spacer(size, grow)` | Fixed or flex gap | `spacer` |
| `Divider(color)` | 1px horizontal rule | `divider` |

### Text Components

| Component | Description | to_node() |
|---|---|---|
| `Heading(text, level)` | Title text (level 1/2/3) | `label` |
| `Label(text, tone, color)` | Body/caption/hint text with wrapping | `label` |

### Interactive Components

| Component | Description | to_node() |
|---|---|---|
| `TextInput(id, placeholder)` | Text input field | L0 only |
| `ButtonRow(id, label)` | Clickable button | `button` |
| `FormField(id, label, placeholder)` | Label + TextInput combo | L0 only |
| `SelectList(items, selected_idx)` | Keyboard-navigable list | `select_list` |

### Data Display Components

| Component | Description | to_node() |
|---|---|---|
| `KeyRow(key, description)` | Keycap chip + description | L0 only |
| `ListItem(title, subtitle, ...)` | Single/double-line list row | L0 only |
| `Row(label, leading, trailing)` | Horizontal info row | L0 only |
| `ScrollLog(lines)` | Bounded text log | L0 only |
| `ChatBubble(text, role)` | Chat message bubble | L0 only |
| `InfoTable(rows)` | Key-value table | L0 only |

### UiNode Tree Components (v3.5)

These produce `dict` values matching the `UiNode` wire format directly.

| Component | Description |
|---|---|
| `Tabs(tabs, active)` | Tabbed container (decomposes to Stack + Interactive) |
| `Grid(columns, children)` | Fixed-column grid layout |
| `Toggle(node_id, value, label)` | On/off toggle switch |
| `Clickable(node_id, child)` | Makes any component clickable |
| `ProgressBar(value, max_value)` | Horizontal progress bar |

### Style Tokens

```python
# Spacing (px)
SPACE_XS = 4.0
SPACE_SM = 8.0
SPACE_MD = 12.0
SPACE_LG = 16.0
SPACE_XL = 24.0

# Typography (pt)
TEXT_HINT = 11.0
TEXT_CAPTION = 12.0
TEXT_BODY = 14.0
TEXT_HEADING = 16.0
TEXT_TITLE = 20.0
TEXT_TITLE_XL = 28.0

# Radii
RADIUS_SM = 4.0
RADIUS_MD = 8.0
RADIUS_LG = 12.0
RADIUS_BADGE = 6.0
```

---

## Common Patterns

### Minimal App

```json
// Host sends:
{"type": "init", "protocol": "pgap/3", "app_id": "hello", "workspace_root": "/tmp", "capabilities": [], "feature_flags": [], "theme": {}}

// App replies:
{"type": "ready", "sdk": "my-sdk/1.0", "features_used": []}

// Host sends each frame:
{"type": "render", "frame_id": 1, "rect": {"x": 0, "y": 0, "w": 800, "h": 600}}

// App draws and signals done:
{"type": "rect", "x": 0, "y": 0, "w": 800, "h": 600, "fill": "#1e1e2e", "radius": 0}
{"type": "text", "x": 20, "y": 20, "text": "Hello!", "size": 14, "color": "#cdd6f4", "monospace": false, "bold": false, "align": "top_left", "max_width": null, "elide": false, "selectable": false}
{"type": "frame_done", "frame_id": 1}
```

### Component Tree App

```json
// Instead of individual draw commands, emit one ComponentTree:
{"type": "component_tree", "root": {
  "type": "stack",
  "direction": "vertical",
  "children": [
    {"type": "app_bar", "title": "My App", "subtitle": ""},
    {"type": "section", "title": "Settings"},
    {"type": "card", "children": [
      {"type": "label", "text": "Option 1", "size": 14, "color": "", "tone": "body", "bold": false, "monospace": false, "max_lines": 0},
      {"type": "divider", "color": ""},
      {"type": "label", "text": "Option 2", "size": 14, "color": "", "tone": "body", "bold": false, "monospace": false, "max_lines": 0}
    ], "padding": 12},
    {"type": "spacer", "size": 0, "grow": true},
    {"type": "footer_keys", "entries": [
      {"keys": ["j"], "description": "down"},
      {"keys": ["k"], "description": "up"},
      {"keys": ["Enter"], "description": "select"}
    ], "divider": true}
  ],
  "gap": 8,
  "padding": {"top": 8, "right": 24, "bottom": 24, "left": 24}
}}
{"type": "frame_done", "frame_id": 1}
```

### List with Actions

```json
{"type": "component_tree", "root": {
  "type": "stack",
  "direction": "vertical",
  "children": [
    {"type": "app_bar", "title": "Tasks", "subtitle": "3 items"},
    {"type": "select_list", "items": [
      {"name": "Build project", "description": "cargo build --release", "leading": "1", "trailing": ""},
      {"name": "Run tests", "description": "cargo test", "leading": "2", "trailing": ""},
      {"name": "Deploy", "description": "just promote beta", "leading": "3", "trailing": ""}
    ], "selected_idx": 0}
  ],
  "gap": 0,
  "padding": {"top": 0, "right": 0, "bottom": 0, "left": 0}
}}
```

### Form with Input

```json
{"type": "component_tree", "root": {
  "type": "stack",
  "direction": "vertical",
  "children": [
    {"type": "app_bar", "title": "New Item", "subtitle": ""},
    {"type": "card", "children": [
      {"type": "label", "text": "Name", "size": 11, "color": "", "tone": "hint", "bold": false, "monospace": false, "max_lines": 0},
      {"type": "input", "node_id": "name", "placeholder": "Enter name...", "value": ""},
      {"type": "spacer", "size": 8, "grow": false},
      {"type": "button", "node_id": "submit", "label": "Create", "disabled": false}
    ], "padding": 16}
  ],
  "gap": 8,
  "padding": {"top": 8, "right": 24, "bottom": 24, "left": 24}
}}
```

### Dashboard with Badges

```json
{"type": "component_tree", "root": {
  "type": "stack",
  "direction": "vertical",
  "children": [
    {"type": "app_bar", "title": "Dashboard", "subtitle": ""},
    {"type": "stack", "direction": "horizontal", "children": [
      {"type": "badge", "label": "OK", "fill": "#a6e3a1", "fg": "#1e1e2e"},
      {"type": "badge", "label": "3 warnings", "fill": "#f9e2af", "fg": "#1e1e2e"},
      {"type": "dot", "color": "#a6e3a1", "size": 8}
    ], "gap": 8, "padding": {"top": 0, "right": 0, "bottom": 0, "left": 0}},
    {"type": "divider", "color": ""},
    {"type": "label", "text": "All systems operational", "size": 14, "color": "", "tone": "hint", "bold": false, "monospace": false, "max_lines": 0}
  ],
  "gap": 8,
  "padding": {"top": 8, "right": 24, "bottom": 24, "left": 24}
}}
```
