---
title: "App"
description: "Base class for Plexi pane apps with lifecycle hooks"
verified_version: "3.6.54"
---

# App

Base class for Plexi v3 apps. Subclass and override event handlers.

Override any of:

Awaited (block the event loop until they return):
    on_init(ctx)                                — after Init handshake
    on_render(ctx)                              — on each Render event
    on_suspend()                                — on Suspend
    on_resume()                                 — on Resume
    on_shutdown()                               — on Shutdown

Task (dispatched as asyncio tasks — event loop continues):
    on_key(ctx, key, mods)                      — on Key event
    on_click(ctx, x, y, button)                 — on Click event
    on_mouse_down(ctx, x, y, button)            — on MouseDown event
    on_mouse_up(ctx, x, y, button)              — on MouseUp event
    on_mouse_move(ctx, x, y, buttons)           — on MouseMove event
    on_command(ctx, text)                        — on Command event
    on_paste(ctx, text)                          — on Paste event
    on_text_submitted(ctx, id, text)             — on TextInput Enter press
    on_pipe_message(ctx, pipe_id, payload)       — on PipeMessage
    on_path_changed(ctx, cwd)                    — on PathChanged
    on_inject(ctx, payload)                      — on Inject event
    on_nav_back(ctx, view_id)                    — on NavBack event
    on_timer(ctx, timer_id)                      — on Timer event
    on_scroll(ctx, id, offset_y)                 — on Scroll event
    on_file_picked(ctx, request_id, paths)       — on FilePicked event
    on_file_pick_cancelled(ctx, request_id)      — on FilePickCancelled
    on_mcp_call(ctx, tool_name, arguments)       — on MCP tool call

Fire-and-forget (no RenderContext — called outside a render frame):
    on_app_spawned(pane_id, type_id)             — app spawn succeeded
    on_pane_spawned(pane_id, request_id)         — pane spawn succeeded
    on_pane_spawn_error(reason, request_id)      — pane spawn failed
    on_midi_input_opened(pipe_id, port_id, port_name) — MIDI input opened

Task handlers are dispatched as asyncio tasks — the event loop does not
wait for them to complete before processing the next event. Declare them
``async def`` whenever they do any I/O. Never call blocking operations
(time.sleep, requests.get, etc.) directly from these handlers; use
``await asyncio.to_thread(fn)`` or ``threading.Thread`` + ``emit.run_sync()``.

Awaited handlers block the event loop until they return. Use ``await``-able
Emitter helpers freely; they do not deadlock because the stdin reader runs
as a concurrent task.

Fire-and-forget handlers do not receive a RenderContext because they are
dispatched outside a render frame.

## Methods

### `on_init`

```python
def on_init(_ctx: RenderContext) -> 'Coroutine[Any, Any, None] | None'
```

---

### `on_render`

```python
def on_render(_ctx: RenderContext) -> None
```

---

### `on_key`

```python
def on_key(_ctx: RenderContext, _key: str, _mods: dict) -> 'Coroutine[Any, Any, None] | None'
```

---

### `on_click`

```python
def on_click(_ctx: RenderContext, _x: float, _y: float, _button: str) -> 'Coroutine[Any, Any, None] | None'
```

---

### `on_mouse_down`

```python
def on_mouse_down(_ctx: RenderContext, _x: float, _y: float, _button: str) -> 'Coroutine[Any, Any, None] | None'
```

---

### `on_mouse_up`

```python
def on_mouse_up(_ctx: RenderContext, _x: float, _y: float, _button: str) -> 'Coroutine[Any, Any, None] | None'
```

---

### `on_mouse_move`

```python
def on_mouse_move(_ctx: RenderContext, _x: float, _y: float, _buttons: list) -> 'Coroutine[Any, Any, None] | None'
```

---

### `on_command`

```python
def on_command(_ctx: RenderContext, _text: str) -> 'Coroutine[Any, Any, None] | None'
```

---

### `on_paste`

```python
def on_paste(_ctx: RenderContext, _text: str) -> 'Coroutine[Any, Any, None] | None'
```

---

### `on_pipe_message`

```python
def on_pipe_message(_ctx: RenderContext, _pipe_id: str, _payload: Any) -> 'Coroutine[Any, Any, None] | None'
```

---

### `on_path_changed`

```python
def on_path_changed(_ctx: RenderContext, _cwd: str) -> 'Coroutine[Any, Any, None] | None'
```

---

### `on_inject`

```python
def on_inject(_ctx: RenderContext, _payload: Any) -> 'Coroutine[Any, Any, None] | None'
```

---

### `on_nav_back`

```python
def on_nav_back(_ctx: RenderContext, _view_id: str) -> 'Coroutine[Any, Any, None] | None'
```

Called when the host emits ``NavBack`` — user pressed Cmd+[ or the
back arrow in the pane chrome. ``view_id`` is the view being navigated
*back to* (the new top of stack, or empty string for root).

The app should update its own view state to show ``view_id``, then call
``ctx.emit.pop_nav()`` to remove the entry from the host stack.

---

### `on_app_spawned`

```python
def on_app_spawned(_pane_id: int, _type_id: str) -> None
```

---

### `on_pane_spawned`

```python
def on_pane_spawned(_pane_id: int, _request_id: 'str | None' = None) -> None
```

Called when a SpawnPane request succeeded (#592). Override to track the spawned pane.

---

### `on_pane_spawn_error`

```python
def on_pane_spawn_error(_reason: str, _request_id: 'str | None' = None) -> None
```

Called when a SpawnPane request failed (#592). Override to handle the error.

---

### `on_timer`

```python
def on_timer(_ctx: RenderContext, _timer_id: str) -> 'Coroutine[Any, Any, None] | None'
```

---

### `on_scroll`

```python
def on_scroll(_ctx: RenderContext, _id: str, _offset_y: float) -> 'Coroutine[Any, Any, None] | None'
```

---

### `on_text_submitted`

```python
def on_text_submitted(_ctx: RenderContext, _id: str, _text: str) -> 'Coroutine[Any, Any, None] | None'
```

---

### `on_file_picked`

```python
def on_file_picked(_ctx: RenderContext, _request_id: str, _paths: 'list[str]') -> 'Coroutine[Any, Any, None] | None'
```

Called when the user selected one or more files in the picker.

``_request_id`` matches the id passed to ``ctx.emit.open_file_picker``.
``_paths`` is a list of absolute file paths chosen by the user.

---

### `on_file_pick_cancelled`

```python
def on_file_pick_cancelled(_ctx: RenderContext, _request_id: str) -> 'Coroutine[Any, Any, None] | None'
```

Called when the user dismissed the file picker without selecting a file,
or if the ``fs.pick`` capability was not declared.

---

### `on_mcp_call`

```python
def on_mcp_call(_ctx: RenderContext, _tool_name: str, _arguments: dict) -> 'dict | Coroutine[Any, Any, dict] | None'
```

Called when an external MCP client calls a tool declared in [app.mcp].

Override to handle the call and return a result dict. The result should
follow MCP CallToolResult schema: ``{"content": [{"type": "text", "text": "..."}]}``.
Return ``None`` to respond with a generic 'not implemented' error.

---

### `on_midi_input_opened`

```python
def on_midi_input_opened(_pipe_id: str, _port_id: str, _port_name: str) -> None
```

Override to react to a successful OpenMidiInput. Apps that just
want the byte stream typically read directly from the binary pipe
opened alongside this event — Plexi sends `pipe_opened` first.

---

### `tool`

```python
def tool(name: str, description: str, schema: dict, timeout_ms: 'int | None' = None) -> Any
```

Decorator — register a method as an AI-callable tool and expose it.

Tools are functions that AI agents can invoke by name. Each tool has a description
and a JSON schema defining its parameters.

Args:
    name: Tool identifier; used by agents to invoke this tool
    description: Human-readable description of what the tool does
    schema: JSON schema object for tool parameters (type: "object" with properties)
    timeout_ms: Optional timeout in milliseconds; None = no timeout

Returns:
    A decorator function that registers the method as a tool.

Example:

    @app.tool("increment", description="Increment counter", schema={
        "type": "object",
        "properties": {"n": {"type": "integer", "description": "Amount to increment"}},
        "required": ["n"],
    })
    async def handle_increment(self, args):
        n = args.get("n", 0)
        self.counter += n
        return {"new_value": self.counter}

The decorated method receives a dict of parsed arguments and can return any JSON-serializable
value or raise an exception (which becomes the tool's error response).

---

### `on_suspend`

```python
def on_suspend() -> None
```

---

### `on_resume`

```python
def on_resume() -> None
```

---

### `on_shutdown`

```python
def on_shutdown() -> None
```

---

### `run`

```python
def run() -> None
```

Start the PGAP v3 asyncio event loop. Blocks until Shutdown.

This is the entry point for all Plexi apps. Call it from your main block:

    if __name__ == '__main__':
        app = MyApp()
        app.run()

---

