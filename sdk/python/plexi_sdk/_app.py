from __future__ import annotations

import asyncio
import inspect
import json
import sys
from typing import Any, Coroutine

from ._protocol import PROTOCOL_VERSION
from ._constants import _SDK_VERSION, BG as _DEFAULT_BG
from ._emitter import Emitter, _emit, _sync_hook_scope
from ._pipe import Pipe
from ._render_context import RenderContext


# ── Arg descriptor ────────────────────────────────────────────────────────────

_MISSING: Any = object()


class Arg:
    """Declare a typed launch argument on an App subclass.

    The SDK parses sys.argv before on_init runs and sets the resolved value as
    an instance attribute with the same name as the class-level declaration.

    Usage::

        class MyApp(App):
            repo_dir: Arg[str | None] = Arg("--repo-dir", default=lambda ctx: ctx.workspace_root)
            limit: Arg[int] = Arg("--limit", type=int, default=100)
            count: Arg[int] = Arg(positional=True, type=int, default=10)

            async def on_init(self, ctx):
                print(self.repo_dir)  # already resolved
    """

    def __init__(
        self,
        *flags: str,
        positional: bool = False,
        type: Any = None,
        default: Any = _MISSING,
        dest: "str | None" = None,
        nargs: Any = None,
    ) -> None:
        self.flags = flags
        self.positional = positional
        self.arg_type = type
        self.default = default
        self.dest = dest
        self.nargs = nargs

    def __class_getitem__(cls, _item: Any) -> "type[Arg]":
        return cls


def _log_task_exception(task: asyncio.Task) -> None:
    """Done callback for background tasks — logs unhandled exceptions."""
    try:
        exc = task.exception()
    except (asyncio.CancelledError, asyncio.InvalidStateError):
        return
    if exc is not None:
        sys.stderr.write(f"plexi_sdk: unhandled exception in background task: {exc}\n")


# Map egui's Debug-format key names to the documented canonical SDK names.
# The host sends "ArrowLeft" etc. (egui Key::ArrowLeft Debug repr); apps
# should use "left"/"right"/"up"/"down" as documented. Normalizing here
# means both forms work correctly — agents and humans can use the documented
# names without knowing egui's internal representation.
_KEY_ALIASES: "dict[str, str]" = {
    "ArrowLeft": "left",
    "ArrowRight": "right",
    "ArrowUp": "up",
    "ArrowDown": "down",
    # egui Debug-format names → SDK canonical names
    "Enter": "return",
    "Escape": "escape",
    "Backspace": "backspace",
    "Tab": "tab",
    # Space arrives as Event::Text(" "), not Event::Key
    " ": "space",
}


def _normalize_key(key: str) -> str:
    return _KEY_ALIASES.get(key, key)


# ── App base class ────────────────────────────────────────────────────────────

class App:
    """
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
        on_pane_spawned(pane_id, request_id)         — pane spawn succeeded
        on_pane_spawn_error(reason, request_id)      — pane spawn failed
        on_context_state(state)                      — context state query result
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
    """

    # Populated by __init_subclass__ when Arg fields are declared on a subclass.
    _arg_specs: "list[tuple[str, Arg]]" = []

    # Background color applied automatically before each on_render call.
    # Set to None to disable the default background and manage clearing manually.
    default_background: "str | None" = _DEFAULT_BG

    def __init__(self) -> None:
        self._sdk_initialized: bool = True
        self.app_id: str = ""
        self.workspace_root: str = ""
        self.capabilities: list[str] = []
        self.feature_flags: list[str] = []
        self._rect: dict = {"x": 0.0, "y": 0.0, "w": 800.0, "h": 600.0}
        self._compact_threshold: float = 280.0
        self._regular_threshold: float = 480.0
        # The running asyncio event loop. Set by run() before hooks are called.
        # Background threads use this via emit.run_sync() to schedule coroutines.
        self._loop: "asyncio.AbstractEventLoop | None" = None
        # All pending-response maps now hold asyncio.Queue so the event loop
        # coroutine can await them without blocking the stdin reader.
        self._pending_capability: "dict[str, asyncio.Queue]" = {}
        self._pending_secret: "dict[str, asyncio.Queue]" = {}
        self._app_state: dict = {}
        self._pending_http: "dict[str, asyncio.Queue]" = {}
        # v3.x async image loading (#1354): awaits PlexiEvent::ImageLoaded keyed
        # on handle UUID. Each entry is consumed by a single load_image() call.
        self._pending_image: "dict[str, asyncio.Queue]" = {}
        # v3.3 ai.query broker (#284): awaits PlexiEvent::AiResponse keyed
        # on request_id. Each entry is consumed by a single ai_query() call.
        self._pending_ai: "dict[str, asyncio.Queue]" = {}
        # v3.4 CoreMIDI (#320): awaits PlexiEvent::MidiDevicesListed keyed
        # on request_id. Each entry is consumed by a single list_midi_devices().
        self._pending_midi_devices: "dict[str, asyncio.Queue]" = {}
        # v3.4 audio device enumeration (#341): awaits PlexiEvent::AudioDevicesListed keyed
        # on request_id. Each entry is consumed by a single list_audio_devices() call.
        self._pending_audio_devices: "dict[str, asyncio.Queue]" = {}
        # v3.4 video substrate (#345): awaits PlexiEvent::VideoOpenAck /
        # VideoOpenError keyed on request_id. Each entry is consumed by a
        # single open_video() call.
        self._pending_video_open: "dict[str, asyncio.Queue]" = {}
        self._pending_notify: "dict[str, asyncio.Queue]" = {}
        # #310: non-blocking notify_*_async callbacks. Keyed on notify_id;
        # each callable is invoked on the event thread when NotifyAction arrives.
        self._pending_notify_callbacks: "dict[str, Any]" = {}
        # v3.5 Canvas Terminal Binding Primitives (#78). Two response shapes:
        # `linked_terminal_ready` carries an int pane_id; `command_preview`
        # carries (command, would_run_in_cwd). Each async helper awaits
        # its own keyed queue.
        self._pending_linked_terminal: "dict[str, asyncio.Queue]" = {}
        self._pending_command_preview: "dict[str, asyncio.Queue]" = {}
        # RenderContext.measure_text: awaits PlexiEvent::TextMeasured keyed on request_id.
        self._pending_measure_text: "dict[str, asyncio.Queue]" = {}
        # RenderContext.measure_text_wrapped: awaits PlexiEvent::TextWrappedMeasured.
        self._pending_measure_text_wrapped: "dict[str, asyncio.Queue]" = {}
        self._pipes: dict[str, Pipe] = {}
        self._last_render_time: "float | None" = None
        self._consecutive_render_errors: int = 0
        # Strong references to background asyncio tasks created by
        # _dispatch_hook_task. Without this, CPython may GC a task before it
        # completes. The done callback removes each task from the set.
        self._background_tasks: "set[asyncio.Task]" = set()
        # Pending text-input submissions keyed on TextInput `id`. The
        # event-loop coroutine fills this when `PlexiEvent::TextSubmitted`
        # arrives; `RenderContext.text_input` drains it during render.
        # One pending value per id — a second submit before the app
        # consumes the first overwrites (apps poll every frame, so
        # this only matters in a perverse scheduling case).
        self._text_submissions: dict[str, str] = {}
        # v3.7 tool protocol (#399): tool_name → handler callable.
        # Registered via @app.tool(...) decorator.
        self._tool_handlers: dict[str, Any] = {}
        # Keep the full declared tool set so repeated @app.tool decorator
        # calls expose the cumulative list instead of replacing prior tools.
        self._tool_defs: dict[str, dict] = {}
        self.emit = Emitter(self)
        # v3.x button primitive (#255): last known mouse position and buffered
        # click events for ctx.button() hit-testing during on_render.
        self._mx: float = 0.0
        self._my: float = 0.0
        self._click_buf: list[tuple[float, float]] = []
        # Hold timer for ctx.button() active_fill (#1083): maps button id →
        # monotonic timestamp at which the active state expires.
        self._btn_active_until: dict[str, float] = {}

    def __init_subclass__(cls, **kwargs: object) -> None:
        super().__init_subclass__(**kwargs)

        # Collect Arg descriptors declared directly on this class
        own_specs: "list[tuple[str, Arg]]" = [
            (name, value)
            for name, value in cls.__dict__.items()
            if isinstance(value, Arg)
        ]
        # Merge with parent's specs (own overrides parent entries with the same name)
        parent_specs: "list[tuple[str, Arg]]" = list(getattr(cls, "_arg_specs", []))
        if own_specs:
            own_names = {n for n, _ in own_specs}
            cls._arg_specs = [(n, s) for n, s in parent_specs if n not in own_names] + own_specs
        else:
            cls._arg_specs = parent_specs

        orig_init = cls.__dict__.get("__init__")
        if orig_init is not None:
            def wrapped(self_inner: "App", *args: Any, _orig: Any = orig_init, **kw: Any) -> None:
                if not getattr(self_inner, "_sdk_initialized", False):
                    App.__init__(self_inner)
                _orig(self_inner, *args, **kw)
            cls.__init__ = wrapped  # type: ignore[assignment]

    # ── Override these ──────────────────────────────────────────────────────
    # All hooks may be overridden as either `def` (sync) or `async def`.
    # _dispatch_hook detects the type at call time — both are valid.
    # Return type is `Coroutine[Any, Any, None] | None` so Pyright accepts
    # both sync (`def` → returns None) and async (`async def` → returns
    # Coroutine) overrides without reportIncompatibleMethodOverride.
    def on_init(self, _ctx: RenderContext) -> "Coroutine[Any, Any, None] | None": return None
    def on_render(self, _ctx: RenderContext) -> None: pass
    def on_key(self, _ctx: RenderContext, _key: str, _mods: dict) -> "Coroutine[Any, Any, None] | None": return None
    def on_click(self, _ctx: RenderContext, _x: float, _y: float, _button: str) -> "Coroutine[Any, Any, None] | None": return None
    def on_mouse_down(self, _ctx: RenderContext, _x: float, _y: float, _button: str) -> "Coroutine[Any, Any, None] | None": return None
    def on_mouse_up(self, _ctx: RenderContext, _x: float, _y: float, _button: str) -> "Coroutine[Any, Any, None] | None": return None
    def on_mouse_move(self, _ctx: RenderContext, _x: float, _y: float, _buttons: list) -> "Coroutine[Any, Any, None] | None": return None
    def on_command(self, _ctx: RenderContext, _text: str) -> "Coroutine[Any, Any, None] | None": return None
    def on_paste(self, _ctx: RenderContext, _text: str) -> "Coroutine[Any, Any, None] | None": return None
    def on_pipe_message(self, _ctx: RenderContext, _pipe_id: str, _payload: Any) -> "Coroutine[Any, Any, None] | None": return None
    def on_path_changed(self, _ctx: RenderContext, _cwd: str) -> "Coroutine[Any, Any, None] | None": return None
    def on_inject(self, _ctx: RenderContext, _payload: Any) -> "Coroutine[Any, Any, None] | None": return None
    def on_nav_back(self, _ctx: RenderContext, _view_id: str) -> "Coroutine[Any, Any, None] | None":
        """Called when the host emits ``NavBack`` — user pressed Cmd+[ or the
        back arrow in the pane chrome. ``view_id`` is the view being navigated
        *back to* (the new top of stack, or empty string for root).

        The app should update its own view state to show ``view_id``, then call
        ``ctx.emit.pop_nav()`` to remove the entry from the host stack.
        """
        return None
    def on_app_spawned(self, _pane_id: int, _type_id: str) -> None: pass
    def on_pane_spawned(self, _pane_id: int, _request_id: "str | None" = None) -> None:
        """Called when a SpawnPane request succeeded (#592). Override to track the spawned pane."""

    def on_pane_spawn_error(self, _reason: str, _request_id: "str | None" = None) -> None:
        """Called when a SpawnPane request failed (#592). Override to handle the error."""

    def on_context_state(self, _state: dict) -> None:
        """Called when a QueryContextState response arrives (#1518).

        ``_state`` is a dict with keys: context_id, name, path, status,
        pane_count, panes (list of pane summaries), children (list of child
        context ids).
        """

    def on_timer(self, _ctx: RenderContext, _timer_id: str) -> "Coroutine[Any, Any, None] | None": return None
    def on_scroll(self, _ctx: RenderContext, _id: str, _offset_y: float) -> "Coroutine[Any, Any, None] | None": return None
    def on_list_select(self, _ctx: "RenderContext", _id: str, _index: int) -> "Coroutine[Any, Any, None] | None":
        """Called when a list_view selection changes via j/k/up/down."""
        return None
    def on_list_activate(self, _ctx: "RenderContext", _id: str, _index: int) -> "Coroutine[Any, Any, None] | None":
        """Called when Enter is pressed on a selected list_view item."""
        return None
    def on_text_submitted(self, _ctx: RenderContext, _id: str, _text: str) -> "Coroutine[Any, Any, None] | None": return None
    def on_file_picked(self, _ctx: RenderContext, _request_id: str, _paths: "list[str]") -> "Coroutine[Any, Any, None] | None":
        """Called when the user selected one or more files in the picker.

        ``_request_id`` matches the id passed to ``ctx.emit.open_file_picker``.
        ``_paths`` is a list of absolute file paths chosen by the user.
        """
        return None
    def on_file_pick_cancelled(self, _ctx: RenderContext, _request_id: str) -> "Coroutine[Any, Any, None] | None":
        """Called when the user dismissed the file picker without selecting a file,
        or if the ``fs.pick`` capability was not declared.
        """
        return None
    """Called when the host updates the scroll offset for a BeginScroll region.

    `id` matches the id passed to `ctx.begin_scroll`. `offset_y` is the new
    vertical offset in logical pixels. Override to re-render content at the
    new position.
    """
    def on_mcp_call(self, _ctx: RenderContext, _tool_name: str, _arguments: dict) -> "dict | Coroutine[Any, Any, dict] | None":
        """Called when an external MCP client calls a tool declared in [app.mcp].

        Override to handle the call and return a result dict. The result should
        follow MCP CallToolResult schema: ``{"content": [{"type": "text", "text": "..."}]}``.
        Return ``None`` to respond with a generic 'not implemented' error.
        """
        return None

    def on_midi_input_opened(
        self,
        _pipe_id: str,
        _port_id: str,
        _port_name: str,
    ) -> None:
        """Override to react to a successful OpenMidiInput. Apps that just
        want the byte stream typically read directly from the binary pipe
        opened alongside this event — Plexi sends `pipe_opened` first."""
        pass

    # ── Tool protocol (#398, #399) ──────────────────────────────────────────

    def tool(self, name: str, description: str, schema: dict,
             timeout_ms: "int | None" = None) -> Any:
        """Decorator — register a method as an AI-callable tool and expose it.

        Tools are functions that AI agents can invoke by name. Each tool has a description
        and a JSON schema defining its parameters.

        Args:
            name: Tool identifier; used by agents to invoke this tool
            description: Human-readable description of what the tool does
            schema: JSON schema object for tool parameters (type: "object" with properties)
            timeout_ms: Optional timeout in milliseconds; None = no timeout

        Returns:
            A decorator function that registers the method as a tool.

        Example::

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
        """
        def decorator(fn: Any) -> Any:
            self._tool_handlers[name] = fn
            tool_def: dict = {
                "name": name,
                "description": description,
                "input_schema": schema,
            }
            if timeout_ms is not None:
                tool_def["timeout_ms"] = timeout_ms
            self._tool_defs[name] = tool_def
            self.emit.expose_tools(list(self._tool_defs.values()))
            return fn
        return decorator

    def on_suspend(self) -> None: pass
    def on_resume(self) -> None: pass
    def on_shutdown(self) -> None: pass

    # ── Internal ────────────────────────────────────────────────────────────

    async def _handle_tool_call(self, ev: dict) -> None:
        """Dispatch a ``PlexiEvent::ToolCall`` to the registered handler.

        Sends ``DrawCommand::ToolResult`` with the return value (JSON-serialised)
        or with an error string if the handler raises or is not registered.
        """
        call_id: str = ev.get("call_id", "")
        name: str = ev.get("name", "")
        input_json: str = ev.get("input_json", "{}")

        try:
            args = json.loads(input_json) if input_json else {}
        except json.JSONDecodeError as exc:
            _emit({
                "type": "tool_result",
                "call_id": call_id,
                "output_json": None,
                "error": f"tool_input_parse_error: {exc}",
            })
            return

        handler = self._tool_handlers.get(name)
        if handler is None:
            _emit({
                "type": "tool_result",
                "call_id": call_id,
                "output_json": None,
                "error": f"tool_not_found: no handler registered for tool {name!r}",
            })
            return

        try:
            import inspect as _inspect
            if _inspect.iscoroutinefunction(handler):
                result = await handler(args)
            else:
                result = handler(args)
            output_json = json.dumps(result) if result is not None else json.dumps({})
            _emit({
                "type": "tool_result",
                "call_id": call_id,
                "output_json": output_json,
                "error": None,
            })
        except Exception as exc:
            import traceback as _tb
            _tb.print_exc()
            _emit({
                "type": "tool_result",
                "call_id": call_id,
                "output_json": None,
                "error": f"tool_handler_error: {exc}",
            })

    async def _handle_mcp_tool_call(self, ev: dict) -> None:
        """Dispatch a PlexiEvent::McpToolCall to on_mcp_call."""
        call_id: str = ev.get("call_id", "")
        tool_name: str = ev.get("tool_name", "")
        arguments: dict = ev.get("arguments", {})

        try:
            import inspect as _inspect
            if _inspect.iscoroutinefunction(self.on_mcp_call):
                result = await self.on_mcp_call(self._make_ctx(), tool_name, arguments)
            else:
                result = self.on_mcp_call(self._make_ctx(), tool_name, arguments)

            if result is None:
                _emit({
                    "type": "mcp_tool_result",
                    "call_id": call_id,
                    "result": None,
                    "error": f"tool_not_implemented: {tool_name!r}",
                })
            else:
                _emit({
                    "type": "mcp_tool_result",
                    "call_id": call_id,
                    "result": result,
                    "error": None,
                })
        except Exception as exc:
            import traceback as _tb
            _tb.print_exc()
            _emit({
                "type": "mcp_tool_result",
                "call_id": call_id,
                "result": None,
                "error": f"mcp_tool_handler_error: {exc}",
            })

    def _take_text_submission(self, id: str) -> "str | None":
        """Pop the most recent submission for `id` if one is queued, else None.

        Called by `RenderContext.text_input` to surface a buffered
        `TextSubmitted` value into the current frame's render call.
        """
        return self._text_submissions.pop(id, None)

    def _make_ctx(self, frame_id: int = 0, elapsed: float = 0.0,
                  clicks: "list[tuple[float, float]] | None" = None) -> RenderContext:
        ctx = RenderContext(
            frame_id=frame_id,
            rect=self._rect,
            workspace_root=self.workspace_root,
            capabilities=self.capabilities,
            feature_flags=self.feature_flags,
            app=self,
            elapsed=elapsed,
            clicks=clicks or [],
        )
        ctx._compact_threshold = self._compact_threshold
        ctx._regular_threshold = self._regular_threshold
        return ctx

    def _parse_launch_args(self, ctx: RenderContext) -> None:
        """Parse sys.argv against declared Arg specs and set resolved instance attributes.

        Called automatically before on_init when the class declares Arg fields.
        Lambda defaults receive ctx and are resolved here.
        """
        import argparse as _argparse
        specs: "list[tuple[str, Arg]]" = type(self)._arg_specs
        if not specs:
            return

        parser = _argparse.ArgumentParser(add_help=False)
        lambda_defaults: "dict[str, Any]" = {}

        for attr_name, arg_spec in specs:
            dest = arg_spec.dest or attr_name
            is_lambda = callable(arg_spec.default) and not isinstance(arg_spec.default, type)
            if is_lambda:
                raw_default: Any = None
                lambda_defaults[attr_name] = arg_spec.default
            elif arg_spec.default is _MISSING:
                raw_default = None
            else:
                raw_default = arg_spec.default

            if arg_spec.positional:
                pkw: "dict[str, Any]" = {}
                if arg_spec.nargs is not None:
                    pkw["nargs"] = arg_spec.nargs
                    pkw["default"] = raw_default if raw_default is not None else []
                elif arg_spec.default is not _MISSING:
                    # Explicit default → optional positional
                    pkw["nargs"] = "?"
                    pkw["default"] = raw_default
                # else: no nargs, no default → required positional (argparse enforces)
                if arg_spec.arg_type is not None and arg_spec.arg_type is not bool:
                    pkw["type"] = arg_spec.arg_type
                parser.add_argument(dest, **pkw)
            else:
                if not arg_spec.flags:
                    continue
                okw: "dict[str, Any]" = {"dest": dest, "default": raw_default}
                if arg_spec.arg_type is bool:
                    okw["action"] = "store_true"
                elif arg_spec.arg_type is not None:
                    okw["type"] = arg_spec.arg_type
                if arg_spec.nargs is not None:
                    okw["nargs"] = arg_spec.nargs
                parser.add_argument(*arg_spec.flags, **okw)

        argv = [a for a in sys.argv[1:] if a != "--plexi-introspect"]
        try:
            ns, _ = parser.parse_known_args(argv)
        except SystemExit as e:
            sys.exit(e.code)

        for attr_name, arg_spec in specs:
            dest = arg_spec.dest or attr_name
            value = getattr(ns, dest, None)

            if value is None and arg_spec.default is _MISSING and not arg_spec.positional:
                flag = arg_spec.flags[0] if arg_spec.flags else attr_name
                sys.stderr.write(f"{flag}: required argument missing\n")
                sys.exit(1)

            if value is None and attr_name in lambda_defaults:
                value = lambda_defaults[attr_name](ctx)

            setattr(self, attr_name, value)
        self.emit.info(
            f"args: parsed {len(specs)} arg(s): "
            + ", ".join(f"{n}={getattr(self, n)!r}" for n, _ in specs)
        )

    def run(self) -> None:
        """Start the PGAP v3 asyncio event loop. Blocks until Shutdown.

        This is the entry point for all Plexi apps. Call it from your main block:

            if __name__ == '__main__':
                app = MyApp()
                app.run()
        """
        if "--plexi-introspect" in sys.argv:
            self._run_introspect()
            return
        sys.stdout.reconfigure(line_buffering=True)  # type: ignore[union-attr]
        asyncio.run(self._async_main())

    def _run_introspect(self) -> None:
        """Static capability check mode — called when launched with --plexi-introspect.

        Inspects method bodies of this App subclass for emit.* / ctx.* calls
        using AST analysis (not regex) to avoid false positives in docstrings.
        Only scans methods defined in the subclass's own module (not base class).
        Prints {"required_capabilities": [...]} to stdout, then exits.
        """
        import ast
        import inspect
        import json
        import textwrap
        from ._emitter import CAPABILITY_REGISTRY

        required: set[str] = set()
        app_module = type(self).__module__

        for _name, method in inspect.getmembers(type(self), predicate=inspect.isfunction):
            if getattr(method, "__module__", None) != app_module:
                continue
            try:
                source = textwrap.dedent(inspect.getsource(method))
                tree = ast.parse(source)
            except (OSError, TypeError, SyntaxError, IndentationError):
                continue
            for node in ast.walk(tree):
                if not (isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute)):
                    continue
                func = node.func
                base: "str | None" = None
                if isinstance(func.value, ast.Name):
                    base = func.value.id
                elif (
                    isinstance(func.value, ast.Attribute)
                    and isinstance(func.value.value, ast.Name)
                    and func.value.value.id == "self"
                ):
                    base = f"self.{func.value.attr}"
                if base in ("self.emit", "self.ctx", "ctx", "emit"):
                    method_name = func.attr
                    if method_name in CAPABILITY_REGISTRY:
                        required.add(CAPABILITY_REGISTRY[method_name])

        print(json.dumps({"required_capabilities": sorted(required)}), flush=True)
        sys.exit(0)

    async def _async_main(self) -> None:
        """Asyncio entry point — two concurrent tasks to eliminate deadlocks.

        The root cause of the old single-loop design: when the dispatcher
        awaited a hook (e.g. on_init) that itself called a blocking helper
        (e.g. request_linked_terminal), the event loop had no concurrent
        stdin reader in flight. Nothing could deliver the response event
        while the hook was suspended, causing a permanent deadlock.

        Fix: split into two tasks that run concurrently on the same event loop.

          _reader  — always has a run_in_executor(readline) in flight.
                     Handles response events inline (put_nowait into pending
                     queues) so they can unblock awaiting hooks even while the
                     dispatcher is suspended.

          _dispatcher — drains a hook_q, dispatches hook events sequentially.
                        Can safely await hooks because _reader is always running
                        alongside it and will deliver response events.

        Response events MUST be handled inline in _reader — never enqueued —
        so that hooks awaiting on pending queues can be unblocked even when
        the dispatcher is suspended mid-hook.
        """
        loop = asyncio.get_running_loop()
        self._loop = loop
        hook_q: asyncio.Queue = asyncio.Queue()

        async def _reader() -> None:
            while True:
                raw = await loop.run_in_executor(None, sys.stdin.readline)
                if not raw:
                    # EOF — host closed stdin; signal dispatcher to shut down.
                    await hook_q.put({"type": "shutdown"})
                    return
                raw = raw.strip()
                if not raw:
                    continue
                try:
                    ev = json.loads(raw)
                except json.JSONDecodeError:
                    continue

                t = ev.get("type", "")

                # ── Response events: handled inline so they can unblock ──────
                # hooks suspended in the dispatcher. These must NEVER go on
                # hook_q — that would leave awaiting coroutines stuck forever.

                if t == "capability_decision":
                    req_id = ev.get("request_id", "")
                    granted = ev.get("granted", False)
                    q = self._pending_capability.pop(req_id, None)
                    if q:
                        q.put_nowait(granted)

                elif t == "secret_value":
                    key = ev.get("key", "")
                    value = ev.get("value")
                    q = self._pending_secret.pop(key, None)
                    if q:
                        q.put_nowait(value)

                elif t == "http_response":
                    req_id = ev.get("request_id", "")
                    q = self._pending_http.pop(req_id, None)
                    if q:
                        if ev.get("error"):
                            q.put_nowait(("error", ev["error"]))
                        else:
                            q.put_nowait(("ok", ev.get("body", "")))

                elif t == "image_loaded":
                    handle = ev.get("handle", "")
                    q = self._pending_image.pop(handle, None)
                    if q:
                        status = ev.get("status", "error")
                        message = ev.get("message")
                        q.put_nowait((status, message))

                elif t == "ai_response":
                    # v3.3 ai.query broker (#284). Hand the whole event dict to
                    # `Emitter.ai_query` so it can split error vs success and
                    # attach token counts.
                    req_id = ev.get("request_id", "")
                    q = self._pending_ai.pop(req_id, None)
                    if q:
                        q.put_nowait(ev)
                    else:
                        import logging as _logging
                        _logging.warning(
                            f"ai_response: no pending request for req_id={req_id!r} — "
                            "response dropped (query may have timed out already)"
                        )

                elif t == "midi_devices_listed":
                    # v3.4 CoreMIDI (#320). Forward to Emitter.list_midi_devices.
                    req_id = ev.get("request_id", "")
                    q = self._pending_midi_devices.pop(req_id, None)
                    if q:
                        q.put_nowait(ev)

                elif t == "audio_devices_listed":
                    # v3.4 audio device enumeration (#341). Forward to Emitter.list_audio_devices.
                    req_id = ev.get("request_id", "")
                    q = self._pending_audio_devices.pop(req_id, None)
                    if q:
                        q.put_nowait(ev)

                elif t == "video_open_ack":
                    # v3.4 video substrate (#345). Forward to Emitter.open_video().
                    req_id = str(ev.get("request_id", ""))
                    q = self._pending_video_open.pop(req_id, None)
                    if q:
                        q.put_nowait(ev)

                elif t == "video_open_error":
                    # OpenVideo failed (capability denied, NotImplemented from the
                    # production stub, bad source). Forward the error event so
                    # `open_video()` can raise CapabilityDeniedError / RuntimeError.
                    req_id = str(ev.get("request_id", ""))
                    q = self._pending_video_open.pop(req_id, None)
                    if q:
                        q.put_nowait(ev)

                elif t == "linked_terminal_ready":
                    # v3.5 #78. Forward the terminal_pane_id (int) to the
                    # awaiting helper. 0 = capability denied — the helper
                    # raises CapabilityDeniedError when it sees that.
                    req_id = ev.get("request_id", "")
                    q = self._pending_linked_terminal.pop(req_id, None)
                    if q:
                        q.put_nowait(int(ev.get("terminal_pane_id", 0)))

                elif t == "command_preview":
                    # v3.5 #78. Forward (command, would_run_in_cwd) tuple to the
                    # awaiting helper. would_run_in_cwd is "" on capability denial.
                    req_id = ev.get("request_id", "")
                    q = self._pending_command_preview.pop(req_id, None)
                    if q:
                        q.put_nowait((
                            str(ev.get("command", "")),
                            str(ev.get("would_run_in_cwd", "")),
                        ))

                elif t == "notify_action":
                    # notify_choice / notify_input: put the value back.
                    # notify / notify_and_wait: put action_label back.
                    # Esc cancel: return "__cancel__" so callers can check easily.
                    notify_id = ev.get("notify_id", "")
                    action_label = ev.get("action_label", "")
                    value = ev.get("value")
                    if action_label == "cancel":
                        result = "__cancel__"
                    elif value is not None:
                        result = value
                    else:
                        result = action_label or "acknowledge"
                    q = self._pending_notify.pop(notify_id, None)
                    if q:
                        q.put_nowait(result)
                    else:
                        cb = self._pending_notify_callbacks.pop(notify_id, None)
                        if cb is not None:
                            self.emit.info(f"notify_async: dispatching callback for {notify_id!r}")
                            try:
                                cb(result)
                            except Exception as exc:
                                sys.stderr.write(f"plexi_sdk: notify_async callback raised: {exc}\n")

                elif t == "text_measured":
                    # Response to RenderContext.measure_text(). Forward (width, height)
                    # to the awaiting coroutine keyed on request_id.
                    req_id = ev.get("request_id", "")
                    q = self._pending_measure_text.pop(req_id, None)
                    if q:
                        q.put_nowait((
                            float(ev.get("width", 0.0)),
                            float(ev.get("height", 0.0)),
                        ))

                elif t == "text_wrapped_measured":
                    req_id = ev.get("request_id", "")
                    q = self._pending_measure_text_wrapped.pop(req_id, None)
                    if q:
                        q.put_nowait(float(ev.get("height", 0.0)))

                # ── Inline non-hook events (fast, no user code) ──────────────

                elif t == "pipe_opened":
                    pipe_id = ev.get("pipe_id", "")
                    socket_path = ev.get("socket_path", "")
                    p = self._pipes.get(pipe_id)
                    if p:
                        p._on_opened(socket_path)

                elif t == "pipe_overrun":
                    self.emit.warn(
                        f"pipe overrun pipe_id={ev.get('pipe_id')} "
                        f"dropped={ev.get('dropped_frames')}"
                    )

                elif t == "midi_input_error":
                    # OpenMidiInput failed (capability denied, port_id not found,
                    # CoreMIDI error). Apps log this; the typical recovery is to
                    # surface the error in-pane and let the user pick a different
                    # port from list_midi_devices.
                    self.emit.warn(
                        f"midi_input_error pipe_id={ev.get('pipe_id')} "
                        f"error={ev.get('error')}"
                    )

                elif t == "midi_send_error":
                    # SendMidi failed. Surfaces only on capability denial / open
                    # failure / coremidi error — successful sends produce no event.
                    self.emit.warn(
                        f"midi_send_error port_id={ev.get('port_id')} "
                        f"error={ev.get('error')}"
                    )

                elif t == "text_submitted":
                    # Host-owned text input: the user pressed Enter on a
                    # `DrawCommand::TextInput` field. Stash the value keyed
                    # on the input id; `RenderContext.text_input(...)` will
                    # drain it on the next frame the app polls.
                    # Also dispatch on_text_submitted as a hook task if the
                    # app has overridden it — apps that use the event-handler
                    # path can do I/O cleanly without side effects in on_render.
                    tid = ev.get("id", "")
                    if tid:
                        value = ev.get("value", "")
                        self._text_submissions[tid] = value
                        if type(self).on_text_submitted is not App.on_text_submitted:
                            self._dispatch_hook_task(
                                self.on_text_submitted, self._make_ctx(), tid, value
                            )

                elif t == "run_update":
                    pass  # apps can override on_run_update if needed

                # ── Hook events: forwarded to the dispatcher ─────────────────
                else:
                    await hook_q.put(ev)

        async def _dispatcher() -> None:
            while True:
                ev = await hook_q.get()
                t = ev.get("type", "")

                if t == "init":
                    proto = ev.get("protocol", "")
                    if not proto.startswith(PROTOCOL_VERSION):
                        sys.stderr.write(
                            f"plexi_sdk: unsupported protocol {proto!r}, expected {PROTOCOL_VERSION}\n"
                        )
                        sys.exit(1)
                    self.app_id = ev.get("app_id", "")
                    self.workspace_root = ev.get("workspace_root", "")
                    self.capabilities = ev.get("capabilities", [])
                    self.feature_flags = ev.get("feature_flags", [])
                    self._compact_threshold = ev.get("compact_threshold", 280.0)
                    self._regular_threshold = ev.get("regular_threshold", 480.0)
                    # Send Ready
                    features_used = [f for f in self.feature_flags
                                      if f in ("pane_groups_v1",)]
                    _emit({"type": "ready", "sdk": SDK_ID, "features_used": features_used})
                    self.emit.info(f"sdk: default_background={self.default_background!r}")
                    if getattr(type(self), "_arg_specs", None):
                        self._parse_launch_args(self._make_ctx())
                    await self._dispatch_hook(self.on_init, self._make_ctx())

                elif t == "render":
                    import time as _time
                    now = _time.monotonic()
                    elapsed = (now - self._last_render_time) if self._last_render_time is not None else 0.0
                    self._last_render_time = now
                    frame_id = ev.get("frame_id", 0)
                    if "rect" in ev:
                        self._rect = ev["rect"]
                    elif "width" in ev:
                        # legacy compat
                        self._rect = {"x": 0.0, "y": 0.0,
                                      "w": ev["width"], "h": ev["height"]}
                    pending_clicks = list(self._click_buf)
                    self._click_buf.clear()
                    ctx = self._make_ctx(frame_id, elapsed=elapsed, clicks=pending_clicks)
                    if self.default_background is not None:
                        ctx.clear(self.default_background)
                    try:
                        await self._dispatch_hook(self.on_render, ctx)
                        self._consecutive_render_errors = 0
                    except Exception as e:
                        self._consecutive_render_errors += 1
                        ctx.error(f"on_render exception: {e}")
                        if self._consecutive_render_errors >= 3:
                            import traceback as _tb
                            _tb.print_exc()
                            raise
                    ctx.frame_done()

                elif t == "key":
                    ctx = self._make_ctx()
                    self._dispatch_hook_task(self.on_key, ctx, _normalize_key(ev.get("key", "")), ev.get("modifiers", {}))

                elif t == "click":
                    self._click_buf.append((ev.get("x", 0.0), ev.get("y", 0.0)))
                    ctx = self._make_ctx()
                    self._dispatch_hook_task(self.on_click, ctx, ev.get("x", 0.0), ev.get("y", 0.0),
                                             ev.get("button", "primary"))

                elif t == "mouse_down":
                    ctx = self._make_ctx()
                    await self._dispatch_hook(self.on_mouse_down, ctx, ev.get("x", 0.0), ev.get("y", 0.0),
                                              ev.get("button", "primary"))

                elif t == "mouse_up":
                    ctx = self._make_ctx()
                    await self._dispatch_hook(self.on_mouse_up, ctx, ev.get("x", 0.0), ev.get("y", 0.0),
                                              ev.get("button", "primary"))

                elif t == "mouse_move":
                    self._mx = ev.get("x", 0.0)
                    self._my = ev.get("y", 0.0)
                    ctx = self._make_ctx()
                    await self._dispatch_hook(self.on_mouse_move, ctx, ev.get("x", 0.0), ev.get("y", 0.0),
                                              ev.get("buttons", []))

                elif t == "command":
                    ctx = self._make_ctx()
                    self._dispatch_hook_task(self.on_command, ctx, ev.get("text", ""))

                elif t == "paste":
                    ctx = self._make_ctx()
                    self._dispatch_hook_task(self.on_paste, ctx, ev.get("text", ""))

                elif t == "pipe_message":
                    ctx = self._make_ctx()
                    self._dispatch_hook_task(self.on_pipe_message, ctx, ev.get("pipe_id", ""), ev.get("payload"))

                elif t == "path_changed":
                    ctx = self._make_ctx()
                    self._dispatch_hook_task(self.on_path_changed, ctx, ev.get("cwd", ""))

                elif t == "suspend":
                    await self._dispatch_hook(self.on_suspend)

                elif t == "resume":
                    await self._dispatch_hook(self.on_resume)

                elif t == "shutdown":
                    # Cancel any pending ai_query waiters so their coroutines
                    # unblock immediately instead of waiting up to 35s for a
                    # response that will never arrive.
                    if self._pending_ai:
                        import logging as _logging
                        _logging.warning(
                            f"shutdown: cancelling {len(self._pending_ai)} in-flight "
                            f"ai_query request(s): {list(self._pending_ai.keys())}"
                        )
                        for _pending_q in self._pending_ai.values():
                            _pending_q.put_nowait(
                                {"error": "ai_query cancelled: app is shutting down"}
                            )
                        self._pending_ai.clear()
                    await self._dispatch_hook(self.on_shutdown)
                    return

                elif t == "inject_state":
                    self._app_state = ev.get("payload") or {}
                    ctx = self._make_ctx()
                    self._dispatch_hook_task(self.on_inject, ctx, ev.get("payload", {}))

                elif t == "midi_input_opened":
                    # Confirms an OpenMidiInput call landed a CoreMIDI source.
                    # Apps that care about "the port is now wired to my pipe"
                    # see this event after the corresponding PipeOpened — they
                    # can override on_midi_input_opened to react.
                    self._dispatch_hook_task(
                        self.on_midi_input_opened,
                        str(ev.get("pipe_id", "")),
                        str(ev.get("port_id", "")),
                        str(ev.get("port_name", "")),
                    )

                elif t == "timer":
                    timer_id = ev.get("timer_id", "")
                    ctx = self._make_ctx()
                    self._dispatch_hook_task(self.on_timer, ctx, timer_id)

                elif t == "scroll_offset":
                    # Host-managed scroll region (#446): the user scrolled inside
                    # a BeginScroll viewport. Forward to on_scroll so the app can
                    # store the new offset and re-render at the translated position.
                    scroll_id = ev.get("id", "")
                    offset_y = float(ev.get("offset_y", 0.0))
                    ctx = self._make_ctx()
                    try:
                        await self._dispatch_hook(self.on_scroll, ctx, scroll_id, offset_y)
                    except Exception as e:
                        sys.stderr.write(f"on_scroll handler raised: {e}\n")

                elif t == "list_select":
                    _lid = ev.get("id")
                    _lidx = ev.get("index")
                    if _lid is None or _lidx is None:
                        sys.stderr.write(f"list_select event missing required fields: {ev}\n")
                    else:
                        ctx = self._make_ctx()
                        self._dispatch_hook_task(self.on_list_select, ctx, _lid, _lidx)

                elif t == "list_activate":
                    _lid = ev.get("id")
                    _lidx = ev.get("index")
                    if _lid is None or _lidx is None:
                        sys.stderr.write(f"list_activate event missing required fields: {ev}\n")
                    else:
                        ctx = self._make_ctx()
                        self._dispatch_hook_task(self.on_list_activate, ctx, _lid, _lidx)

                elif t == "app_spawned":
                    # Confirmation that a SpawnApp request succeeded. Apps that
                    # want to track the spawned pane can override on_app_spawned.
                    self._dispatch_hook_task(
                        self.on_app_spawned,
                        int(ev.get("pane_id", 0)),
                        str(ev.get("type_id", "")),
                    )

                elif t == "pane_spawned":
                    req_id = ev.get("request_id")
                    self._dispatch_hook_task(
                        self.on_pane_spawned,
                        int(ev.get("pane_id", 0)),
                        str(req_id) if req_id is not None else None,
                    )

                elif t == "pane_spawn_error":
                    req_id = ev.get("request_id")
                    self._dispatch_hook_task(
                        self.on_pane_spawn_error,
                        str(ev.get("reason", "")),
                        str(req_id) if req_id is not None else None,
                    )

                elif t == "context_state_response":
                    self._dispatch_hook_task(
                        self.on_context_state,
                        ev.get("state", {}),
                    )

                elif t == "nav_back":
                    # Navigation stack back event (#392). The host pops the top
                    # nav entry and sends this with the view_id the app should
                    # navigate back to (empty string = root view).
                    ctx = self._make_ctx()
                    await self._dispatch_hook(
                        self.on_nav_back, ctx, str(ev.get("view_id", ""))
                    )

                elif t == "file_picked":
                    # File picker result (#514) — user selected one or more files.
                    request_id = str(ev.get("request_id", ""))
                    paths: list[str] = list(ev.get("paths", []))
                    ctx = self._make_ctx()
                    await self._dispatch_hook(self.on_file_picked, ctx, request_id, paths)

                elif t == "file_pick_cancelled":
                    # File picker cancelled (#514) — dialog dismissed or capability denied.
                    request_id = str(ev.get("request_id", ""))
                    ctx = self._make_ctx()
                    await self._dispatch_hook(self.on_file_pick_cancelled, ctx, request_id)

                elif t == "tool_call":
                    # v3.7 tool protocol (#399). Host asks this pane to execute
                    # a registered tool. Dispatched as a background task so it
                    # doesn't block the event loop while the handler runs.
                    self._dispatch_hook_task(self._handle_tool_call, ev)

                elif t == "mcp_tool_call":
                    # MCP bridge (#958). External MCP client called a tool — dispatch to on_mcp_call.
                    self._dispatch_hook_task(self._handle_mcp_tool_call, ev)

        reader_task = asyncio.create_task(_reader())
        try:
            await _dispatcher()
        finally:
            reader_task.cancel()
            for p in self._pipes.values():
                p.close()
            # _reader is blocked in run_in_executor(sys.stdin.readline) which
            # cannot be interrupted by task cancellation — the executor thread
            # stays alive until stdin EOF, which the host may not send within
            # the 2s shutdown window. os._exit() terminates immediately without
            # waiting for threads, avoiding the SIGTERM that would otherwise fire.
            import os as _os
            _os._exit(0)

    async def _dispatch_hook(self, hook: "Any", *args: Any) -> None:
        """Dispatch a lifecycle hook and await its completion.

        Use this for hooks where ordering matters — on_render (FrameDone must
        follow all draw commands), on_init (startup must complete before the
        first render), on_shutdown (clean-up must finish before exit).

        Async hooks are awaited directly; they may call any ``await``-able
        Emitter helper without deadlock because _reader runs concurrently as
        a separate task and will deliver response events while this hook is
        suspended.

        Sync hooks run on the event loop thread. They are safe for
        pure-compute / draw-command work (on_render). A sync hook that calls
        any blocking operation (time.sleep, requests.get, etc.) will freeze
        the entire event loop — use _dispatch_hook_task for input events where
        blocking is a realistic concern, or move blocking work to a thread via
        ``threading.Thread`` + ``emit.run_sync()``.
        """
        if inspect.iscoroutinefunction(hook):
            await hook(*args)
        else:
            with _sync_hook_scope():
                hook(*args)

    def _dispatch_hook_task(self, hook: "Any", *args: Any) -> None:
        """Dispatch a lifecycle hook as a non-blocking background task.

        Use this for input-driven hooks (on_key, on_click, on_command, etc.)
        where a slow or async handler must not stall the stdin reader or delay
        the next Render event.

        Async hooks are scheduled as asyncio tasks via create_task — the
        dispatcher returns immediately and the hook runs concurrently on the
        same event loop. All ``await``-able Emitter helpers work normally.

        Sync hooks that do not block are called directly on the event loop
        thread (zero overhead, same as before). Sync hooks that *do* block
        (time.sleep, requests.get, urllib calls, etc.) are the root cause of
        the deadlock described in issue #393. The correct fix is to declare
        the handler ``async def`` and use ``await asyncio.to_thread(fn)`` or
        ``await self.emit.http_get(url)`` for any I/O, or to kick off a
        ``threading.Thread`` and use ``emit.run_sync(...)`` to bridge back.
        Sync blocking is logged as a warning so the problem is surfaced at
        runtime rather than silently freezing the app.

        Note: because tasks run concurrently, a queued on_key task may still
        be running when on_render fires. Apps with shared mutable state should
        use asyncio locks or confine mutations to on_render (the poll pattern).
        """
        if inspect.iscoroutinefunction(hook):
            task = asyncio.create_task(hook(*args))
            # Keep a strong reference so the GC doesn't collect the task before
            # it finishes. The done callback removes it from the set.
            self._background_tasks.add(task)
            task.add_done_callback(self._background_tasks.discard)
            task.add_done_callback(_log_task_exception)
        else:
            try:
                with _sync_hook_scope():
                    hook(*args)
            except Exception as e:
                sys.stderr.write(f"plexi_sdk: sync hook {getattr(hook, '__name__', hook)!r} raised: {e}\n")


# SDK_ID used in the ready handshake. Derived from the version constant so
# __init__.py and _app.py stay in sync without a circular import.
SDK_ID = f"plexi-sdk-py/{_SDK_VERSION}"
