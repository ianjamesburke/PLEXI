# PGAP — Plexi General App Protocol

**Version:** pgap/3  
**Ground truth:** `src/app_protocol.rs`, `src/app_permissions.rs`

---

## What is PGAP?

PGAP is the wire protocol between a Plexi app process and the host. Every app—built-in or third-party—speaks the same protocol.

**Transport:** newline-delimited JSON on **stdin** (host→app) and **stdout** (app→host). One JSON object per line; no framing, no length prefix.

**Binary data** (audio PCM, video frames, raw bytes) travels on **typed pipes**—Unix sockets opened by the host on demand. The PGAP wire carries only JSON control and draw messages.

---

## Handshake

1. Host spawns the app binary.
2. Host sends exactly one `init` event.
3. App replies with `{"type":"ready","sdk":"...","features_used":[...]}` once after receiving `init`.
4. Each frame: host sends `render`; app replies with draw commands terminated by `frame_done`.
5. Input events (`key`, `click`, `command`, mouse events) arrive between frames as they occur.
6. Out-of-frame commands (`capability_request`, `secret_get`, `notify`, etc.) may arrive at any time; host processes them immediately.
7. On close: host sends `shutdown`; app must exit cleanly within a short timeout.

```python
# Minimal app (pseudocode)
init = read_json_line(stdin)        # PlexiEvent: type == "init"
write_json({"type": "ready", "sdk": "my-sdk/1.0", "features_used": []})
while True:
    event = read_json_line(stdin)
    if event["type"] == "render":
        write_json({"type": "rect", "x": 0, "y": 0, "w": 800, "h": 600, "fill": "#1e1e2e"})
        write_json({"type": "text", "x": 20, "y": 20, "text": "Hello!", "size": 14.0,
                    "color": "#cdd6f4", "selectable": False, "elide": False,
                    "max_width": None, "align": "top_left"})
        write_json({"type": "frame_done", "frame_id": event["frame_id"]})
    elif event["type"] == "shutdown":
        sys.exit(0)
```

---

## Stability Markers

| Marker | Meaning |
|--------|---------|
| **stable** | Production-ready. Wire shape is frozen for v3. |
| **pre-v1** | Wire shape is defined; production implementation is pending. Using this today will return an error response. |
| **deprecated** | Still accepted on the wire. Will be removed in a future major version. Use the replacement instead. |

---

## Events — Host → App (`PlexiEvent`)

These are JSON objects the host writes to the app's stdin.

| `type` | Description | Stability |
|--------|-------------|-----------|
| `init` | Sent exactly once on startup. Fields: `protocol` (e.g. `"pgap/3"`), `app_id`, `workspace_root`, `capabilities` (granted list), `feature_flags`. App must reply with `ready`. | **stable** |
| `render` | Request a new frame. Fields: `frame_id` (u64), `rect` (`{x,y,w,h}`). App replies with draw commands + `frame_done`. | **stable** |
| `resize` | Surface was resized. Fields: `width`, `height`. | **stable** |
| `key` | Key event. Fields: `key` (string), `modifiers` (`{shift,ctrl,alt,cmd}`). | **stable** |
| `click` | Mouse click in app surface. Fields: `x`, `y`, `button` (`"primary"` \| `"secondary"`). | **stable** |
| `mouse_down` | Pointer pressed. Fields: `x`, `y`, `button`. | **stable** |
| `mouse_up` | Pointer released. Fields: `x`, `y`, `button`. | **stable** |
| `mouse_move` | Pointer moved (only when `set_mouse_tracking` is on). Fields: `x`, `y`, `buttons` (held). | **stable** |
| `command` | User submitted text via the command bar. Fields: `text`. | **stable** |
| `capability_decision` | Response to `capability_request`. Fields: `request_id`, `granted` (bool). | **stable** |
| `secret_value` | Response to `secret_get`. Fields: `key`, `value` (string or null when denied). | **stable** |
| `run_update` | Run lifecycle update. Fields: `run_id`, `status` (`"pending"` \| `"running"` \| `"blocked_on_user"` \| `"completed"` \| `"failed"`), `payload`. | **stable** |
| `pipe_message` | JSON pipe message. Fields: `pipe_id`, `payload`. | **stable** |
| `path_changed` | Pane group CWD broadcast. Fields: `cwd`. | **stable** |
| `suspend` | App is being backgrounded. | **stable** |
| `resume` | App is being foregrounded. | **stable** |
| `shutdown` | App is being closed. Must exit within a short timeout. | **stable** |
| `app_spawned` | Confirmation of a `spawn_app` request. Fields: `pane_id`, `type_id`. | **deprecated** |
| `pane_spawned` | Confirmation of a `spawn_pane` request. Fields: `pane_id`, `request_id?`. | **stable** |
| `pane_spawn_error` | `spawn_pane` failed. Fields: `reason`, `request_id?`. | **stable** |
| `pipe_opened` | Binary pipe opened. Fields: `pipe_id`, `socket_path`. | **stable** |
| `pipe_overrun` | Binary pipe backpressure. Fields: `pipe_id`, `dropped_frames`. | **stable** |
| `inject_state` | State injected at startup (persisted app state) or via test harness. Fields: `payload`. | **stable** |
| `http_response` | Response to `http_request`. Fields: `request_id`, `status`, `body`, `error?`. | **stable** |
| `notify_action` | User responded to a `notify` with a `notify_id`. Fields: `notify_id`, `action_label`, `value?`. | **stable** |
| `timer` | One-shot timer fired. Fields: `timer_id`. | **stable** |
| `text_measured` | Response to `measure_text`. Fields: `request_id`, `width`, `height`. | **stable** |
| `text_submitted` | User pressed Enter in a `text_input` field. Fields: `id`, `value`. | **stable** |
| `paste` | Clipboard paste forwarded. Fields: `text`. | **stable** |
| `ai_response` | Response to `ai_query`. Fields: `request_id`, `content?`, `tokens_in`, `tokens_out`, `error?`. | **stable** |
| `tool_call` | Host calls a tool exposed via `expose_tools`. Fields: `call_id`, `name`, `input_json`. | **stable** |
| `mcp_tool_call` | External MCP client called a declared tool. Fields: `call_id`, `tool_name`, `arguments`. | **stable** |
| `audio_devices_listed` | Response to `list_audio_devices`. Fields: `request_id`, `inputs`, `outputs`, `error?`. | **stable** |
| `audio_capture_started` | Capture opened successfully. Fields: `pipe_id`, `sample_rate`, `channels`, `buffer_size`. | **stable** |
| `audio_capture_error` | Capture failed. Fields: `pipe_id`, `error`. | **stable** |
| `midi_devices_listed` | Response to `list_midi_devices`. Fields: `request_id`, `inputs`, `outputs`, `error?`. | **stable** |
| `midi_input_opened` | MIDI input port opened. Fields: `port_id`, `pipe_id`. | **stable** |
| `midi_input_error` | MIDI input failed. Fields: `port_id`, `error`. | **stable** |
| `midi_send_error` | `send_midi` failed. Fields: `port_id`, `error`. | **stable** |
| `video_open_ack` | Video decoder opened. Fields: `request_id`, `handle_id`, `width`, `height`, `fps`, `duration_ms`. | **pre-v1** |
| `video_open_error` | Video decoder failed. Fields: `request_id`, `error`. | **pre-v1** |
| `linked_terminal_ready` | Linked terminal opened. Fields: `request_id`, `terminal_pane_id`. | **stable** |
| `command_preview` | Response to `request_command_preview`. Fields: `request_id`, `command`, `would_run_in_cwd`. | **stable** |
| `nav_back` | Escape pressed while nav stack depth > 0. Fields: `view_id`. | **stable** |
| `file_picked` | User selected file(s). Fields: `request_id`, `paths` (array). | **stable** |
| `file_pick_cancelled` | File picker dismissed or capability denied. Fields: `request_id`. | **stable** |
| `stream_chunk` | Stdout/stderr bytes from `stream_process`. Fields: `correlation_id`, `channel`, `bytes` (array of ints 0–255). Delivered at up to ~30 Hz. | **stable** |
| `stream_end` | Child process exited or was cancelled. Fields: `correlation_id`, `exit_code`. | **stable** |
| `scroll_offset` | Scroll offset changed for a `begin_scroll` region. Fields: `id`, `offset_y`. | **stable** |

---

## Draw Commands — App → Host (`DrawCommand`)

Draw commands are JSON objects the app writes to stdout, one per line. The `type` field identifies the command.

DrawCommands are split into three logical groups:

### Render Commands (visual)

Go to the pending frame buffer; rendered to screen on `frame_done`.

| `type` | Fields | Description | Stability |
|--------|--------|-------------|-----------|
| `rect` | `x`, `y`, `w`, `h`, `fill` (hex), `radius?` (default 0) | Fill a rectangle. Alpha supported via `#rrggbbaa`. | **stable** |
| `text` | `x`, `y`, `text`, `size`, `color` (hex), `selectable` (required), `elide` (required), `max_width?`, `align?` (`"top_left"` default \| `"center"`), `monospace?`, `bold?` | Draw text. `selectable: true` makes the text drag-selectable. `elide: true` appends `…` when clipped by `max_width`. | **stable** |
| `line` | `x1`, `y1`, `x2`, `y2`, `color` (hex), `width?` (default 1.0) | Draw a line segment. | **stable** |
| `circle` | `cx`, `cy`, `r`, `fill` (hex) | Draw a filled circle. Alpha supported. | **stable** |
| `arc` | `cx`, `cy`, `r`, `start_angle`, `end_angle`, `fill` (hex) | Draw a filled arc/pie slice. Angles in radians, clockwise from east. Full circle: 0 to `TAU` (≈6.283). | **stable** |
| `list` | `x?`, `y?`, `w?`, `h?`, `items` (array), `selected` (int), `item_height?` | High-level scrollable list managed by host. Each item: `{label, secondary?, icon?, is_dir?}`. | **stable** |
| `push_clip` | `x`, `y`, `w`, `h` | Push a clip rect. All subsequent draw commands are clipped to this rect (intersected with current stack). Must be balanced with `pop_clip`. | **stable** |
| `pop_clip` | (none) | Pop the most recently pushed clip rect. | **stable** |
| `badge` | `x`, `y`, `label`, `fill` (hex), `fg` (hex), `font_size`, `radius` | Host-rendered pill badge. Host measures label width with real font metrics. `y` is the vertical centre. | **stable** |
| `key_chip` | `x`, `y`, `label`, `font_size` | Host-rendered keyboard keycap chip. | **stable** |
| `key_chip_row` | `x`, `y`, `keys` (array of strings), `description?`, `font_size` | Row of keycap chips with optional trailing description. | **stable** |
| `shortcuts` | `x`, `y`, `max_width`, `pairs` (array of `{keys, description}`), `font_size` | Multi-group shortcut row. Host wraps to new line when a group would exceed `max_width`. | **stable** |
| `text_row` | `x`, `y`, `items` (array of `{text, color, size, monospace}`), `gap`, `align` | Multiple text segments in a horizontal row, measured by host. | **stable** |
| `markdown` | `x`, `y`, `w`, `text`, `base_size`, `color` | Render markdown text using `egui_commonmark`. | **stable** |
| `image` | `src`, `x`, `y`, `w`, `h`, `fit?` (`"contain"` default \| `"cover"` \| `"fill"`) | Draw an image from a workspace-scoped path or data URL. | **stable** |
| `audio_meter` | `rect` (`{x,y,w,h}`), `pipe_id` | Render an amplitude meter reading from a binary pipe. | **stable** |
| `text_input` | `id`, `x`, `y`, `w`, `h?` (default 24), `placeholder`, `multiline?` (default false) | Host-buffered text input. Host emits `text_submitted` on Enter. Per-keystroke access not available. | **stable** |
| `begin_scroll` | `id`, `x`, `y`, `w`, `h`, `content_height` | Begin a host-managed vertical scroll region. Host emits `scroll_offset` on scroll. `id` must be stable across frames. | **stable** |
| `end_scroll` | (none) | Close the most recently opened scroll region. Must balance `begin_scroll`. | **stable** |

### Host Commands (side-effectful)

Processed immediately by host routing; not queued to the frame buffer.

| `type` | Fields | Capability | Description | Stability |
|--------|--------|------------|-------------|-----------|
| `capability_request` | `request_id`, `capability` (string) | — | Ask host to prompt user to grant a runtime capability. Host replies with `capability_decision`. | **stable** |
| `secret_get` | `key` | `secrets.get` | Request a workspace-scoped secret from the host keychain. Host replies with `secret_value`. | **stable** |
| `save_app_state` | `payload` | — | Persist arbitrary JSON state. Host writes to workspace or global state file. | **stable** |
| `run_get` | `intent`, `payload` | — | Request a run. Surfaces in the Run palette (Cmd+R). | **stable** |
| `run_complete` | `run_id`, `result` | — | Signal a run the app owns has finished. | **stable** |
| `notify` | `level`, `title`, `body`, `priority` (required), `kind?`, `options?`, `actions?`, `input_prompt?`, `required?`, `notify_id?`, `image_inline?`, `image_pipe_id?`, `timeout_secs?`, `on_dismiss?` | — | Post a notification. `kind`: `"message"` (default), `"choice"`, or `"input"`. `priority` typical values: 0 (background), 50 (normal), 100 (important), 200 (critical). | **stable** |
| `pipe_open` | `pipe_id`, `mode` (`"json"` \| `"binary"`), `direction` (`"in"` \| `"out"` \| `"duplex"`) | `pipe.open` | Open a typed pipe. | **stable** |
| `pipe_open_directed` | `pipe_id`, `target_pane_id` | `pipe.open` | Open a directed JSON pipe to a specific pane. Always duplex/JSON. | **stable** |
| `pipe_send` | `pipe_id`, `payload` | — | Send a JSON payload on a pipe. | **stable** |
| `status_summary` | `text` | — | Update the status text shown in the pane chrome. | **stable** |
| `spawn_app` | `type_id`, `layout?`, `args?` | `spawn.app` | **Deprecated.** Use `spawn_pane` instead. Host replies with `app_spawned`. | **deprecated** |
| `spawn_pane` | `type_id`, `layout?` (`"split_v"` \| `"split_h"` \| `"split_above"` \| `"split_left"` \| `"overlay"`), `args?`, `pipe_id?`, `from_pane_id?`, `request_id?`, `response_file?`, `ephemeral?` | `panes.spawn` | Unified pane spawn. Host replies with `pane_spawned` or `pane_spawn_error`. | **stable** |
| `set_pane_title` | `pane_id`, `name` | — | Set the title on a terminal pane's tab. | **stable** |
| `list_panes` | `response_file` | — | List all open panes. Host writes JSON array to `response_file`. | **stable** |
| `get_pane_info` | `pane_id`, `response_file` | — | Get info for a specific pane. | **stable** |
| `focus_pane` | `pane_id` | — | Move UI focus to a pane. Fire-and-forget. | **stable** |
| `close_pane` | `pane_id` | — | Close a pane. Fire-and-forget. | **stable** |
| `send_to_pane` | `pane_id`, `text`, `response_file?` | — | Write text to a pane's PTY stdin. `\n` in text is interpreted as Enter. | **stable** |
| `create_context` | `root?`, `name?` | — | Create a new context. | **stable** |
| `focus_context` | `root` | — | Focus an existing context by root, or create one. | **stable** |
| `set_context_root` | `root` | — | Set/update the root of the active context. | **stable** |
| `http_request` | `request_id`, `url`, `method?` (default `"GET"`), `headers?`, `body?` | `net.http` | Brokered HTTP request. Host replies with `http_response`. | **stable** |
| `ai_query` | `request_id`, `model_tier` (`"low"` \| `"medium"` \| `"high"`), `system`, `messages`, `tools` | `ai.query` | Tier-routed LLM call through the Plexi AI broker. All fields required. Host replies with `ai_response`. Non-empty `tools` triggers a tool-use loop. | **stable** |
| `expose_tools` | `tools` (array of `AiTool`) | — | Declare callable tools to the host tool registry. May be sent at any time. Replaces prior registration for this pane. | **stable** |
| `tool_result` | `call_id`, `output_json?`, `error?` | — | Return the result of a `tool_call` event. One of `output_json` or `error` must be set. | **stable** |
| `mcp_tool_result` | `call_id`, `result?`, `error?` | — | Return the result of an `mcp_tool_call` event. | **stable** |
| `audio_play` | `source?`, `pipe_id?`, `volume?` (default 1.0), `state` (`"playing"` \| `"paused"` \| `"stopped"`) | `audio.playback` | Host-owned audio playback via rodio. | **stable** |
| `audio_capture` | `pipe_id`, `device_id?`, `sample_rate`, `buffer_size` | `audio.record` | Start mic capture. PCM delivered on binary pipe. Host replies with `audio_capture_started` or `audio_capture_error`. | **stable** |
| `list_audio_devices` | `request_id` | — | Enumerate audio devices. No capability gate. Host replies with `audio_devices_listed`. | **stable** |
| `list_midi_devices` | `request_id` | — | Enumerate MIDI ports. No capability gate. Host replies with `midi_devices_listed`. | **stable** |
| `open_midi_input` | `port_id`, `pipe_id` | `midi.in` | Open a MIDI input port. MIDI byte streams delivered on binary pipe. | **stable** |
| `close_midi_input` | `port_id` | — | Close a previously opened MIDI input port. No-op if not open. | **stable** |
| `send_midi` | `port_id`, `bytes` (array of ints) | `midi.out` | Send one MIDI 1.0 byte stream. Fire-and-forget (errors surface as `midi_send_error`). | **stable** |
| `open_video` | `request_id`, `source`, `pipe_id` | `video.playback` | Open a video decoder. RGBA8 frames delivered on binary pipe. All fields required. | **pre-v1** |
| `set_video_state` | `handle_id`, `state` (`{"kind":"play"}` \| `{"kind":"pause"}` \| `{"kind":"seek","position_ms":N}`) | `video.playback` | Drive playback for an opened video handle. | **pre-v1** |
| `close_video` | `handle_id` | `video.playback` | Tear down a video decoder. Fire-and-forget. | **pre-v1** |
| `cd_request` | `cwd` | — | cd all terminals in the same pane group to `cwd`. | **stable** |
| `set_timer` | `timer_id`, `after_ms` | `timer` | One-shot timer. Host fires `timer` event after `after_ms` ms. | **stable** |
| `cancel_timer` | `timer_id` | — | Cancel a pending timer. No-op if already fired or nonexistent. | **stable** |
| `request_linked_terminal` | `request_id`, `cwd?`, `label?` | `terminal.bindings` | Ask host to open a linked terminal pane. Host replies with `linked_terminal_ready`. | **stable** |
| `run_in_linked_terminal` | `terminal_pane_id`, `command`, `echo` (bool) | `terminal.bindings` | Execute a command in a linked terminal. `echo: true` types the command visibly. | **stable** |
| `insert_path_token` | `terminal_pane_id`, `path`, `mode` (`"replace"` \| `"append"`) | `terminal.bindings` | Insert a path into the linked terminal at the cursor. `"replace"` sends Ctrl-W first. | **stable** |
| `request_command_preview` | `request_id`, `terminal_pane_id`, `command` | `terminal.bindings` | Preview what `command` would run (and in which cwd) without executing it. | **stable** |
| `open_artifact` | `path`, `mode` (`"open_in_pane"` \| `"reveal_in_finder"` \| `"open_with_default"`) | `terminal.bindings` | Open a workspace artifact via the host. | **stable** |
| `push_nav` | `view_id`, `title` | — | Signal the app pushed a navigation level. Host shows a back arrow in pane chrome. | **stable** |
| `pop_nav` | (none) | — | Signal the app popped a navigation level. | **stable** |
| `set_mouse_tracking` | `enabled` (bool) | — | Enable/disable `mouse_move` event delivery. Off by default. | **stable** |
| `stream_process` | `correlation_id`, `terminal_pane_id`, `command`, `channel` (`"stdout"` \| `"stderr"` \| `"structured"`) | `terminal.bindings` | Spawn `command` via `sh -c` and stream output. Host delivers `stream_chunk` events at ~30 Hz and `stream_end` on exit. | **stable** |
| `cancel_process` | `correlation_id` | — | Cancel an in-flight `stream_process`. SIGTERM, then SIGKILL after 1s. `stream_end` always delivered. | **stable** |
| `open_file_picker` | `request_id`, `filter` (array of extensions, no dots), `multiple` (bool) | `fs.pick` | Show a native file picker dialog. Host replies with `file_picked` or `file_pick_cancelled`. | **stable** |

### Control Commands (protocol)

Inline-handled; not queued to the frame buffer.

| `type` | Fields | Description | Stability |
|--------|--------|-------------|-----------|
| `ready` | `sdk?`, `features_used?` | SDK ready handshake. Sent once after `init`. | **stable** |
| `frame_done` | `frame_id` | End of frame. `frame_id` must match the triggering `render` event. | **stable** |
| `log` | `level` (`"error"` \| `"warn"` \| `"info"` \| `"debug"`), `message` | Forward a log message into Plexi's logger, tagged with `app_id`. | **stable** |
| `schedule_render` | `after_ms` | Request a new `render` event after `after_ms` ms. For game loops and animations. | **stable** |
| `copy_to_clipboard` | `text` | Write text to the OS clipboard. No capability required. No response. | **stable** |
| `measure_text` | `request_id`, `text`, `font_size`, `monospace?` | Request a one-shot text measurement. Host replies with `text_measured`. | **stable** |

---

## Capabilities

Declared in `manifest.toml` under `[app.capabilities]`. The host enforces them at dispatch time. An app may also request capabilities at runtime via `capability_request`.

| String | What it grants | Stability |
|--------|----------------|-----------|
| `fs.read` | Read files within `workspace_root`. | **stable** |
| `fs.write` | Write files within `workspace_root`. | **stable** |
| `net.http` | Outbound HTTP(S) requests via the host broker (`http_request`). | **stable** |
| `secrets.get` | Call `secret_get`; scoped to `workspace_root`. | **stable** |
| `pipe.open` | Open typed pipes (`pipe_open`, `pipe_open_directed`). | **stable** |
| `spawn.app` | **Deprecated.** Use `panes.spawn` instead. Accepted for back-compat. | **deprecated** |
| `audio.record` | Capture microphone audio via host broker (`audio_capture`). | **stable** |
| `audio.playback` | Play audio via host rodio broker (`audio_play`). | **stable** |
| `video.playback` | Decode and display video via host broker (`open_video`). | **pre-v1** |
| `llm` | LLM API calls via host broker (reads `OPENROUTER_API_KEY`). | **stable** |
| `timer` | One-shot timers (`set_timer`, `cancel_timer`). | **stable** |
| `ai.query` | Tier-routed LLM calls through the Plexi AI broker (`ai_query`). Host owns the API key; apps never see it. | **stable** |
| `midi.in` | Receive MIDI 1.0 byte streams from hardware via CoreMIDI broker. | **stable** |
| `midi.out` | Send MIDI 1.0 byte streams to hardware via CoreMIDI broker. | **stable** |
| `terminal.bindings` | Drive a linked terminal pane — covers all Canvas Terminal Binding Primitives (`request_linked_terminal`, `run_in_linked_terminal`, `insert_path_token`, `request_command_preview`, `open_artifact`, `stream_process`). | **stable** |
| `fs.pick` | Show a native file picker dialog (`open_file_picker`). | **stable** |
| `panes.spawn` | Spawn a new pane via `spawn_pane`. | **stable** |

### Declaring capabilities in a manifest

```toml
[app.capabilities]
capabilities = ["fs.read", "ai.query", "pipe.open"]
```

Unknown capability strings cause `plexi install` to fail with a clear error. There are no wildcard or glob patterns — declare each capability explicitly.

---

## Deprecated APIs

### `spawn.app` → `panes.spawn` migration

`spawn.app` was the original pane-spawn capability. It is accepted for back-compat but will be removed in a future major version.

**Old (deprecated):**
```python
ctx.host({
    "type": "spawn_app",
    "type_id": "my-viewer",
    "layout": "split_v",
    "args": ["--file", path],
})
# Host replies with PlexiEvent: type == "app_spawned"
```

**New (stable):**
```python
ctx.host({
    "type": "spawn_pane",
    "type_id": "my-viewer",
    "layout": "split_v",
    "args": ["--file", path],
    "request_id": "spawn-1",  # optional, surfaces in pane_spawned/pane_spawn_error
})
# Host replies with PlexiEvent: type == "pane_spawned" or "pane_spawn_error"
```

Update your manifest capability from `spawn.app` to `panes.spawn` at the same time.

---

## Pre-v1: `video.playback`

The wire shape for video playback (`open_video`, `set_video_state`, `close_video`) is fully defined and the capability string is accepted. However, the production decoder is not yet shipped (#346). Calling `open_video` today returns `PlexiEvent::VideoOpenError` with `"video decoder not implemented"`.

Apps that need video rendering should either:
1. Wait for #346 to ship before relying on this surface.
2. Implement their own decoding and deliver frames via a binary pipe to an `audio_meter` alternative or a raw `image` draw command.

---

## See also

- [NORTH_STAR.md](NORTH_STAR.md) — product direction
- [GLOSSARY.md](GLOSSARY.md) — shared vocabulary (pane, context, PGAP, capability, secret)
- [docs/sdk-ui-guide.md](docs/sdk-ui-guide.md) — SDK UI patterns and draw command cookbook
- `src/app_protocol.rs` — ground truth for all wire types
- `src/app_permissions.rs` — capability enum and enforcement
