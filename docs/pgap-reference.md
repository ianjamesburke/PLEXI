# PGAP Protocol Reference

**PGAP** — Plexi Generic App Protocol, version 3. Newline-delimited JSON over stdin/stdout between the Plexi host and an external app process. Binary data (audio PCM, video frames, raw bytes) travels on typed pipes (unix domain sockets) — never on stdio.

---

## Contents

1. [Overview](#1-overview)
2. [Handshake](#2-handshake)
3. [Frame Loop](#3-frame-loop)
4. [PlexiEvent Catalog](#4-plexievent-catalog-host--app)
5. [DrawCommand Catalog](#5-drawcommand-catalog-app--host)
6. [Capability / Permission Flow](#6-capability--permission-flow)
7. [Typed Pipes](#7-typed-pipes)
8. [Error Handling](#8-error-handling)
9. [manifest.toml Reference](#9-manifesttoml-reference)
10. [SDK Quick-Start](#10-sdk-quick-start)

---

## 1. Overview

```
┌─────────────────────────────────────┐
│           Plexi Host                │
│                                     │
│  ┌──────────────────────────────┐   │
│  │  App process (stdin/stdout)  │   │
│  │                              │   │
│  │  PlexiEvent ──────────────►  │   │  (JSON lines on stdin)
│  │  DrawCommand ◄────────────   │   │  (JSON lines on stdout)
│  │  AppReply   ◄─────────────   │   │  (JSON line on stdout, once)
│  └──────────────────────────────┘   │
│                                     │
│  Binary typed pipes (unix sockets)  │
│  /tmp/plexi-pipe-<rand>-<id>.sock   │
│  ┌──────┐     ┌──────┐              │
│  │ PCM  │     │video │  ...         │
│  └──────┘     └──────┘              │
└─────────────────────────────────────┘
```

All messages are newline-delimited JSON. Each JSON object has a `"type"` field (snake_case). The host writes one JSON line to the app's stdin per event; the app writes one JSON line to stdout per command or reply.

Binary data never crosses the stdio boundary. When an app needs to stream audio or video, it opens a typed pipe via `PipeOpen` or `AudioCapture`. The host allocates a unix domain socket, sends a `PipeOpened` event with the path, and the app connects as a client.

---

## 2. Handshake

The handshake must complete before any frames are rendered. The host kills the app if `Ready` does not arrive within **3 seconds** of spawning.

```
Host                                    App
 │                                       │
 │  {"type":"init", "protocol":"pgap/3", │
 │   "app_id":"my-app",                  │
 │   "workspace_root":"/projects/foo",   │
 │   "capabilities":["fs.read"],         │
 │   "feature_flags":["media_v1"]}       │
 │ ────────────────────────────────────► │  (written to app stdin)
 │                                       │
 │                                       │  (app initialises internal state)
 │                                       │
 │  {"type":"ready",                     │
 │   "sdk":"plexi-sdk-py/0.4.0",         │
 │   "features_used":["media_v1"]}       │
 │ ◄──────────────────────────────────── │  (written to app stdout)
 │                                       │
 │  ── frame loop begins ──              │
```

**Rules:**
- The host sends exactly one `Init`.
- The app sends exactly one `AppReply::Ready` in response, then begins the draw loop.
- The app must refuse unknown protocol versions and exit with a non-zero code.
- `features_used` in `Ready` lists only the feature flags from `Init` that the app will actually use. Unknown flags in `Init` are silently ignored.

---

## 3. Frame Loop

```
Host                         App
 │                            │
 │  Render {frame_id: 42}     │
 │ ──────────────────────────►│
 │                            │  app calls on_render()
 │  Rect {…}                  │
 │ ◄──────────────────────────│
 │  Text {…}                  │
 │ ◄──────────────────────────│
 │  FrameDone {frame_id: 42}  │
 │ ◄──────────────────────────│
 │                            │
 │  (host composites frame)   │
```

- The host sends `Render` at its target frame rate. The app must reply with zero or more draw commands followed by `FrameDone` carrying the same `frame_id`.
- **Out-of-frame commands** (`Log`, `CapabilityRequest`, `SecretGet`, `Notify`, `AudioCapture`, `PipeOpen`, etc.) may be sent by the app at any time — before, during, or between frames. The host processes them immediately without waiting for `FrameDone`.
- Input events (`Key`, `Click`, `Command`) arrive from the host between frames as they occur. The app handles them and will draw updated state on the next `Render`.
- If the app has nothing to draw, it must still emit `FrameDone` when it receives `Render`.

---

## 4. PlexiEvent Catalog (host → app)

All events share the wire format `{"type": "<snake_case_variant>", ...fields}`.

---

### `init`

**Use when:** App is first spawned. Sent exactly once.

| Field | Type | Description |
|---|---|---|
| `protocol` | `string` | Protocol version, e.g. `"pgap/3"`. App must refuse unknown versions. |
| `app_id` | `string` | Stable identifier for this app instance, e.g. `"audio-recorder"`. |
| `workspace_root` | `string` | Absolute path to the workspace root. All `SecretGet` calls are scoped here. |
| `capabilities` | `string[]` | Capabilities granted (from manifest or runtime prompt). |
| `feature_flags` | `string[]` | Additive feature flags. Unknown flags are ignored. |

```json
{
  "type": "init",
  "protocol": "pgap/3",
  "app_id": "todo",
  "workspace_root": "/Users/ian/projects/myapp",
  "capabilities": ["fs.read", "fs.write"],
  "feature_flags": ["pane_groups_v1"]
}
```

---

### `render`

**Use when:** Host requests a new frame. App must reply with draw commands + `FrameDone`.

| Field | Type | Description |
|---|---|---|
| `frame_id` | `uint64` | Monotonically increasing frame identifier. |
| `rect` | `{x, y, w, h: float}` | Surface rect the app should fill. |

```json
{
  "type": "render",
  "frame_id": 42,
  "rect": {"x": 0.0, "y": 0.0, "w": 1200.0, "h": 800.0}
}
```

---

### `resize`

**Use when:** The pane surface dimensions changed. The app should re-layout on the next `Render`.

| Field | Type | Description |
|---|---|---|
| `width` | `float` | New surface width in logical pixels. |
| `height` | `float` | New surface height in logical pixels. |

```json
{"type": "resize", "width": 1400.0, "height": 900.0}
```

---

### `key`

**Use when:** User pressed a key while this app's pane has focus.

| Field | Type | Description |
|---|---|---|
| `key` | `string` | Key name, e.g. `"a"`, `"Enter"`, `"ArrowUp"`, `"Escape"`. |
| `modifiers` | `{shift, ctrl, alt, cmd: bool}` | Active modifier keys. |

```json
{
  "type": "key",
  "key": "ArrowDown",
  "modifiers": {"shift": false, "ctrl": false, "alt": false, "cmd": false}
}
```

---

### `click`

**Use when:** User clicked within the app's surface.

| Field | Type | Description |
|---|---|---|
| `x` | `float` | Click X in logical coordinates relative to the app surface. |
| `y` | `float` | Click Y in logical coordinates relative to the app surface. |
| `button` | `"primary" \| "secondary"` | Which mouse button. |

```json
{"type": "click", "x": 240.0, "y": 150.0, "button": "primary"}
```

---

### `command`

**Use when:** User submitted a command via the Plexi command bar while this app was active.

| Field | Type | Description |
|---|---|---|
| `text` | `string` | The raw command string the user typed. |

```json
{"type": "command", "text": "new task buy milk"}
```

---

### `capability_decision`

**Use when:** Host has resolved a `CapabilityRequest` the app sent earlier.

| Field | Type | Description |
|---|---|---|
| `request_id` | `string` | Matches the `request_id` from the app's `CapabilityRequest`. |
| `granted` | `bool` | Whether the user approved. |

```json
{"type": "capability_decision", "request_id": "req-abc-123", "granted": true}
```

---

### `secret_value`

**Use when:** Host is returning a secret the app requested via `SecretGet`.

| Field | Type | Description |
|---|---|---|
| `key` | `string` | The secret key that was requested. |
| `value` | `string \| null` | The secret value, or `null` if denied or not found. |

```json
{"type": "secret_value", "key": "OPENAI_API_KEY", "value": "sk-..."}
```

---

### `run_update`

**Use when:** A run the app started via `RunGet` has a lifecycle update.

| Field | Type | Description |
|---|---|---|
| `run_id` | `string` | Identifier for this run. |
| `status` | `string` | One of: `"pending"`, `"running"`, `"blocked_on_user"`, `"completed"`, `"failed"`. |
| `payload` | `any` | Arbitrary JSON payload associated with this update. |

```json
{
  "type": "run_update",
  "run_id": "run-xyz-789",
  "status": "completed",
  "payload": {"output": "done"}
}
```

---

### `pipe_message`

**Use when:** A JSON-mode pipe has an inbound message for the app. Not used for binary pipes.

| Field | Type | Description |
|---|---|---|
| `pipe_id` | `string` | The pipe identifier. |
| `payload` | `any` | Arbitrary JSON payload. |

```json
{"type": "pipe_message", "pipe_id": "my-json-pipe", "payload": {"event": "tick"}}
```

---

### `path_changed`

**Use when:** Any member of the app's pane group changed its working directory. Only sent to apps in a named group (see `manifest.toml` `group` field).

| Field | Type | Description |
|---|---|---|
| `cwd` | `string` | New absolute path. |

```json
{"type": "path_changed", "cwd": "/Users/ian/projects/new-dir"}
```

---

### `suspend`

**Use when:** The app is being backgrounded (host window loses focus or app pane is hidden).

No fields.

```json
{"type": "suspend"}
```

---

### `resume`

**Use when:** The app is foregrounded again after a `Suspend`.

No fields.

```json
{"type": "resume"}
```

---

### `shutdown`

**Use when:** The app pane is being closed. The process must exit cleanly within a short timeout.

No fields.

```json
{"type": "shutdown"}
```

---

### `app_spawned`

**Use when:** A `SpawnApp` command the app sent was fulfilled. Confirms the new pane.

| Field | Type | Description |
|---|---|---|
| `pane_id` | `uint64` | Pane identifier of the newly spawned app. |
| `type_id` | `string` | The `type_id` that was spawned. |

```json
{"type": "app_spawned", "pane_id": 7, "type_id": "todo"}
```

---

### `pipe_opened`

**Use when:** A binary typed pipe is ready for the app to connect to.

| Field | Type | Description |
|---|---|---|
| `pipe_id` | `string` | The pipe identifier. |
| `socket_path` | `string` | Unix domain socket path the app must connect to as a client. |

```json
{
  "type": "pipe_opened",
  "pipe_id": "mic-capture",
  "socket_path": "/tmp/plexi-pipe-391820-mic-capture.sock"
}
```

---

### `pipe_overrun`

**Use when:** The host's ring buffer for a binary pipe was full and frames were dropped (backpressure).

| Field | Type | Description |
|---|---|---|
| `pipe_id` | `string` | The pipe that overran. |
| `dropped_frames` | `uint64` | Number of frames dropped since the last `PipeOverrun`. |

```json
{"type": "pipe_overrun", "pipe_id": "mic-capture", "dropped_frames": 3}
```

---

## 5. DrawCommand Catalog (app → host)

All commands share the wire format `{"type": "<snake_case_variant>", ...fields}`.

Commands are divided into two groups:
- **Frame-scoped:** must appear between `Render` and `FrameDone`. Host batches them for compositing.
- **Out-of-frame:** processed immediately when received, independent of the frame cycle.

---

### Frame-scoped commands

---

#### `rect`

**Use when:** Filling a solid background, card, or UI region.

| Field | Type | Default | Description |
|---|---|---|---|
| `x` | `float` | — | Left edge. |
| `y` | `float` | — | Top edge. |
| `w` | `float` | — | Width. |
| `h` | `float` | — | Height. |
| `fill` | `string` | — | CSS hex color, e.g. `"#313244"`. |
| `radius` | `float` | `0.0` | Corner radius. |

```json
{"type": "rect", "x": 0.0, "y": 0.0, "w": 1200.0, "h": 800.0, "fill": "#1e1e2e", "radius": 0.0}
```

---

#### `text`

**Use when:** Rendering a label, heading, or body text.

| Field | Type | Default | Description |
|---|---|---|---|
| `x` | `float` | — | Baseline left X. |
| `y` | `float` | — | Baseline Y. |
| `text` | `string` | — | Text content. |
| `size` | `float` | — | Font size in logical pixels. |
| `color` | `string` | — | CSS hex color. |
| `monospace` | `bool` | `false` | Use monospace font. |
| `bold` | `bool` | `false` | Bold weight. |

```json
{"type": "text", "x": 20.0, "y": 40.0, "text": "Hello v3!", "size": 15.0, "color": "#cdd6f4", "monospace": false, "bold": false}
```

---

#### `line`

**Use when:** Drawing dividers, borders, or graph elements.

| Field | Type | Default | Description |
|---|---|---|---|
| `x1`, `y1` | `float` | — | Start point. |
| `x2`, `y2` | `float` | — | End point. |
| `color` | `string` | — | CSS hex color. |
| `width` | `float` | `1.0` | Stroke width. |

```json
{"type": "line", "x1": 0.0, "y1": 48.0, "x2": 1200.0, "y2": 48.0, "color": "#45475a", "width": 1.0}
```

---

#### `list`

**Use when:** Rendering a scrollable item list — host handles scrolling and item layout.

| Field | Type | Default | Description |
|---|---|---|---|
| `items` | `ListItem[]` | — | List items (see below). |
| `selected` | `uint` | — | Index of the highlighted item. |
| `item_height` | `float` | `0.0` | Row height. `0.0` lets the host choose. |

**ListItem fields:**

| Field | Type | Default | Description |
|---|---|---|---|
| `label` | `string` | — | Primary display text. |
| `secondary` | `string \| null` | `null` | Secondary/subtitle text. |
| `icon` | `string \| null` | `null` | Reserved for future use. |
| `is_dir` | `bool` | `false` | Render as a directory entry. |

```json
{
  "type": "list",
  "items": [
    {"label": "Buy milk", "secondary": null, "icon": null, "is_dir": false},
    {"label": "Fix bug #42", "secondary": "in progress", "icon": null, "is_dir": false}
  ],
  "selected": 0,
  "item_height": 40.0
}
```

---

#### `video_player`

**Use when:** Displaying a video. Host owns decoding and rendering; app declares geometry and playback state.

| Field | Type | Description |
|---|---|---|
| `source` | `string` | File path or URL to the video. |
| `x`, `y` | `float` | Top-left of the player rect. |
| `w`, `h` | `float` | Player dimensions. |
| `state` | `string` | `"play"`, `"pause"`, or `"seek:<ms>"` (e.g. `"seek:3500"`). |

```json
{
  "type": "video_player",
  "source": "/Users/ian/clips/intro.mp4",
  "x": 0.0, "y": 0.0, "w": 1920.0, "h": 1080.0,
  "state": "play"
}
```

---

#### `audio_meter`

**Use when:** Showing a real-time audio level meter fed by a binary pipe.

| Field | Type | Description |
|---|---|---|
| `x`, `y` | `float` | Top-left of the meter widget. |
| `w`, `h` | `float` | Meter dimensions. |
| `pipe_id` | `string` | Binary-mode pipe carrying PCM data. |

```json
{"type": "audio_meter", "x": 20.0, "y": 100.0, "w": 200.0, "h": 40.0, "pipe_id": "mic-capture"}
```

---

#### `frame_done`

**Use when:** All draw commands for a frame have been emitted. Required to complete every render cycle.

| Field | Type | Description |
|---|---|---|
| `frame_id` | `uint64` | Must match the `frame_id` from the triggering `Render` event. |

```json
{"type": "frame_done", "frame_id": 42}
```

---

### Out-of-frame commands

---

#### `log`

**Use when:** Forwarding app-side log messages into the host's logger (tagged with `app::<app_id>`).

| Field | Type | Description |
|---|---|---|
| `level` | `string` | One of: `"error"`, `"warn"`, `"info"`, `"debug"`. |
| `message` | `string` | Log message. |

```json
{"type": "log", "level": "info", "message": "recording started"}
```

---

#### `capability_request`

**Use when:** Requesting a capability at runtime. Host shows a modal prompt; responds with `CapabilityDecision`.

| Field | Type | Description |
|---|---|---|
| `request_id` | `string` | Caller-generated UUID. Echoed back in `CapabilityDecision`. |
| `capability` | `string` | Capability string, e.g. `"net.http"`, `"audio.record"`. |

```json
{"type": "capability_request", "request_id": "req-abc-123", "capability": "net.http"}
```

---

#### `secret_get`

**Use when:** Reading a workspace-scoped secret (API keys, tokens). Automatically scoped to `Init.workspace_root`.

| Field | Type | Description |
|---|---|---|
| `key` | `string` | Secret key name, e.g. `"OPENAI_API_KEY"`. |

```json
{"type": "secret_get", "key": "OPENAI_API_KEY"}
```

---

#### `run_get`

**Use when:** Starting a host-managed "run" (surfaced in Cmd+R palette).

| Field | Type | Description |
|---|---|---|
| `intent` | `string` | Human-readable intent, e.g. `"Summarize file"`. |
| `payload` | `any` | Arbitrary context passed to the run handler. |

```json
{"type": "run_get", "intent": "Summarize file", "payload": {"path": "/Users/ian/notes.txt"}}
```

---

#### `run_complete`

**Use when:** Signaling that a run the app owns has finished.

| Field | Type | Description |
|---|---|---|
| `run_id` | `string` | Run identifier from `RunUpdate`. |
| `result` | `any` | Final result payload. |

```json
{"type": "run_complete", "run_id": "run-xyz-789", "result": {"lines": 42}}
```

---

#### `notify`

**Use when:** Posting a notification visible to the user. Actions are optional.

| Field | Type | Default | Description |
|---|---|---|---|
| `level` | `string` | — | One of: `"info"`, `"warn"`, `"error"`. |
| `title` | `string` | — | Short notification title. |
| `body` | `string` | — | Notification body text. |
| `actions` | `NotificationAction[]` | `[]` | Optional action buttons. |

**NotificationAction fields:**

| Field | Type | Description |
|---|---|---|
| `label` | `string` | Button label. |
| `action_type` | `string` | One of: `"resume_run"`, `"open_intent"`, `"run_command"`. |
| `payload` | `any` | Action-specific data. |

```json
{
  "type": "notify",
  "level": "info",
  "title": "Recording saved",
  "body": "output.wav written to workspace",
  "actions": [
    {"label": "Open", "action_type": "run_command", "payload": {"command": "open output.wav"}}
  ]
}
```

---

#### `audio_play`

**Use when:** Playing back audio through the host audio device.

| Field | Type | Description |
|---|---|---|
| `source` | `string` | File path or `pipe_id` for binary-mode audio. |
| `volume` | `float` | Volume from `0.0` to `1.0`. |
| `state` | `string` | One of: `"play"`, `"pause"`, `"stop"`. |

```json
{"type": "audio_play", "source": "/Users/ian/output.wav", "volume": 0.8, "state": "play"}
```

---

#### `audio_capture`

**Use when:** Opening a microphone capture session. Host streams PCM to the named binary pipe and sends back `PipeOpened`.

> Note: Do not send a separate `PipeOpen` for the same `pipe_id` — `AudioCapture` allocates the pipe internally.

| Field | Type | Default | Description |
|---|---|---|---|
| `pipe_id` | `string` | — | Identifier for the audio pipe. |
| `sample_rate` | `uint32` | `48000` | Sample rate in Hz. |
| `buffer_size` | `uint32` | `512` | Buffer size in frames. |

```json
{"type": "audio_capture", "pipe_id": "mic-capture", "sample_rate": 48000, "buffer_size": 512}
```

---

#### `pipe_open`

**Use when:** Opening a general-purpose typed pipe. For audio, use `AudioCapture` instead.

| Field | Type | Description |
|---|---|---|
| `pipe_id` | `string` | Caller-chosen identifier. Must be unique per app. |
| `mode` | `string` | `"json"` or `"binary"`. |
| `direction` | `string` | `"in"`, `"out"`, or `"duplex"`. |

For binary pipes, the host replies with `PipeOpened` carrying the socket path. For JSON pipes, no socket is created — messages travel over the PGAP wire.

```json
{"type": "pipe_open", "pipe_id": "video-frames", "mode": "binary", "direction": "in"}
```

---

#### `pipe_send`

**Use when:** Sending a message on a JSON-mode pipe. Not valid for binary pipes.

| Field | Type | Description |
|---|---|---|
| `pipe_id` | `string` | Must reference an open JSON-mode pipe. |
| `payload` | `any` | Arbitrary JSON payload. |

```json
{"type": "pipe_send", "pipe_id": "control-channel", "payload": {"cmd": "stop"}}
```

---

#### `status_summary`

**Use when:** Updating the status text shown in the parent pane chrome.

| Field | Type | Description |
|---|---|---|
| `text` | `string` | Status string, e.g. `"3 items"`, `"Recording… 00:12"`. |

```json
{"type": "status_summary", "text": "12 todos · 3 done"}
```

---

#### `spawn_app`

**Use when:** Requesting the host to open a new app pane. Requires the `spawn.app` capability.

| Field | Type | Default | Description |
|---|---|---|---|
| `type_id` | `string` | — | The app identifier to spawn, e.g. `"todo"`. |
| `layout` | `string \| null` | `"split_v"` | `"split_v"` (below), `"split_h"` (right), or `"overlay"` (full pane). |
| `args` | `string[]` | `[]` | argv appended to the child process after its binary path. |

Host responds with `AppSpawned` on success.

```json
{"type": "spawn_app", "type_id": "todo", "layout": "split_h", "args": ["--filter", "active"]}
```

---

### AppReply

Sent exactly once, immediately after receiving `Init`.

#### `ready`

| Field | Type | Description |
|---|---|---|
| `sdk` | `string` | SDK identifier and version, e.g. `"plexi-sdk-py/0.4.0"`. |
| `features_used` | `string[]` | Subset of `Init.feature_flags` this app will actually use. |

```json
{"type": "ready", "sdk": "plexi-sdk-py/0.4.0", "features_used": ["pane_groups_v1"]}
```

---

### Legacy commands (v1/v2)

> **Do not use in new apps.** These commands are kept for backward compatibility only. Use `PipeOpen`/`PipeSend` for structured process communication instead.

---

#### `run_in_terminal` _(deprecated)_

Emit a shell command to the linked terminal PTY.

```json
{"type": "run_in_terminal", "command": "ls -la"}
```

---

#### `cd` _(deprecated)_

Tell the linked terminal to change directory.

```json
{"type": "cd", "path": "/Users/ian/projects"}
```

---

## 6. Capability / Permission Flow

### Manifest pre-declaration

Apps declare required capabilities in `manifest.toml`. The host grants them automatically on launch without prompting the user (subject to workspace policy).

```toml
[app.capabilities]
capabilities = ["audio.record", "fs.write", "pipe.open"]
```

Capabilities declared here appear in `Init.capabilities` when the app starts.

### Runtime capability request

An app can request capabilities it did not declare at startup. The host shows a user-facing modal; the response arrives as `CapabilityDecision`.

```
App                                  Host
 │                                    │
 │  capability_request                │
 │  {request_id: "req-1",             │
 │   capability: "net.http"}          │
 │ ──────────────────────────────────►│
 │                                    │  [host shows modal to user]
 │                                    │
 │  capability_decision               │
 │  {request_id: "req-1",             │
 │   granted: true}                   │
 │ ◄──────────────────────────────────│
```

The request is asynchronous. The `request_id` must be a unique string (typically a UUID) generated by the app. The Python SDK's `emit.capability_request(capability)` helper blocks on a `Queue` until the response arrives, so it can be called synchronously from a background thread.

**Known capability strings:**

| Capability | What it grants |
|---|---|
| `fs.read` | Read files under `workspace_root`. |
| `fs.write` | Write files under `workspace_root`. |
| `audio.record` | Microphone capture. |
| `net.http` | Outbound HTTP requests. |
| `pipe.open` | Open typed pipes. |
| `spawn.app` | Spawn child app panes. |

---

## 7. Typed Pipes

Typed pipes are the side channel for binary data — audio PCM, video frames, or any raw byte stream. They use unix domain sockets so data never crosses the stdio boundary.

### Binary pipe frame format

Every binary message is wrapped in a length-prefixed frame:

```
 ┌──────────────────────────────────────────────┐
 │  4 bytes: payload length, big-endian uint32  │
 │  N bytes: payload                            │
 └──────────────────────────────────────────────┘
```

End-of-stream is signaled by a length-0 sentinel: `\x00\x00\x00\x00`.

Maximum frame size: **1 MiB** (1,048,576 bytes). Larger frames are rejected by the host.

### Binary pipe lifecycle

```
App                                     Host
 │                                       │
 │  pipe_open / audio_capture            │
 │ ──────────────────────────────────────►
 │                                       │  [host binds unix socket]
 │  pipe_opened                          │
 │  {pipe_id, socket_path}               │
 │ ◄──────────────────────────────────── │
 │                                       │
 │  [app connects to socket_path]        │
 │ ──── unix socket connect ─────────────►
 │                                       │
 │ ══════ binary frames flow ════════════ │
 │                                       │
 │  (ring overrun → pipe_overrun event)  │
 │ ◄──────────────────────────────────── │
```

**Connection timing:** The app must connect to `socket_path` after receiving `PipeOpened`, not before. The host's drain thread starts a non-blocking `accept()` loop; if the app never connects, `close()` still completes cleanly (no deadlock).

**Backpressure:** The host uses a lock-free ring buffer (capacity 32 frames). When the ring is full, the oldest frame is evicted and the host sends a `PipeOverrun` event with `dropped_frames`. The app should log or handle overruns gracefully.

### Python example — reading binary frames

```python
pipe = self.emit.audio_capture("mic", sample_rate=48000)
pipe.connect()  # blocks until PipeOpened arrives and socket is connected
while True:
    frame = pipe.read_frame()  # returns bytes or None on EOF
    if frame is None:
        break
    process_pcm(frame)
```

### JSON pipe lifecycle

JSON pipes carry structured messages over the PGAP wire — no socket is involved.

1. App sends `PipeOpen` with `mode: "json"`.
2. App sends `PipeSend` to push a message to the host.
3. Host delivers inbound messages as `PipeMessage` events on stdin.

```json
{"type": "pipe_open", "pipe_id": "ctrl", "mode": "json", "direction": "duplex"}
{"type": "pipe_send", "pipe_id": "ctrl", "payload": {"cmd": "start"}}
```

---

## 8. Error Handling

| Scenario | Host behavior |
|---|---|
| Unknown `type` in a `DrawCommand` | Logs a `warn` entry tagged `app::<app_id>`, skips the message, continues. |
| JSON parse failure on a stdout line | Logs a `warn` entry, skips the line, continues. |
| App exits without sending `Ready` | Host waits up to 3 seconds, then kills the process. |
| `FrameDone` frame_id mismatch | Host logs `warn`, drops the frame. |
| Binary frame exceeds 1 MiB | `write_binary` returns `WriteFailed`; host logs and drops the frame. |
| App sends `PipeSend` on a binary pipe | Host logs `warn` (`"pipe is binary mode"`), ignores the message. |
| App process crashes | Host detects EOF on stdout, logs `error`, tears down the pane. |

Apps should never use `todo!()` or `unimplemented!()` in trait method implementations — a panic on the render thread freezes the entire GUI. Return `Err`, `None`, or a no-op instead.

---

## 9. manifest.toml Reference

Every app directory must contain a `manifest.toml`. The host refuses to install apps with missing required fields.

### Required fields

```toml
[app]
id      = "my-app"           # stable identifier, kebab-case
name    = "My App"           # human-readable display name
version = "0.1.0"            # semver
entry   = "my_app.py"        # entry point relative to app directory
```

### Optional fields

```toml
[app]
description = "One-line description shown in the app browser."

[app.capabilities]
# Capabilities pre-declared for automatic grant on launch.
capabilities = ["fs.read", "fs.write", "audio.record", "pipe.open", "net.http", "spawn.app"]

# Pane group membership. Apps in the same group share PathChanged broadcasts.
# The "cwd" group is built-in; the todo app uses it to track the focused terminal.
group = "cwd"

# Preferred initial layout when launched via SpawnApp.
# "split_v" (default) | "split_h" | "overlay"
layout_hint = "split_h"

# Required if this app uses spawn_app to launch children.
[app.capabilities.spawn]
app = true
```

### Worked examples

**Audio recorder** (binary pipe + audio meter):
```toml
[app]
id = "audio-recorder"
name = "Audio Recorder"
version = "0.1.0"
description = "Record audio from mic to WAV. Proves binary pipe + AudioMeter."
entry = "audio_recorder.py"

[app.capabilities]
capabilities = ["audio.record", "fs.write", "pipe.open"]
```

**Todo** (filesystem + pane group):
```toml
[app]
id = "todo"
name = "Todo"
version = "0.1.0"
description = "Simple todo list persisted to <workspace_root>/.plexi/todos.json."
entry = "todo.py"

[app.capabilities]
capabilities = ["fs.read", "fs.write"]
group = "cwd"
```

---

## 10. SDK Quick-Start

The Python SDK (`sdk/python/plexi_sdk.py`) is a zero-dependency stdlib implementation. Copy it into your app directory and subclass `App`.

```python
from __future__ import annotations  # required for Python 3.9 compat in bundles
from plexi_sdk import App, RenderContext, BG, FG, BODY, PAD

class CounterApp(App):

    def on_init(self, ctx: RenderContext) -> None:
        self.count = 0
        ctx.emit.status_summary("0 clicks")

    def on_render(self, ctx: RenderContext) -> None:
        # Background
        ctx.rect(0, 0, ctx.w, ctx.h, fill=BG)
        # Label
        label = f"Count: {self.count}"
        ctx.text(PAD, PAD + BODY, label, size=BODY, color=FG)
        # FrameDone is emitted automatically by the SDK after on_render returns.

    def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
        if key == "space":
            self.count += 1
            ctx.emit.status_summary(f"{self.count} clicks")
        elif key == "r":
            self.count = 0

    def on_click(self, ctx: RenderContext, x: float, y: float, button: str) -> None:
        if button == "primary":
            self.count += 1
            ctx.emit.status_summary(f"{self.count} clicks")

    def on_shutdown(self) -> None:
        pass  # clean up resources here if needed

CounterApp().run()
```

**Key SDK conventions:**

- `ctx.emit` is an `Emitter` — use it for out-of-frame commands (`notify`, `secret_get`, `pipe_open`, `audio_capture`, etc.).
- `ctx.emit.secret_get(key)` and `ctx.emit.capability_request(cap)` **block** until the host responds. Call from a background thread if the app must remain responsive.
- `FrameDone` is emitted automatically after `on_render` returns. Do not emit it manually.
- `emit.info/warn/error/debug(msg)` forward log messages into the host logger under the `app::<app_id>` tag.
- Theme constants (`BG`, `FG`, `ACCENT`, `PAD`, `BODY`, etc.) are exported from the SDK for consistent visual style across apps.

**macOS bundle note:** GUI app bundles do not inherit shell `PATH`. Always add `from __future__ import annotations` as the first line of every Python file — macOS ships Python 3.9 at `/usr/bin/python3` and `str | None` syntax requires this import on 3.9.
