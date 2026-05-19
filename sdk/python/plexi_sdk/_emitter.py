from __future__ import annotations

import asyncio
import functools
import json
import sys
import threading
import uuid
from contextlib import contextmanager
from typing import TYPE_CHECKING, Any, Iterator

from ._protocol import AiResponse, MidiPortInfo, MidiDeviceList, AudioDeviceInfo, AudioDeviceList, CameraDeviceInfo, CameraDeviceList
from ._types import CapabilityDeniedError, VideoHandle

if TYPE_CHECKING:
    from ._app import App
    from ._pipe import Pipe

# ── Reserved notification shortcuts ──────────────────────────────────────────
# Keys reserved by the notification overlay for navigation. The host strips
# these if an app sends them, and the SDK warns at call time.
_RESERVED_SHORTCUTS = frozenset("jkhl") | frozenset("123456789")

# ── Capability registry ───────────────────────────────────────────────────────
# Maps Emitter/RenderContext method names to the capability string they require.
# Used by --plexi-introspect mode to report what capabilities an app uses.
CAPABILITY_REGISTRY: dict[str, str] = {
    "http_get": "net.http",
    "http_request": "net.http",
    "load_image": "net.http",
    "ai_query": "ai.query",
    "secret_get": "secrets.get",
    "get_secret": "secrets.get",
    "open_file_picker": "fs.pick",
    "spawn_pane": "panes.spawn",
    "open_midi_input": "midi.in",
    "send_midi": "midi.out",
    "open_video": "video.playback",
    "set_video_state": "video.playback",
    "close_video": "video.playback",
    "list_camera_devices": "video.capture",
    "audio_capture": "audio.record",
    "stop_audio_capture": "audio.record",
    "audio_play": "audio.playback",
    "set_timer": "timer",
    "cancel_timer": "timer",
    "pipe_open": "pipe.open",
    "pipe_open_directed": "pipe.open",
    "request_linked_terminal": "terminal.bindings",
    "run_in_linked_terminal": "terminal.bindings",
    "insert_path_token": "terminal.bindings",
    "request_command_preview": "terminal.bindings",
    "open_artifact": "terminal.bindings",
}

# ── Internal helpers ──────────────────────────────────────────────────────────
_LOCK = threading.Lock()

# Per-thread sentinel: True while a *sync* hook is executing on the event loop
# thread. Blocking emit methods check this and raise immediately instead of
# silently returning an un-awaited coroutine object.
_SYNC_HOOK_LOCAL: threading.local = threading.local()


@contextmanager
def _sync_hook_scope() -> Iterator[None]:
    """Mark the current thread as executing a sync hook."""
    old_active = getattr(_SYNC_HOOK_LOCAL, 'active', False)
    _SYNC_HOOK_LOCAL.active = True
    try:
        yield
    finally:
        _SYNC_HOOK_LOCAL.active = old_active


def _blocking_emit_method(fn):
    """Decorator: raises TypeError if a blocking emit is called from a sync hook.

    Async hooks use ``await``, so the wrapper body runs, sentinel is clear,
    and the inner coroutine is returned for the caller to await normally.
    Sync hooks call without ``await`` — wrapper body runs, sentinel is set,
    TypeError fires before any I/O is attempted.
    Background threads (run_sync path) are on a different thread, so the
    thread-local sentinel is always clear for them.
    """
    name = fn.__name__

    @functools.wraps(fn)
    def wrapper(self, *args, **kwargs):
        if getattr(_SYNC_HOOK_LOCAL, 'active', False):
            raise TypeError(
                f"emit.{name}() must be called with 'await' from an 'async def' hook.\n"
                f"Your hook is 'def' (sync) — blocking emit calls require 'async def'.\n"
                f"Fix: change 'def on_init(self, ctx)' → 'async def on_init(self, ctx)'\n"
                f"     then call:  result = await self.emit.{name}(...)"
            )
        return fn(self, *args, **kwargs)

    return wrapper


def _log_background_task_error(task: "asyncio.Task[Any]") -> None:
    """Done callback for schedule_task — logs unhandled exceptions to stderr."""
    try:
        exc = task.exception()
    except (asyncio.CancelledError, asyncio.InvalidStateError):
        return
    if exc is not None:
        sys.stderr.write(f"plexi_sdk: unhandled exception in schedule_task: {exc}\n")


def _emit(obj: dict) -> None:
    """Thread-safe JSON line write to stdout."""
    with _LOCK:
        sys.stdout.write(json.dumps(obj) + "\n")
        sys.stdout.flush()


def _make_async_queue() -> asyncio.Queue:
    """Return a fresh asyncio.Queue for one pending request slot."""
    return asyncio.Queue(maxsize=1)


# ── Emitter (always available, even outside a frame) ─────────────────────────

class Emitter:
    """Emit out-of-frame commands to Plexi. Thread-safe."""

    def __init__(self, app: "App"):
        self._app = app

    # Logging
    def log(self, level: str, message: str) -> None:
        _emit({"type": "log", "level": level, "message": message})

    def info(self, message: str) -> None:  self.log("info", message)
    def warn(self, message: str) -> None:  self.log("warn", message)
    def error(self, message: str) -> None: self.log("error", message)
    def debug(self, message: str) -> None: self.log("debug", message)

    # Notifications — kind = "message" (plain text, one Acknowledge button).
    #
    # `priority` is REQUIRED on every notify call. Higher = more urgent.
    # The host uses it to pick the next front-most notification after
    # dismiss and to order the Cmd+] / Cmd+[ preview traversal. Typical
    # values: 0 (background info), 50 (normal), 100 (important), 200
    # (critical). No default — apps must pick deliberately.
    #
    # Scope (window / context / global) is NOT set by the app. It's declared
    # in manifest.toml under [launch] notification_scope and resolved by the
    # host at dispatch time. Apps just call notify(); the manifest controls
    # whether the notification crosses window or context boundaries.
    def notify(self, title: str, priority: int, body: str = "",
               level: str = "info",
               actions: "list | None" = None,
               image_inline: "dict | None" = None,
               image_pipe_id: "str | None" = None,
               timeout_secs: "int | None" = None,
               on_dismiss: "str | None" = None) -> None:
        """Post a message notification. The modal shows title + body and a
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
        """
        payload = {"type": "notify", "level": level, "title": title,
                   "body": body, "kind": "message",
                   "priority": priority,
                   "actions": actions or []}
        if image_inline is not None:
            payload["image_inline"] = image_inline
        if image_pipe_id is not None:
            payload["image_pipe_id"] = image_pipe_id
        if timeout_secs is not None:
            payload["timeout_secs"] = int(timeout_secs)
        if on_dismiss is not None:
            payload["on_dismiss"] = on_dismiss
        _emit(payload)

    def run_sync(self, coro: "Any") -> Any:
        """Run a coroutine from a background thread and return its result.

        Use this when calling async Emitter methods from a non-async context
        (e.g. inside a ``threading.Thread`` callback)::

            def _worker(self) -> None:
                result = self.emit.run_sync(self.emit.http_get(url))

        Must NOT be called from within the event loop thread — it will deadlock.
        Raises ``RuntimeError`` if there is no running event loop to dispatch to
        (e.g. called before ``App.run()`` starts).
        """
        if getattr(_SYNC_HOOK_LOCAL, 'active', False):
            raise RuntimeError(
                "emit.run_sync() called from a sync hook (e.g. on_render) — "
                "this deadlocks the event loop. Use emit.schedule_task(coro) "
                "instead when you don't need the return value, or change the "
                "hook to 'async def' and use 'await self.emit.method()'."
            )
        loop = self._app._loop
        if loop is None:
            raise RuntimeError(
                "emit.run_sync() called before the event loop started. "
                "Only call it after App.run() is running (e.g. from a "
                "background thread started inside on_init or later)."
            )
        future = asyncio.run_coroutine_threadsafe(coro, loop)
        return future.result()

    def schedule_task(self, coro: "Any") -> Any:
        """Schedule a coroutine as a background asyncio task from any context.

        Safe to call from sync hooks (on_render, on_key, etc.) or background
        threads. Returns immediately without blocking. The coroutine runs in
        the background; use this when you don't need the return value::

            def on_render(self, ctx: RenderContext) -> None:
                if self._btn.render(ctx):
                    self.emit.schedule_task(self._do_query())

        Raises ``RuntimeError`` if the event loop hasn't started yet.
        """
        loop = self._app._loop
        if loop is None:
            raise RuntimeError(
                "emit.schedule_task() called before the event loop started. "
                "Only call it after App.run() is running (e.g. from on_init or later)."
            )
        try:
            on_loop_thread = asyncio.get_running_loop() is loop
        except RuntimeError:
            on_loop_thread = False

        if not on_loop_thread:
            return asyncio.run_coroutine_threadsafe(coro, loop)
        task = loop.create_task(coro)
        # Strong reference prevents GC before the task completes.
        self._app._background_tasks.add(task)
        task.add_done_callback(self._app._background_tasks.discard)
        task.add_done_callback(_log_background_task_error)
        return task

    # kind = "choice"
    @_blocking_emit_method
    async def notify_choice(self, title: str, options: list, priority: int,
                            body: str = "",
                            level: str = "info", required: bool = False,
                            image_inline: "dict | None" = None,
                            image_pipe_id: "str | None" = None,
                            timeout_secs: "int | None" = None,
                            on_dismiss: "str | None" = None) -> str:
        """Post a choice notification and await until the user picks one.

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
        """
        # Warn if any option uses a reserved shortcut key.
        import warnings
        for opt in options:
            sc = opt.get("shortcut", "")
            if sc and sc.lower() in _RESERVED_SHORTCUTS:
                warnings.warn(
                    f"notify_choice: shortcut {sc!r} on option {opt.get('label', '')!r} "
                    f"conflicts with notification navigation keys (j/k/h/l, 1-9). "
                    f"Use y/n or another letter. The host will strip it.",
                    stacklevel=2,
                )
        notify_id = str(uuid.uuid4())
        q: asyncio.Queue[str] = _make_async_queue()
        self._app._pending_notify[notify_id] = q
        payload = {"type": "notify", "level": level, "title": title, "body": body,
                   "kind": "choice", "options": options, "required": required,
                   "priority": priority,
                   "notify_id": notify_id}
        if image_inline is not None:
            payload["image_inline"] = image_inline
        if image_pipe_id is not None:
            payload["image_pipe_id"] = image_pipe_id
        if timeout_secs is not None:
            payload["timeout_secs"] = int(timeout_secs)
        if on_dismiss is not None:
            payload["on_dismiss"] = on_dismiss
        _emit(payload)
        return await q.get()

    # Convenience wrapper for the inline-image case (#74). Handles base64
    # encoding + size-cap enforcement client-side so we never spend wire
    # bytes on payloads the host will only reject.
    @_blocking_emit_method
    async def notify_with_image(self, title: str, body: str, image_bytes: bytes,
                                mime: str, priority: int,
                                level: str = "info",
                                choices: "list | None" = None) -> "str | None":
        """Post a notification with an inline base64-encoded image attachment.

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
        """
        if mime not in ("image/png", "image/jpeg"):
            raise ValueError(f"notify_with_image() mime must be image/png or image/jpeg, got {mime!r}")
        # 50 KB cap: matches the host's `MAX_INLINE_IMAGE_BYTES`. Apps that
        # exceed this should use the pipe-ref path. Failing locally is the
        # least surprising behaviour.
        MAX_INLINE_BYTES = 50 * 1024
        if len(image_bytes) > MAX_INLINE_BYTES:
            raise ValueError(
                f"notify_with_image(): image is {len(image_bytes)} bytes, "
                f"exceeds {MAX_INLINE_BYTES}-byte cap. Use image_pipe_id for large images."
            )
        import base64
        encoded = base64.standard_b64encode(image_bytes).decode("ascii")
        image_inline = {"mime": mime, "base64": encoded}
        if choices is None:
            self.notify(title=title, body=body, level=level,
                        priority=priority, image_inline=image_inline)
            return None
        return await self.notify_choice(title=title, options=choices, body=body,
                                        level=level, priority=priority,
                                        image_inline=image_inline)

    # kind = "input"
    @_blocking_emit_method
    async def notify_input(self, title: str, priority: int,
                           prompt: str = "", body: str = "",
                           level: str = "info", required: bool = False,
                           timeout_secs: "int | None" = None,
                           on_dismiss: "str | None" = None) -> str:
        """Post an input notification and await until the user submits or
        cancels. Returns the typed text (possibly empty), or "__cancel__" if
        the user dismissed with Esc (only possible when required=False).

        `priority` is required (int, higher = more urgent).

        Await this from async hooks. From background threads use
        ``self.emit.run_sync(self.emit.notify_input(...))``.
        """
        notify_id = str(uuid.uuid4())
        q: asyncio.Queue[str] = _make_async_queue()
        self._app._pending_notify[notify_id] = q
        payload = {"type": "notify", "level": level, "title": title, "body": body,
                   "kind": "input", "input_prompt": prompt, "required": required,
                   "priority": priority,
                   "notify_id": notify_id}
        if timeout_secs is not None:
            payload["timeout_secs"] = int(timeout_secs)
        if on_dismiss is not None:
            payload["on_dismiss"] = on_dismiss
        _emit(payload)
        return await q.get()

    @_blocking_emit_method
    async def notify_and_wait(self, title: str, priority: int,
                              body: str = "", level: str = "info",
                              actions: "list | None" = None,
                              timeout_secs: "int | None" = None,
                              on_dismiss: "str | None" = None) -> str:
        """Post a message notification and await until the user acknowledges
        or cancels. Returns "acknowledge" on Enter/Space/button, "cancel" on Esc.

        For richer interaction, use `notify_choice` or `notify_input`.
        `priority` is required (int, higher = more urgent).
        `actions` is the legacy server-side side-effect list.

        Await this from async hooks. From background threads use
        ``self.emit.run_sync(self.emit.notify_and_wait(...))``.
        """
        notify_id = str(uuid.uuid4())
        q: asyncio.Queue[str] = _make_async_queue()
        self._app._pending_notify[notify_id] = q
        payload = {"type": "notify", "level": level, "title": title, "body": body,
                   "kind": "message", "actions": actions or [],
                   "priority": priority,
                   "notify_id": notify_id}
        if timeout_secs is not None:
            payload["timeout_secs"] = int(timeout_secs)
        if on_dismiss is not None:
            payload["on_dismiss"] = on_dismiss
        _emit(payload)
        return await q.get()

    # ── Non-blocking (callback-based) notify variants (#310) ─────────────────
    # Each method returns immediately with the notify_id (str). The callback
    # `on_response` fires on the event thread when the user interacts.
    # Registry entry is removed after the first invocation.

    def notify_async(self, title: str, priority: int, body: str = "",
                     level: str = "info",
                     actions: "list | None" = None,
                     image_inline: "dict | None" = None,
                     image_pipe_id: "str | None" = None,
                     timeout_secs: "int | None" = None,
                     on_dismiss: "str | None" = None,
                     on_response: "Any" = None) -> str:
        """Non-blocking notify_and_wait. Returns immediately with notify_id.

        If on_response is None, behaves like fire-and-forget notify() — returns "".
        If on_response is set, registers the callback; it receives "acknowledge"
        or "__cancel__" when the user interacts.

        `priority` is required (int, higher = more urgent).
        """
        if on_response is None:
            self.notify(title=title, priority=priority, body=body, level=level,
                        actions=actions, image_inline=image_inline,
                        image_pipe_id=image_pipe_id, timeout_secs=timeout_secs,
                        on_dismiss=on_dismiss)
            return ""
        notify_id = str(uuid.uuid4())
        self._app._pending_notify_callbacks[notify_id] = on_response
        self.info(f"notify_async: registered callback for {notify_id!r}")
        payload: dict = {"type": "notify", "level": level, "title": title,
                         "body": body, "kind": "message", "actions": actions or [],
                         "priority": priority, "notify_id": notify_id}
        if image_inline is not None:
            payload["image_inline"] = image_inline
        if image_pipe_id is not None:
            payload["image_pipe_id"] = image_pipe_id
        if timeout_secs is not None:
            payload["timeout_secs"] = int(timeout_secs)
        if on_dismiss is not None:
            payload["on_dismiss"] = on_dismiss
        _emit(payload)
        return notify_id

    def notify_choice_async(self, title: str, options: list, priority: int,
                            body: str = "",
                            level: str = "info", required: bool = False,
                            image_inline: "dict | None" = None,
                            image_pipe_id: "str | None" = None,
                            timeout_secs: "int | None" = None,
                            on_dismiss: "str | None" = None,
                            on_response: "Any" = None) -> str:
        """Non-blocking notify_choice. Returns immediately with notify_id.

        `on_response` receives the chosen option's value (or label if no value
        set), or "__cancel__" if the user dismissed (required=False only).
        `priority` is required (int, higher = more urgent).
        """
        import warnings
        for opt in options:
            sc = opt.get("shortcut", "")
            if sc and sc.lower() in _RESERVED_SHORTCUTS:
                warnings.warn(
                    f"notify_choice_async: shortcut {sc!r} on option "
                    f"{opt.get('label', '')!r} conflicts with notification "
                    f"navigation keys (j/k/h/l, 1-9). Use y/n or another letter.",
                    stacklevel=2,
                )
        notify_id = str(uuid.uuid4())
        if on_response is not None:
            self._app._pending_notify_callbacks[notify_id] = on_response
            self.info(f"notify_choice_async: registered callback for {notify_id!r}")
        payload: dict = {"type": "notify", "level": level, "title": title,
                         "body": body, "kind": "choice", "options": options,
                         "required": required, "priority": priority,
                         "notify_id": notify_id}
        if image_inline is not None:
            payload["image_inline"] = image_inline
        if image_pipe_id is not None:
            payload["image_pipe_id"] = image_pipe_id
        if timeout_secs is not None:
            payload["timeout_secs"] = int(timeout_secs)
        if on_dismiss is not None:
            payload["on_dismiss"] = on_dismiss
        _emit(payload)
        return notify_id

    def notify_input_async(self, title: str, priority: int,
                           prompt: str = "", body: str = "",
                           level: str = "info", required: bool = False,
                           timeout_secs: "int | None" = None,
                           on_dismiss: "str | None" = None,
                           on_response: "Any" = None) -> str:
        """Non-blocking notify_input. Returns immediately with notify_id.

        `on_response` receives the typed text (possibly empty), or "__cancel__"
        if the user dismissed (required=False only).
        `priority` is required (int, higher = more urgent).
        """
        notify_id = str(uuid.uuid4())
        if on_response is not None:
            self._app._pending_notify_callbacks[notify_id] = on_response
            self.info(f"notify_input_async: registered callback for {notify_id!r}")
        payload: dict = {"type": "notify", "level": level, "title": title,
                         "body": body, "kind": "input", "input_prompt": prompt,
                         "required": required, "priority": priority,
                         "notify_id": notify_id}
        if timeout_secs is not None:
            payload["timeout_secs"] = int(timeout_secs)
        if on_dismiss is not None:
            payload["on_dismiss"] = on_dismiss
        _emit(payload)
        return notify_id

    def notify_and_wait_async(self, title: str, priority: int,
                              body: str = "", level: str = "info",
                              actions: "list | None" = None,
                              timeout_secs: "int | None" = None,
                              on_dismiss: "str | None" = None,
                              on_response: "Any" = None) -> str:
        """Non-blocking notify_and_wait. Returns immediately with notify_id.

        `on_response` receives "acknowledge" on Enter/Space/button, or
        "__cancel__" on Esc.
        `priority` is required (int, higher = more urgent).
        """
        notify_id = str(uuid.uuid4())
        if on_response is not None:
            self._app._pending_notify_callbacks[notify_id] = on_response
            self.info(f"notify_and_wait_async: registered callback for {notify_id!r}")
        payload: dict = {"type": "notify", "level": level, "title": title,
                         "body": body, "kind": "message", "actions": actions or [],
                         "priority": priority, "notify_id": notify_id}
        if timeout_secs is not None:
            payload["timeout_secs"] = int(timeout_secs)
        if on_dismiss is not None:
            payload["on_dismiss"] = on_dismiss
        _emit(payload)
        return notify_id

    # Terminal commands (legacy back-compat)
    def run_in_terminal(self, command: str) -> None:
        _emit({"type": "run_in_terminal", "command": command})

    def cd(self, path: str) -> None:
        _emit({"type": "cd", "path": path})

    def status_summary(self, text: str) -> None:
        _emit({"type": "status_summary", "text": text})

    # ── Navigation stack (#392) ─────────────────────────────────────────────

    def push_nav(self, view_id: str, title: str) -> None:
        """Push a new view onto the host navigation stack.

        The host appends an entry keyed on `view_id` with display `title`
        and shows a back arrow + `title` in the pane chrome while this view
        is active. Cmd+[ (or clicking the back arrow) sends
        ``PlexiEvent::NavBack { view_id }`` back to the app, where
        ``view_id`` is the view being navigated *back to* (the entry below
        the current top, or empty string for root).

        The app is responsible for tracking its own internal view state and
        calling ``pop_nav()`` after navigating back.
        """
        _emit({"type": "push_nav", "view_id": view_id, "title": title})

    def pop_nav(self) -> None:
        """Pop the current view off the host navigation stack.

        Call this after the app has already rendered the previous view (e.g.
        inside the ``on_nav_back`` handler). The host removes the top entry
        from the stack; if the stack becomes empty the back arrow disappears.
        """
        _emit({"type": "pop_nav"})

    # ── File picker (#514) ──────────────────────────────────────────────────

    def open_file_picker(
        self,
        request_id: str,
        filter: "list[str] | None" = None,
        multiple: bool = False,
    ) -> None:
        """Show a native macOS file picker dialog. Requires ``fs.pick`` capability.

        The host responds asynchronously with either:
        - ``on_file_picked(ctx, request_id, paths)`` — one or more files selected
        - ``on_file_pick_cancelled(ctx, request_id)`` — user dismissed the dialog

        Args:
            request_id: Caller-supplied correlation ID echoed back in the response.
            filter: File extension whitelist without leading dots
                    (e.g. ``["mp4", "mov"]``). ``None`` or empty = all files.
            multiple: Allow picking more than one file.
        """
        _emit({
            "type": "open_file_picker",
            "request_id": request_id,
            "filter": filter or [],
            "multiple": multiple,
        })

    def spawn_pane(
        self,
        type_id: str,
        layout: str = "split_v",
        args: "list[str] | None" = None,
        pipe_id: "str | None" = None,
        from_pane_id: "int | None" = None,
        request_id: "str | None" = None,
        target_context: "int | None" = None,
    ) -> None:
        """Request the host to open a new pane with the given app (#592).

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
        """
        cmd: "dict[str, object]" = {
            "type": "spawn_pane",
            "type_id": type_id,
            "layout": layout,
            "args": args or [],
        }
        if pipe_id is not None:
            cmd["pipe_id"] = pipe_id
        if from_pane_id is not None:
            cmd["from_pane_id"] = from_pane_id
        if request_id is not None:
            cmd["request_id"] = request_id
        if target_context is not None:
            cmd["target_context"] = target_context
        _emit(cmd)

    def query_context_state(self, context_id: int) -> None:
        """Query the state of a context (#1518).

        The host responds with ``on_context_state(state)`` containing pane
        count, child contexts, and aggregate status. The requesting pane must
        be in the queried context itself or in an ancestor context (visibility
        boundary). Non-descendant queries are silently denied.

        Args:
            context_id: The context to query.
        """
        _emit({"type": "query_context_state", "context_id": context_id})

    def cd_to(self, cwd: str) -> None:
        """Request the host to cd all terminals in the same pane group to `cwd`."""
        _emit({"type": "cd_request", "cwd": cwd})

    def copy_to_clipboard(self, text: str) -> None:
        """Write `text` to the OS clipboard via the host (issue #146).

        Routed through `egui::Context::copy_text` so the platform backend
        (NSPasteboard / X11 / Wayland / Win32) handles the actual write.
        Synchronous from the app's perspective — no acknowledgement event.
        No capability flag is required; clipboard writes are low-risk and
        the app already chooses when to fire (key handler, button, etc.).
        """
        _emit({"type": "copy_to_clipboard", "text": text})


    # ── Canvas Terminal Binding Primitives (#78) ────────────────────────────
    #
    # All five gate on the manifest capability `terminal.bindings`. Apps
    # without it get either a sentinel response (for the request/response
    # primitives) or a silent drop (for fire-and-forget primitives) — the
    # host logs `capability denied` either way so the bug is debuggable.
    #
    # The lifecycle: call `request_linked_terminal()` once at startup
    # (typically inside `on_init`) and stash the returned pane id. Pass it
    # to every subsequent `run_in_linked_terminal` / `insert_path_token` /
    # `request_command_preview` call.
    @_blocking_emit_method
    async def request_linked_terminal(
        self,
        cwd: "str | None" = None,
        label: "str | None" = None,
    ) -> int:
        """Ask the host to open a fresh terminal pane next to this app and
        return its `terminal_pane_id` (an integer). Awaits until the host
        emits `LinkedTerminalReady`.

        Raises `CapabilityDeniedError` if the manifest doesn't declare
        `terminal.bindings`.

        Await this from async hooks. From background threads use
        ``self.emit.run_sync(self.emit.request_linked_terminal(...))``.
        """
        req_id = str(uuid.uuid4())
        q: asyncio.Queue[int] = _make_async_queue()
        self._app._pending_linked_terminal[req_id] = q
        _emit({
            "type": "request_linked_terminal",
            "request_id": req_id,
            "cwd": cwd,
            "label": label,
        })
        pane_id = await q.get()
        if pane_id == 0:
            raise CapabilityDeniedError(
                "request_linked_terminal: capability 'terminal.bindings' "
                "not declared in manifest"
            )
        return int(pane_id)

    def run_in_linked_terminal(
        self,
        terminal_pane_id: int,
        command: str,
        echo: bool = True,
    ) -> None:
        """Run `command` in the linked terminal. With `echo=True` the user
        sees the command typed into the terminal; with `echo=False` the
        signalled intent is silent execution (PTY-level echo is shell-
        controlled — the flag is best-effort observational).

        Fire-and-forget — capability denial drops silently with a host
        log line."""
        _emit({
            "type": "run_in_linked_terminal",
            "terminal_pane_id": int(terminal_pane_id),
            "command": command,
            "echo": bool(echo),
        })

    def insert_path_token(
        self,
        terminal_pane_id: int,
        path: str,
        mode: str = "replace",
    ) -> None:
        """Inject `path` at the linked terminal's cursor.

        `mode = "replace"` — Ctrl-W (kill-word) before the path so the
                            shell readline removes the partial token.
        `mode = "append"`  — write the path verbatim.

        The host POSIX-quotes the path when it contains shell metacharacters.
        Fire-and-forget — capability denial drops silently with a host log
        line."""
        if mode not in ("replace", "append"):
            raise ValueError(
                f"insert_path_token: mode must be 'replace' or 'append', got {mode!r}"
            )
        _emit({
            "type": "insert_path_token",
            "terminal_pane_id": int(terminal_pane_id),
            "path": path,
            "mode": mode,
        })

    @_blocking_emit_method
    async def request_command_preview(
        self,
        terminal_pane_id: int,
        command: str,
    ) -> "tuple[str, str]":
        """Return `(command, would_run_in_cwd)` for `command` in the linked
        terminal. Doesn't execute. Useful for confirmation modals before
        destructive operations.

        If the host denies the request, `would_run_in_cwd` is empty string;
        the SDK does NOT raise — apps that want to be loud should check for
        empty cwd themselves (the most common failure mode is a missing
        capability, which the host already logs).

        Await this from async hooks. From background threads use
        ``self.emit.run_sync(self.emit.request_command_preview(...))``.
        """
        req_id = str(uuid.uuid4())
        q: asyncio.Queue[tuple[str, str]] = _make_async_queue()
        self._app._pending_command_preview[req_id] = q
        _emit({
            "type": "request_command_preview",
            "request_id": req_id,
            "terminal_pane_id": int(terminal_pane_id),
            "command": command,
        })
        return await q.get()

    def open_artifact(
        self,
        path: str,
        mode: str = "open_in_pane",
    ) -> None:
        """Open a workspace artifact via the host.

        Modes:
          - `"open_in_pane"`        — directories open the file browser
                                      next to the app; files open with
                                      the OS default app (v3.5).
          - `"reveal_in_finder"`    — `open -R <path>` on macOS.
          - `"open_with_default"`   — `open <path>` on macOS.

        Fire-and-forget. Capability denial drops silently with a host log
        line."""
        if mode not in ("open_in_pane", "reveal_in_finder", "open_with_default"):
            raise ValueError(
                f"open_artifact: mode must be one of "
                f"open_in_pane / reveal_in_finder / open_with_default, "
                f"got {mode!r}"
            )
        _emit({"type": "open_artifact", "path": path, "mode": mode})

    def schedule_render(self, after_ms: int = 16) -> None:
        """Ask the host to send a new Render event after `after_ms` milliseconds.
        Call at the end of on_render to sustain a game/animation loop.
        16 ms ≈ 60 fps.  32 ms ≈ 30 fps."""
        _emit({"type": "schedule_render", "after_ms": after_ms})

    # Runs
    def run_get(self, intent: str, payload: Any = None) -> str:
        run_id = str(uuid.uuid4())
        _emit({"type": "run_get", "intent": intent,
               "payload": payload or {}})
        return run_id

    # ── Async blocking helpers ────────────────────────────────────────────────
    # Each of these is a coroutine — await them from async hooks, or call
    # self.emit.run_sync(self.emit.method(...)) from a background thread.

    @_blocking_emit_method
    async def capability_request(self, capability: str) -> bool:
        """Await until host grants or denies the capability. Returns True if granted.

        Await from async hooks. From background threads:
        ``self.emit.run_sync(self.emit.capability_request(capability))``.
        """
        req_id = str(uuid.uuid4())
        q: asyncio.Queue[bool] = _make_async_queue()
        self._app._pending_capability[req_id] = q
        _emit({"type": "capability_request", "request_id": req_id,
               "capability": capability})
        return await q.get()

    @_blocking_emit_method
    async def secret_get(self, key: str) -> "str | None":
        """Await until host returns the secret value (or None if denied).

        From background threads:
        ``self.emit.run_sync(self.emit.secret_get(key))``.
        """
        q: asyncio.Queue[str | None] = _make_async_queue()
        self._app._pending_secret[key] = q
        _emit({"type": "secret_get", "key": key})
        return await q.get()

    @_blocking_emit_method
    async def get_secret(self, key: str) -> "str | None":
        """Alias for secret_get(). Preferred name going forward."""
        return await self.secret_get(key)

    @_blocking_emit_method
    async def http_get(self, url: str) -> str:
        """HTTP GET brokered through the host. Requires net.http capability.
        Raises RuntimeError on failure.

        From background threads:
        ``self.emit.run_sync(self.emit.http_get(url))``.
        """
        return await self.http_request(url)

    @_blocking_emit_method
    async def http_request(self, url: str, method: str = "GET",
                           headers: "dict[str, str] | None" = None,
                           body: "str | None" = None) -> str:
        """HTTP request brokered through the host. Requires net.http capability.
        Supports custom method, headers, and body. Raises RuntimeError on failure.

        From background threads:
        ``self.emit.run_sync(self.emit.http_request(...))``.
        """
        req_id = str(uuid.uuid4())
        q: asyncio.Queue[tuple[str, str]] = _make_async_queue()
        self._app._pending_http[req_id] = q
        payload: dict = {"type": "http_request", "request_id": req_id,
                         "method": method, "url": url}
        if headers:
            payload["headers"] = headers
        if body is not None:
            payload["body"] = body
        _emit(payload)
        status, value = await q.get()
        if status == "error":
            raise RuntimeError(f"http_request {url!r}: {value}")
        return value

    @_blocking_emit_method
    async def load_image(self, src: str) -> str:
        """Request an async image fetch brokered through the host. Requires net.http capability.

        Returns a stable handle ID. Pass the handle to ``ctx.image(handle, x, y, w, h)``
        to render. While loading, ``ctx.image()`` renders a placeholder rect. On load
        completion the host fires ``PlexiEvent::ImageLoaded`` which triggers a re-render.

        Raises ``RuntimeError`` immediately if ``net.http`` is not declared in the manifest.
        Raises ``RuntimeError`` if the fetch fails (network error, bad URL, decode error).

        From background threads:
        ``self.emit.run_sync(self.emit.load_image(url))``.
        """
        if "net.http" not in self._app.capabilities:
            raise RuntimeError(
                "load_image: net.http capability not declared in manifest — "
                "add 'net.http' to [capabilities] in manifest.toml"
            )
        handle = str(uuid.uuid4())
        q: asyncio.Queue[tuple[str, "str | None"]] = _make_async_queue()
        self._app._pending_image[handle] = q
        _emit({"type": "load_image", "handle": handle, "src": src})
        status, message = await q.get()
        if status == "error":
            raise RuntimeError(f"load_image {src!r}: {message}")
        return handle

    @_blocking_emit_method
    async def ai_query(self, model_tier: str, system: str,
                       messages: "list[dict]",
                       tools: "list[dict] | None" = None) -> AiResponse:
        """Call into the host's Plexi AI broker (#284). Requires the
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
        """
        if model_tier not in ("low", "medium", "high"):
            raise ValueError(
                f"ai_query: model_tier must be one of low|medium|high, got {model_tier!r}"
            )
        req_id = str(uuid.uuid4())
        q: asyncio.Queue[dict] = _make_async_queue()
        self._app._pending_ai[req_id] = q
        payload: dict = {
            "type": "ai_query",
            "request_id": req_id,
            "model_tier": model_tier,
            "system": system,
            "messages": messages,
            "tools": tools or [],
        }
        _emit(payload)
        try:
            ev = await asyncio.wait_for(q.get(), timeout=35.0)
        except asyncio.TimeoutError:
            self._app._pending_ai.pop(req_id, None)
            raise RuntimeError(
                f"ai_query timed out after 35s (req_id={req_id!r}) — "
                "check OPENROUTER_API_KEY and network connectivity"
            )
        error = ev.get("error")
        if error is not None:
            if "capability denied" in error:
                raise CapabilityDeniedError(error)
            raise RuntimeError(f"ai_query failed: {error}")
        return AiResponse(
            content=ev.get("content") or "",
            tokens_in=int(ev.get("tokens_in", 0) or 0),
            tokens_out=int(ev.get("tokens_out", 0) or 0),
        )

    def mcp_tool_result(self, call_id: str, result: "dict | None" = None, error: "str | None" = None) -> None:
        """Send a McpToolResult response for a pending McpToolCall.

        One of ``result`` or ``error`` must be provided (not both, not neither).
        ``result`` should be a MCP CallToolResult dict, e.g.:
        ``{"content": [{"type": "text", "text": "done"}]}``.
        """
        if result is None and error is None:
            raise ValueError("mcp_tool_result: either 'result' or 'error' must be provided")
        if result is not None and error is not None:
            raise ValueError("mcp_tool_result: 'result' and 'error' are mutually exclusive")
        _emit({
            "type": "mcp_tool_result",
            "call_id": call_id,
            "result": result,
            "error": error,
        })

    def expose_tools(self, tools: "list[dict]") -> None:
        """Declare callable tools to the host (#398, #399).

        Each entry in `tools` must have:
          - ``name`` (str): unique tool identifier.
          - ``description`` (str): human-readable description for the LLM.
          - ``input_schema`` (dict): JSON Schema object (type=object).
          - ``timeout_ms`` (int, optional): max ms to wait for result (default 30 000).

        The host registers these tools in the global registry. When an AI turn
        requests a tool call the host sends a ``PlexiEvent::ToolCall``; the SDK
        routes it to the handler registered via ``@app.tool(…)``.
        """
        _emit({"type": "expose_tools", "tools": tools})

    @_blocking_emit_method
    async def list_midi_devices(self, timeout: float = 5.0) -> "MidiDeviceList":
        """Enumerate CoreMIDI input + output ports (#320).
        No capability gate — port names are publicly visible in Audio MIDI
        Setup. Requires `midi.in` / `midi.out` only on subsequent open/send.

        Returns a MidiDeviceList with `inputs` and `outputs` lists of
        MidiPortInfo (id, name, default).

        Raises RuntimeError on host enumeration failure or timeout.

        From background threads:
        ``self.emit.run_sync(self.emit.list_midi_devices())``.
        """
        req_id = str(uuid.uuid4())
        q: asyncio.Queue[dict] = _make_async_queue()
        self._app._pending_midi_devices[req_id] = q
        _emit({"type": "list_midi_devices", "request_id": req_id})
        try:
            ev = await asyncio.wait_for(q.get(), timeout=timeout)
        except asyncio.TimeoutError:
            self._app._pending_midi_devices.pop(req_id, None)
            raise RuntimeError(
                f"list_midi_devices timed out after {timeout}s — host did not respond"
            )
        if ev.get("error"):
            raise RuntimeError(f"list_midi_devices failed: {ev.get('error')}")
        return MidiDeviceList(
            inputs=[
                MidiPortInfo(
                    id=str(p.get("id", "")),
                    name=str(p.get("name", "")),
                    default=bool(p.get("default", False)),
                )
                for p in ev.get("inputs", [])
            ],
            outputs=[
                MidiPortInfo(
                    id=str(p.get("id", "")),
                    name=str(p.get("name", "")),
                    default=bool(p.get("default", False)),
                )
                for p in ev.get("outputs", [])
            ],
        )

    @_blocking_emit_method
    async def list_audio_devices(self, timeout: float = 5.0) -> "AudioDeviceList":
        """Enumerate CoreAudio input + output devices (#341).
        No capability gate — device names are publicly visible.
        Requires ``audio.record`` / ``audio.playback`` only on subsequent capture/play.

        Returns an AudioDeviceList with ``inputs`` and ``outputs`` lists of
        AudioDeviceInfo (id, name, default).

        Raises RuntimeError on host enumeration failure or timeout.

        From background threads:
        ``self.emit.run_sync(self.emit.list_audio_devices())``.
        """
        req_id = str(uuid.uuid4())
        q: asyncio.Queue[dict] = _make_async_queue()
        self._app._pending_audio_devices[req_id] = q
        _emit({"type": "list_audio_devices", "request_id": req_id})
        try:
            ev = await asyncio.wait_for(q.get(), timeout=timeout)
        except asyncio.TimeoutError:
            self._app._pending_audio_devices.pop(req_id, None)
            raise RuntimeError(
                f"list_audio_devices timed out after {timeout}s — host did not respond"
            )
        if ev.get("error"):
            raise RuntimeError(f"list_audio_devices failed: {ev.get('error')}")
        return AudioDeviceList(
            inputs=[
                AudioDeviceInfo(
                    id=str(p.get("id", "")),
                    name=str(p.get("name", "")),
                    default=bool(p.get("default", False)),
                )
                for p in ev.get("inputs", [])
            ],
            outputs=[
                AudioDeviceInfo(
                    id=str(p.get("id", "")),
                    name=str(p.get("name", "")),
                    default=bool(p.get("default", False)),
                )
                for p in ev.get("outputs", [])
            ],
        )

    @_blocking_emit_method
    async def list_camera_devices(self, timeout: float = 5.0) -> "CameraDeviceList":
        """Enumerate camera devices via host AVFoundation broker (#1505).
        Requires ``video.capture`` capability — camera names are gated by TCC.

        Returns a CameraDeviceList with a ``cameras`` list of CameraDeviceInfo (id, name).

        Raises RuntimeError on host enumeration failure, capability denial, or timeout.

        From background threads:
        ``self.emit.run_sync(self.emit.list_camera_devices())``.
        """
        req_id = str(uuid.uuid4())
        q: asyncio.Queue[dict] = _make_async_queue()
        self._app._pending_camera_devices[req_id] = q
        _emit({"type": "list_camera_devices", "request_id": req_id})
        try:
            ev = await asyncio.wait_for(q.get(), timeout=timeout)
        except asyncio.TimeoutError:
            self._app._pending_camera_devices.pop(req_id, None)
            raise RuntimeError(
                f"list_camera_devices timed out after {timeout}s — host did not respond"
            )
        if ev.get("error"):
            raise RuntimeError(f"list_camera_devices failed: {ev.get('error')}")
        return CameraDeviceList(
            cameras=[
                CameraDeviceInfo(
                    id=str(c.get("id", "")),
                    name=str(c.get("name", "")),
                )
                for c in ev.get("cameras", [])
            ]
        )

    def open_midi_input(self, port_id: str, pipe_id: str) -> "Pipe":
        """Open a CoreMIDI input port and pipe its MIDI byte streams into a
        binary pipe. Requires `midi.in` capability declared in manifest.toml.

        Returns the Pipe handle. The host emits `PipeOpened` then
        `MidiInputOpened`; the Pipe auto-connects to its socket on
        `PipeOpened`. Apps then call `pipe.read_frame()` in a loop to
        receive 1–3 byte MIDI messages (channel-voice / system real-time).

        Each pipe frame is one MIDI 1.0 message — apps don't need to parse
        message boundaries themselves.
        """
        # Import here to avoid circular import at module load time.
        from ._pipe import Pipe
        # Open the binary pipe FIRST so the App's `_pipes` registry has it
        # before PipeOpened arrives. The host emits PipeOpened then
        # MidiInputOpened in order; both events are received by the event
        # loop and forwarded to the Pipe / on_midi_input_opened handler.
        p = Pipe(pipe_id=pipe_id, mode="binary", direction="in", app=self._app)
        self._app._pipes[pipe_id] = p
        _emit({
            "type": "open_midi_input",
            "port_id": port_id,
            "pipe_id": pipe_id,
        })
        return p

    def close_midi_input(self, port_id: str) -> None:
        """Close a previously-opened MIDI input port. The associated binary
        pipe drains and closes; the app should drop its Pipe handle."""
        _emit({"type": "close_midi_input", "port_id": port_id})

    def send_midi(self, port_id: str, bytes_: "bytes | bytearray | list[int]") -> None:
        """Fire-and-forget send of one MIDI 1.0 byte stream to `port_id`.
        Requires `midi.out` capability. The host opens the output port lazily
        on the first send and reuses the handle afterwards.

        `bytes_` is a 1–3 byte sequence: NoteOn `[0x90+ch, note, vel]`,
        NoteOff `[0x80+ch, note, vel]`, CC `[0xB0+ch, num, val]`, clock
        pulse `[0xF8]`, etc. Use `plexi_sdk.midi` helpers to construct
        messages with named arguments.

        Successful sends produce no event; failures arrive as a logged
        `midi_send_error` warning. Apps that need explicit error handling
        should validate the destination via `list_midi_devices()` first.
        """
        if isinstance(bytes_, (bytes, bytearray)):
            data = list(bytes_)
        else:
            data = list(bytes_)
        # JSON wire shape: array of integers. The host accepts any ints in
        # 0..=255 and rejects empty arrays.
        _emit({"type": "send_midi", "port_id": port_id, "bytes": data})

    @_blocking_emit_method
    async def open_video(
        self,
        source: str,
        pipe_id: str,
        timeout: float = 10.0,
    ) -> "VideoHandle":
        """Open a video decoder against `source` and return a VideoHandle
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
        """
        from ._pipe import Pipe
        from ._types import VideoHandle
        # Open the binary pipe FIRST so the App's `_pipes` registry has it
        # before PipeOpened arrives. The host emits PipeOpened, then
        # VideoOpenAck. The Pipe auto-connects to its socket on PipeOpened.
        pipe = Pipe(pipe_id=pipe_id, mode="binary", direction="in", app=self._app)
        self._app._pipes[pipe_id] = pipe

        req_id = str(uuid.uuid4())
        q: asyncio.Queue[dict] = _make_async_queue()
        self._app._pending_video_open[req_id] = q
        _emit({
            "type": "open_video",
            "request_id": req_id,
            "source": source,
            "pipe_id": pipe_id,
        })
        try:
            ev = await asyncio.wait_for(q.get(), timeout=timeout)
        except asyncio.TimeoutError:
            self._app._pending_video_open.pop(req_id, None)
            raise RuntimeError(
                f"open_video timed out after {timeout}s — host did not respond"
            )
        error = ev.get("error")
        if error is not None:
            if "capability denied" in error:
                raise CapabilityDeniedError(error)
            raise RuntimeError(f"open_video failed: {error}")
        return VideoHandle(
            handle_id=int(ev.get("handle_id", 0)),
            width=int(ev.get("width", 0)),
            height=int(ev.get("height", 0)),
            fps=float(ev.get("fps", 0.0)),
            duration_ms=int(ev.get("duration_ms", 0)),
            pipe=pipe,
        )

    def set_video_state(self, handle_id: int, state: str, position_ms: int = 0) -> None:
        """Drive playback for a video opened with `open_video` (#345).
        `state` is one of `"play"`, `"pause"`, `"seek"`. For `"seek"`,
        `position_ms` is the absolute target position in milliseconds.
        Fire-and-forget — no response event."""
        if state == "seek":
            payload: dict[str, Any] = {"kind": "seek", "position_ms": int(position_ms)}
        elif state == "play":
            payload = {"kind": "play"}
        elif state == "pause":
            payload = {"kind": "pause"}
        else:
            raise ValueError(
                f"set_video_state: unknown state {state!r} (expected play|pause|seek)"
            )
        _emit({
            "type": "set_video_state",
            "handle_id": int(handle_id),
            "state": payload,
        })

    def close_video(self, handle_id: int) -> None:
        """Close a previously-opened video handle (#345). The host tears down
        the decoder and drains the binary pipe. No response event."""
        _emit({"type": "close_video", "handle_id": int(handle_id)})

    def audio_capture(
        self,
        pipe_id: str,
        sample_rate: int = 48000,
        buffer_size: int = 512,
        device_id: "str | None" = None,
    ) -> "Pipe":
        """Start mic capture (#277). PCM frames stream as raw f32 PCM on
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
        """
        from ._pipe import Pipe
        # Close any existing pipe for this id before replacing — prevents
        # orphaned sockets when the app restarts capture with the same pipe_id.
        existing = self._app._pipes.pop(pipe_id, None)
        if existing is not None:
            existing.close()
        # Open the binary pipe FIRST so PipeOpened can attach when it
        # arrives — same shape as ``open_video``.
        pipe = Pipe(pipe_id=pipe_id, mode="binary", direction="in", app=self._app)
        self._app._pipes[pipe_id] = pipe
        _emit({
            "type": "audio_capture",
            "pipe_id": pipe_id,
            "device_id": device_id,
            "sample_rate": int(sample_rate),
            "buffer_size": int(buffer_size),
        })
        return pipe

    def stop_audio_capture(self, pipe_id: str) -> None:
        """Stop an active audio capture and close its pipe (#862).

        Closes the Pipe registered under ``pipe_id`` and removes it from
        the internal registry. The host cleans up the stale session on the
        next ``AudioCapture`` for the same ``pipe_id``. No host command is
        needed — closing the socket is sufficient.

        Apps should join any reader thread after calling this to avoid
        reading from a closed socket::

            self.emit.stop_audio_capture("mic")
            self._reader_thread.join(timeout=2.0)

        No-op if ``pipe_id`` is not registered.
        """
        pipe = self._app._pipes.pop(pipe_id, None)
        if pipe is not None:
            pipe.close()

    def audio_play(self, source: str, volume: float = 1.0) -> None:
        """Play an audio file via the host (rodio). Requires audio.playback capability.

        source: absolute path to audio file (mp3, wav, ogg, etc.)
        volume: playback volume 0.0–1.0 (default 1.0)

        Fire-and-forget — no response event. Playback stops when the pane
        closes; there is no explicit stop API yet.
        """
        _emit({
            "type": "audio_play",
            "source": source,
            "state": "playing",
            "volume": float(volume),
        })

    def set_timer(self, timer_id: str, after_ms: int) -> None:
        """Fire PlexiEvent::Timer after after_ms milliseconds. Requires timer capability."""
        _emit({"type": "set_timer", "timer_id": timer_id, "after_ms": after_ms})

    def cancel_timer(self, timer_id: str) -> None:
        """Cancel a pending timer set with set_timer()."""
        _emit({"type": "cancel_timer", "timer_id": timer_id})

    def set_mouse_tracking(self, enabled: bool) -> None:
        """Enable or disable PlexiEvent::MouseMove delivery for this pane.

        MouseMove is off by default to avoid flooding apps that don't need
        continuous pointer tracking. Call set_mouse_tracking(True) after
        on_init to start receiving on_mouse_move callbacks. Call with False
        to stop.
        """
        _emit({"type": "set_mouse_tracking", "enabled": enabled})

    # Binary pipe
    def pipe_open(self, pipe_id: str, mode: str = "binary",
                  direction: str = "in") -> "Pipe":
        """Open a typed pipe. Returns a Pipe handle. mode: json|binary. direction: in|out|duplex."""
        from ._pipe import Pipe
        p = Pipe(pipe_id=pipe_id, mode=mode, direction=direction, app=self._app)
        self._app._pipes[pipe_id] = p
        _emit({"type": "pipe_open", "pipe_id": pipe_id,
               "mode": mode, "direction": direction})
        return p

    def pipe_open_directed(self, pipe_id: str, target_pane_id: int) -> "Pipe":
        """Open a *directed* JSON pipe to one specific target pane (#286).

        Mirrors `pipe_open` but the host scopes `PipeMessage` delivery so
        only the caller and `target_pane_id` see traffic on `pipe_id` —
        peers that coincidentally `pipe_open` the same id stay isolated.
        Always JSON-mode duplex; `pipe.send(payload)` is symmetric on
        either end.

        Capability: `pipe.open` (same as `pipe_open`).
        """
        from ._pipe import Pipe
        p = Pipe(pipe_id=pipe_id, mode="json", direction="duplex", app=self._app)
        self._app._pipes[pipe_id] = p
        _emit({
            "type": "pipe_open_directed",
            "pipe_id": pipe_id,
            "target_pane_id": int(target_pane_id),
        })
        return p

    def pipe_send(self, pipe_id: str, payload: Any) -> None:
        """Send a JSON payload on an already-open pipe by id (#286).

        Use this when you don't own a `Pipe` handle locally — for example
        when replying on a directed pipe a *peer* opened (you're the target;
        the host subscribed you, but the SDK doesn't auto-build a handle).
        Sends a `pipe_send` DrawCommand; the host routes per the existing
        directed-pipe pair table.
        """
        _emit({"type": "pipe_send", "pipe_id": pipe_id, "payload": payload})

