---
title: "Emitter"
description: "Host commands, notifications, HTTP, secrets, and device APIs"
verified_version: "3.6.54"
---

# Emitter

Emit out-of-frame commands to Plexi. Thread-safe.

Available as `self.emit` on App or `ctx.emit` on RenderContext. All methods are thread-safe.

## Methods

### `log`

```python
def log(level: str, message: str) -> None
```

---

### `info`

```python
def info(message: str) -> None
```

---

### `warn`

```python
def warn(message: str) -> None
```

---

### `error`

```python
def error(message: str) -> None
```

---

### `debug`

```python
def debug(message: str) -> None
```

---

### `notify`

```python
def notify(title: str, priority: int, body: str = '', level: str = 'info', actions: 'list | None' = None, image_inline: 'dict | None' = None, image_pipe_id: 'str | None' = None, timeout_secs: 'int | None' = None, on_dismiss: 'str | None' = None) -> None
```

Post a message notification. The modal shows title + body and a
single Acknowledge button; Enter / Space acknowledge, Esc dismisses
(unless required=True — use `notify_and_wait` for that flow).

`priority` is required (int, higher = more urgent).
`actions` is the legacy side-effect list (action_type =
resume_run | open_intent | run_command). It does NOT render UI.
`image_inline` (#74): {"mime": "image/png"|"image/jpeg",
"base64": str}. Decoded host-side; >50KB triggers a placeholder.
`image_pipe_id` (#74): pipe id of a binary ring carrying RGBA frames
prefixed with width/height u32-LE. Mutually exclusive with
`image_inline` — host warns + drops the pipe ref if both set.

---

### `run_sync`

```python
def run_sync(coro: 'Any') -> Any
```

Run a coroutine from a background thread and return its result.

Use this when calling async Emitter methods from a non-async context
(e.g. inside a ``threading.Thread`` callback):

    def _worker(self) -> None:
        result = self.emit.run_sync(self.emit.http_get(url))

Must NOT be called from within the event loop thread — it will deadlock.
Raises ``RuntimeError`` if there is no running event loop to dispatch to
(e.g. called before ``App.run()`` starts).

---

### `schedule_task`

```python
def schedule_task(coro: 'Any') -> Any
```

Schedule a coroutine as a background asyncio task from any context.

Safe to call from sync hooks (on_render, on_key, etc.) or background
threads. Returns immediately without blocking. The coroutine runs in
the background; use this when you don't need the return value:

    def on_render(self, ctx: RenderContext) -> None:
        if self._btn.render(ctx):
            self.emit.schedule_task(self._do_query())

Raises ``RuntimeError`` if the event loop hasn't started yet.

---

### `notify_choice`

```python
async def notify_choice(title: str, options: list, priority: int, body: str = '', level: str = 'info', required: bool = False, image_inline: 'dict | None' = None, image_pipe_id: 'str | None' = None, timeout_secs: 'int | None' = None, on_dismiss: 'str | None' = None) -> str
```

Post a choice notification and await until the user picks one.

`options` is a list of dicts: {"label": str, "value": str (optional),
"shortcut": str (optional, single char)}. If `value` is omitted, the
label is returned. Issue #74's structured-choice spec maps directly:
`key` → `shortcut`, `payload` → `value`.

`priority` is required (int, higher = more urgent).
`image_inline` (#74): {"mime", "base64"} — same shape as `notify`.
`image_pipe_id` (#74): pipe id of a binary ring carrying RGBA frames.
Returns the chosen option's value (or the label if no value set). If
`required=False` the user may cancel with Esc — this returns the string
`"__cancel__"`.

Await this from async hooks. From background threads use
``self.emit.run_sync(self.emit.notify_choice(...))``.

---

### `notify_with_image`

```python
async def notify_with_image(title: str, body: str, image_bytes: bytes, mime: str, priority: int, level: str = 'info', choices: 'list | None' = None) -> 'str | None'
```

Post a notification with an inline base64-encoded image attachment.

`image_bytes` must be ≤ 50_000 bytes — anything larger raises
`ValueError` here so the app sees a fast, local failure instead of
having the host silently render a placeholder. Use the pipe-ref
path (`emit.notify(..., image_pipe_id=...)`) for larger images.

`mime` must be `"image/png"` or `"image/jpeg"`.

`choices` is optional. When None this posts a fire-and-forget
message-kind notification (returns None). When set, this routes to
`notify_choice` and awaits until the user picks one (returns the
chosen value, or `"__cancel__"`).

Await this from async hooks. From background threads use
``self.emit.run_sync(self.emit.notify_with_image(...))``.

---

### `notify_input`

```python
async def notify_input(title: str, priority: int, prompt: str = '', body: str = '', level: str = 'info', required: bool = False, timeout_secs: 'int | None' = None, on_dismiss: 'str | None' = None) -> str
```

Post an input notification and await until the user submits or
cancels. Returns the typed text (possibly empty), or "__cancel__" if
the user dismissed with Esc (only possible when required=False).

`priority` is required (int, higher = more urgent).

Await this from async hooks. From background threads use
``self.emit.run_sync(self.emit.notify_input(...))``.

---

### `notify_and_wait`

```python
async def notify_and_wait(title: str, priority: int, body: str = '', level: str = 'info', actions: 'list | None' = None, timeout_secs: 'int | None' = None, on_dismiss: 'str | None' = None) -> str
```

Post a message notification and await until the user acknowledges
or cancels. Returns "acknowledge" on Enter/Space/button, "cancel" on Esc.

For richer interaction, use `notify_choice` or `notify_input`.
`priority` is required (int, higher = more urgent).
`actions` is the legacy server-side side-effect list.

Await this from async hooks. From background threads use
``self.emit.run_sync(self.emit.notify_and_wait(...))``.

---

### `notify_async`

```python
def notify_async(title: str, priority: int, body: str = '', level: str = 'info', actions: 'list | None' = None, image_inline: 'dict | None' = None, image_pipe_id: 'str | None' = None, timeout_secs: 'int | None' = None, on_dismiss: 'str | None' = None, on_response: 'Any' = None) -> str
```

Non-blocking notify_and_wait. Returns immediately with notify_id.

If on_response is None, behaves like fire-and-forget notify() — returns "".
If on_response is set, registers the callback; it receives "acknowledge"
or "__cancel__" when the user interacts.

`priority` is required (int, higher = more urgent).

---

### `notify_choice_async`

```python
def notify_choice_async(title: str, options: list, priority: int, body: str = '', level: str = 'info', required: bool = False, image_inline: 'dict | None' = None, image_pipe_id: 'str | None' = None, timeout_secs: 'int | None' = None, on_dismiss: 'str | None' = None, on_response: 'Any' = None) -> str
```

Non-blocking notify_choice. Returns immediately with notify_id.

`on_response` receives the chosen option's value (or label if no value
set), or "__cancel__" if the user dismissed (required=False only).
`priority` is required (int, higher = more urgent).

---

### `notify_input_async`

```python
def notify_input_async(title: str, priority: int, prompt: str = '', body: str = '', level: str = 'info', required: bool = False, timeout_secs: 'int | None' = None, on_dismiss: 'str | None' = None, on_response: 'Any' = None) -> str
```

Non-blocking notify_input. Returns immediately with notify_id.

`on_response` receives the typed text (possibly empty), or "__cancel__"
if the user dismissed (required=False only).
`priority` is required (int, higher = more urgent).

---

### `notify_and_wait_async`

```python
def notify_and_wait_async(title: str, priority: int, body: str = '', level: str = 'info', actions: 'list | None' = None, timeout_secs: 'int | None' = None, on_dismiss: 'str | None' = None, on_response: 'Any' = None) -> str
```

Non-blocking notify_and_wait. Returns immediately with notify_id.

`on_response` receives "acknowledge" on Enter/Space/button, or
"__cancel__" on Esc.
`priority` is required (int, higher = more urgent).

---

### `run_in_terminal`

```python
def run_in_terminal(command: str) -> None
```

---

### `cd`

```python
def cd(path: str) -> None
```

---

### `status_summary`

```python
def status_summary(text: str) -> None
```

---

### `push_nav`

```python
def push_nav(view_id: str, title: str) -> None
```

Push a new view onto the host navigation stack.

The host appends an entry keyed on `view_id` with display `title`
and shows a back arrow + `title` in the pane chrome while this view
is active. Cmd+[ (or clicking the back arrow) sends
``PlexiEvent::NavBack { view_id }`` back to the app, where
``view_id`` is the view being navigated *back to* (the entry below
the current top, or empty string for root).

The app is responsible for tracking its own internal view state and
calling ``pop_nav()`` after navigating back.

---

### `pop_nav`

```python
def pop_nav() -> None
```

Pop the current view off the host navigation stack.

Call this after the app has already rendered the previous view (e.g.
inside the ``on_nav_back`` handler). The host removes the top entry
from the stack; if the stack becomes empty the back arrow disappears.

---

### `open_file_picker`

```python
def open_file_picker(request_id: str, filter: 'list[str] | None' = None, multiple: bool = False) -> None
```

Show a native macOS file picker dialog. Requires ``fs.pick`` capability.

The host responds asynchronously with either:
- ``on_file_picked(ctx, request_id, paths)`` — one or more files selected
- ``on_file_pick_cancelled(ctx, request_id)`` — user dismissed the dialog

Args:
    request_id: Caller-supplied correlation ID echoed back in the response.
    filter: File extension whitelist without leading dots
            (e.g. ``["mp4", "mov"]``). ``None`` or empty = all files.
    multiple: Allow picking more than one file.

---

### `spawn_pane`

```python
def spawn_pane(type_id: str, layout: str = 'split_v', args: 'list[str] | None' = None, pipe_id: 'str | None' = None, from_pane_id: 'int | None' = None, request_id: 'str | None' = None, target_context: 'int | None' = None) -> None
```

Request the host to open a new pane with the given app (#592).

Requires the ``panes.spawn`` capability. The host responds with
``on_pane_spawned(pane_id, request_id)`` on success or
``on_pane_spawn_error(reason, request_id)`` on failure.

Args:
    type_id: App manifest id or ``"terminal"``.
    layout: One of ``"split_v"`` (default, below), ``"split_h"`` (right),
            ``"split_above"``, ``"split_left"``, ``"overlay"``.
    args: Extra argv passed to the spawned app.
    pipe_id: Optional. If set, the host appends ``--pipe=<id>`` to the
             spawned app's args so it can reply via ``emit.pipe_send()``.
    from_pane_id: Optional pane_id to split relative to instead of the
                  currently focused pane.
    request_id: Optional correlation id echoed back in on_pane_spawned /
                on_pane_spawn_error.
    target_context: Optional context_id to spawn the pane into. Must be
                    the calling pane's own context or a descendant (#1518).

---

### `query_context_state`

```python
def query_context_state(context_id: int) -> None
```

Query the state of a context (#1518).

The host responds with ``on_context_state(state)`` containing pane
count, child contexts, and aggregate status. The requesting pane must
be in the queried context itself or in an ancestor context (visibility
boundary). Non-descendant queries are silently denied.

Args:
    context_id: The context to query.

---

### `cd_to`

```python
def cd_to(cwd: str) -> None
```

Request the host to cd all terminals in the same pane group to `cwd`.

---

### `copy_to_clipboard`

```python
def copy_to_clipboard(text: str) -> None
```

Write `text` to the OS clipboard via the host (issue #146).

Routed through `egui::Context::copy_text` so the platform backend
(NSPasteboard / X11 / Wayland / Win32) handles the actual write.
Synchronous from the app's perspective — no acknowledgement event.
No capability flag is required; clipboard writes are low-risk and
the app already chooses when to fire (key handler, button, etc.).

---

### `request_linked_terminal`

```python
async def request_linked_terminal(cwd: 'str | None' = None, label: 'str | None' = None) -> int
```

Ask the host to open a fresh terminal pane next to this app and
return its `terminal_pane_id` (an integer). Awaits until the host
emits `LinkedTerminalReady`.

Raises `CapabilityDeniedError` if the manifest doesn't declare
`terminal.bindings`.

Await this from async hooks. From background threads use
``self.emit.run_sync(self.emit.request_linked_terminal(...))``.

---

### `run_in_linked_terminal`

```python
def run_in_linked_terminal(terminal_pane_id: int, command: str, echo: bool = True) -> None
```

Run `command` in the linked terminal. With `echo=True` the user
sees the command typed into the terminal; with `echo=False` the
signalled intent is silent execution (PTY-level echo is shell-
controlled — the flag is best-effort observational).

Fire-and-forget — capability denial drops silently with a host
log line.

---

### `insert_path_token`

```python
def insert_path_token(terminal_pane_id: int, path: str, mode: str = 'replace') -> None
```

Inject `path` at the linked terminal's cursor.

`mode = "replace"` — Ctrl-W (kill-word) before the path so the
                    shell readline removes the partial token.
`mode = "append"`  — write the path verbatim.

The host POSIX-quotes the path when it contains shell metacharacters.
Fire-and-forget — capability denial drops silently with a host log
line.

---

### `request_command_preview`

```python
async def request_command_preview(terminal_pane_id: int, command: str) -> 'tuple[str, str]'
```

Return `(command, would_run_in_cwd)` for `command` in the linked
terminal. Doesn't execute. Useful for confirmation modals before
destructive operations.

If the host denies the request, `would_run_in_cwd` is empty string;
the SDK does NOT raise — apps that want to be loud should check for
empty cwd themselves (the most common failure mode is a missing
capability, which the host already logs).

Await this from async hooks. From background threads use
``self.emit.run_sync(self.emit.request_command_preview(...))``.

---

### `open_artifact`

```python
def open_artifact(path: str, mode: str = 'open_in_pane') -> None
```

Open a workspace artifact via the host.

Modes:
  - `"open_in_pane"`        — directories open the file browser
                              next to the app; files open with
                              the OS default app (v3.5).
  - `"reveal_in_finder"`    — `open -R <path>` on macOS.
  - `"open_with_default"`   — `open <path>` on macOS.

Fire-and-forget. Capability denial drops silently with a host log
line.

---

### `schedule_render`

```python
def schedule_render(after_ms: int = 16) -> None
```

Ask the host to send a new Render event after `after_ms` milliseconds.
Call at the end of on_render to sustain a game/animation loop.
16 ms ≈ 60 fps.  32 ms ≈ 30 fps.

---

### `run_get`

```python
def run_get(intent: str, payload: Any = None) -> str
```

---

### `capability_request`

```python
async def capability_request(capability: str) -> bool
```

Await until host grants or denies the capability. Returns True if granted.

Await from async hooks. From background threads:
``self.emit.run_sync(self.emit.capability_request(capability))``.

---

### `secret_get`

```python
async def secret_get(key: str) -> 'str | None'
```

Await until host returns the secret value (or None if denied).

From background threads:
``self.emit.run_sync(self.emit.secret_get(key))``.

---

### `get_secret`

```python
async def get_secret(key: str) -> 'str | None'
```

Alias for secret_get(). Preferred name going forward.

---

### `http_get`

```python
async def http_get(url: str) -> str
```

HTTP GET brokered through the host. Requires net.http capability.
Raises RuntimeError on failure.

From background threads:
``self.emit.run_sync(self.emit.http_get(url))``.

---

### `http_request`

```python
async def http_request(url: str, method: str = 'GET', headers: 'dict[str, str] | None' = None, body: 'str | None' = None) -> str
```

HTTP request brokered through the host. Requires net.http capability.
Supports custom method, headers, and body. Raises RuntimeError on failure.

From background threads:
``self.emit.run_sync(self.emit.http_request(...))``.

---

### `load_image`

```python
async def load_image(src: str) -> str
```

Request an async image fetch brokered through the host. Requires net.http capability.

Returns a stable handle ID. Pass the handle to ``ctx.image(handle, x, y, w, h)``
to render. While loading, ``ctx.image()`` renders a placeholder rect. On load
completion the host fires ``PlexiEvent::ImageLoaded`` which triggers a re-render.

Raises ``RuntimeError`` immediately if ``net.http`` is not declared in the manifest.
Raises ``RuntimeError`` if the fetch fails (network error, bad URL, decode error).

From background threads:
``self.emit.run_sync(self.emit.load_image(url))``.

---

### `ai_query`

```python
async def ai_query(model_tier: str, system: str, messages: 'list[dict]', tools: 'list[dict] | None' = None) -> AiResponse
```

Call into the host's Plexi AI broker (#284). Requires the
`ai.query` capability declared in `manifest.toml`.

Args:
    model_tier: one of "low" | "medium" | "high" — the host maps these
        to Haiku / Sonnet / Opus respectively.
    system: system prompt (may be empty string).
    messages: list of `{"role": "user"|"assistant", "content": str}`
        dicts. At least one message is required.
    tools: optional extra tool defs to include in this query. Tools
        exposed by other panes in the same context via ``@self.tool()``
        or ``expose_tools()`` are automatically injected by the host —
        you do not need to pass them here.

Returns:
    AiResponse with `content`, `tokens_in`, `tokens_out`.

Raises:
    CapabilityDeniedError: app didn't declare `ai.query` in its manifest
        (or the host returned any other "capability denied" error).
    RuntimeError: backend failed (e.g. missing API key, network error).

From background threads:
``self.emit.run_sync(self.emit.ai_query(...))``.

---

### `mcp_tool_result`

```python
def mcp_tool_result(call_id: str, result: 'dict | None' = None, error: 'str | None' = None) -> None
```

Send a McpToolResult response for a pending McpToolCall.

One of ``result`` or ``error`` must be provided (not both, not neither).
``result`` should be a MCP CallToolResult dict, e.g.:
``{"content": [{"type": "text", "text": "done"}]}``.

---

### `expose_tools`

```python
def expose_tools(tools: 'list[dict]') -> None
```

Declare callable tools to the host (#398, #399).

Each entry in `tools` must have:
  - ``name`` (str): unique tool identifier.
  - ``description`` (str): human-readable description for the LLM.
  - ``input_schema`` (dict): JSON Schema object (type=object).
  - ``timeout_ms`` (int, optional): max ms to wait for result (default 30 000).

The host registers these tools in the global registry. When an AI turn
requests a tool call the host sends a ``PlexiEvent::ToolCall``; the SDK
routes it to the handler registered via ``@app.tool(…)``.

---

### `list_midi_devices`

```python
async def list_midi_devices(timeout: float = 5.0) -> 'MidiDeviceList'
```

Enumerate CoreMIDI input + output ports (#320).
No capability gate — port names are publicly visible in Audio MIDI
Setup. Requires `midi.in` / `midi.out` only on subsequent open/send.

Returns a MidiDeviceList with `inputs` and `outputs` lists of
MidiPortInfo (id, name, default).

Raises RuntimeError on host enumeration failure or timeout.

From background threads:
``self.emit.run_sync(self.emit.list_midi_devices())``.

---

### `list_audio_devices`

```python
async def list_audio_devices(timeout: float = 5.0) -> 'AudioDeviceList'
```

Enumerate CoreAudio input + output devices (#341).
No capability gate — device names are publicly visible.
Requires ``audio.record`` / ``audio.playback`` only on subsequent capture/play.

Returns an AudioDeviceList with ``inputs`` and ``outputs`` lists of
AudioDeviceInfo (id, name, default).

Raises RuntimeError on host enumeration failure or timeout.

From background threads:
``self.emit.run_sync(self.emit.list_audio_devices())``.

---

### `open_midi_input`

```python
def open_midi_input(port_id: str, pipe_id: str) -> 'Pipe'
```

Open a CoreMIDI input port and pipe its MIDI byte streams into a
binary pipe. Requires `midi.in` capability declared in manifest.toml.

Returns the Pipe handle. The host emits `PipeOpened` then
`MidiInputOpened`; the Pipe auto-connects to its socket on
`PipeOpened`. Apps then call `pipe.read_frame()` in a loop to
receive 1–3 byte MIDI messages (channel-voice / system real-time).

Each pipe frame is one MIDI 1.0 message — apps don't need to parse
message boundaries themselves.

---

### `close_midi_input`

```python
def close_midi_input(port_id: str) -> None
```

Close a previously-opened MIDI input port. The associated binary
pipe drains and closes; the app should drop its Pipe handle.

---

### `send_midi`

```python
def send_midi(port_id: str, bytes_: 'bytes | bytearray | list[int]') -> None
```

Fire-and-forget send of one MIDI 1.0 byte stream to `port_id`.
Requires `midi.out` capability. The host opens the output port lazily
on the first send and reuses the handle afterwards.

`bytes_` is a 1–3 byte sequence: NoteOn `[0x90+ch, note, vel]`,
NoteOff `[0x80+ch, note, vel]`, CC `[0xB0+ch, num, val]`, clock
pulse `[0xF8]`, etc. Use `plexi_sdk.midi` helpers to construct
messages with named arguments.

Successful sends produce no event; failures arrive as a logged
`midi_send_error` warning. Apps that need explicit error handling
should validate the destination via `list_midi_devices()` first.

---

### `open_video`

```python
async def open_video(source: str, pipe_id: str, timeout: float = 10.0) -> 'VideoHandle'
```

Open a video decoder against `source` and return a VideoHandle
carrying the negotiated width/height/fps/duration_ms plus the
attached Pipe (#345). Requires `video.playback` capability.

**Pre-v1 note:** `video.playback` is not production-ready. The
`file:///` decoder returns NotImplemented in all shipping builds
until AVFoundation backing lands (#346). Only `mock://gradient`
works today. Do not ship apps that depend on real video decoding.

Decoded frames flow as raw RGBA8 packets on the Pipe — one packet
per video frame, length `width * height * 4`. Apps spin a reader
thread that calls `handle.pipe.read_frame()` and either renders
the bytes (if a frame-render API is available) or surfaces a
frame counter.

Source schemes:
  - `mock://gradient` — procedural mock decoder (always works).
  - `file:///...` — production decoder; returns NotImplemented
    until #346 lands AVFoundation backing.

Raises CapabilityDeniedError if `video.playback` is not declared.
Raises RuntimeError on any other failure (NotImplemented, bad
source, decoder error, timeout).

From background threads:
``self.emit.run_sync(self.emit.open_video(source, pipe_id))``.

---

### `set_video_state`

```python
def set_video_state(handle_id: int, state: str, position_ms: int = 0) -> None
```

Drive playback for a video opened with `open_video` (#345).
`state` is one of `"play"`, `"pause"`, `"seek"`. For `"seek"`,
`position_ms` is the absolute target position in milliseconds.
Fire-and-forget — no response event.

---

### `close_video`

```python
def close_video(handle_id: int) -> None
```

Close a previously-opened video handle (#345). The host tears down
the decoder and drains the binary pipe. No response event.

---

### `audio_capture`

```python
def audio_capture(pipe_id: str, sample_rate: int = 48000, buffer_size: int = 512, device_id: 'str | None' = None) -> 'Pipe'
```

Start mic capture (#277). PCM frames stream as raw f32 PCM on
a binary pipe — interleaved per-channel, ``sample_rate`` per
channel per second.

Returns the binary Pipe immediately (negotiated values arrive
async via ``PlexiEvent::AudioCaptureStarted`` and the host log;
the app reads ``handle.read_frame()`` once it connects). Failure
arrives as ``PlexiEvent::AudioCaptureError``.

If ``pipe_id`` is already registered (i.e. the app is restarting
capture), the old Pipe is closed before the new one is created —
prevents OSError on the reader thread's recv() from a stale socket.

Requires ``audio.in`` capability. Raises ``CapabilityDeniedError``
only when the gate fires synchronously at the wire layer; the
usual TCC mic-permission denial surfaces async on the first frame
attempt as an ``AudioCaptureError`` event.

---

### `stop_audio_capture`

```python
def stop_audio_capture(pipe_id: str) -> None
```

Stop an active audio capture and close its pipe (#862).

Closes the Pipe registered under ``pipe_id`` and removes it from
the internal registry. The host cleans up the stale session on the
next ``AudioCapture`` for the same ``pipe_id``. No host command is
needed — closing the socket is sufficient.

Apps should join any reader thread after calling this to avoid
reading from a closed socket:

    self.emit.stop_audio_capture("mic")
    self._reader_thread.join(timeout=2.0)

No-op if ``pipe_id`` is not registered.

---

### `audio_play`

```python
def audio_play(source: str, volume: float = 1.0) -> None
```

Play an audio file via the host (rodio). Requires audio.playback capability.

source: absolute path to audio file (mp3, wav, ogg, etc.)
volume: playback volume 0.0–1.0 (default 1.0)

Fire-and-forget — no response event. Playback stops when the pane
closes; there is no explicit stop API yet.

---

### `set_timer`

```python
def set_timer(timer_id: str, after_ms: int) -> None
```

Fire PlexiEvent::Timer after after_ms milliseconds. Requires timer capability.

---

### `cancel_timer`

```python
def cancel_timer(timer_id: str) -> None
```

Cancel a pending timer set with set_timer().

---

### `set_mouse_tracking`

```python
def set_mouse_tracking(enabled: bool) -> None
```

Enable or disable PlexiEvent::MouseMove delivery for this pane.

MouseMove is off by default to avoid flooding apps that don't need
continuous pointer tracking. Call set_mouse_tracking(True) after
on_init to start receiving on_mouse_move callbacks. Call with False
to stop.

---

### `pipe_open`

```python
def pipe_open(pipe_id: str, mode: str = 'binary', direction: str = 'in') -> 'Pipe'
```

Open a typed pipe. Returns a Pipe handle. mode: json|binary. direction: in|out|duplex.

---

### `pipe_open_directed`

```python
def pipe_open_directed(pipe_id: str, target_pane_id: int) -> 'Pipe'
```

Open a *directed* JSON pipe to one specific target pane (#286).

Mirrors `pipe_open` but the host scopes `PipeMessage` delivery so
only the caller and `target_pane_id` see traffic on `pipe_id` —
peers that coincidentally `pipe_open` the same id stay isolated.
Always JSON-mode duplex; `pipe.send(payload)` is symmetric on
either end.

Capability: `pipe.open` (same as `pipe_open`).

---

### `pipe_send`

```python
def pipe_send(pipe_id: str, payload: Any) -> None
```

Send a JSON payload on an already-open pipe by id (#286).

Use this when you don't own a `Pipe` handle locally — for example
when replying on a directed pipe a *peer* opened (you're the target;
the host subscribed you, but the SDK doesn't auto-build a handle).
Sends a `pipe_send` DrawCommand; the host routes per the existing
directed-pipe pair table.

---

