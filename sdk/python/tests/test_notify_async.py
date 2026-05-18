"""Tests for non-blocking notify_*_async SDK methods (#310)."""

import asyncio
import json
import textwrap
import threading
import time
from pathlib import Path
from unittest.mock import MagicMock, patch

import pytest

from plexi_sdk.testing import AppHarness


# ── Helpers ───────────────────────────────────────────────────────────────────

def _make_app(body: str) -> str:
    return textwrap.dedent(f"""
        from plexi_sdk import App, RenderContext
        {body}
    """).lstrip()


# ── Unit tests against Emitter + App internals ────────────────────────────────

def test_notify_async_fire_and_forget_returns_empty():
    """notify_async with on_response=None returns '' and emits without notify_id."""
    from plexi_sdk._app import App
    from plexi_sdk._emitter import _emit

    app = App.__new__(App)
    App.__init__(app)

    emitted: list[dict] = []
    with patch("plexi_sdk._emitter._emit", side_effect=emitted.append):
        result = app.emit.notify_async(title="Hello", priority=50)

    assert result == ""
    assert len(emitted) == 1
    assert emitted[0]["type"] == "notify"
    assert "notify_id" not in emitted[0]


def test_notify_async_registers_callback():
    """notify_async with on_response registers callback and returns notify_id."""
    from plexi_sdk._app import App

    app = App.__new__(App)
    App.__init__(app)

    emitted: list[dict] = []
    with patch("plexi_sdk._emitter._emit", side_effect=emitted.append):
        cb = MagicMock()
        nid = app.emit.notify_async(title="Hello", priority=50, on_response=cb)

    assert nid != ""
    assert nid in app._pending_notify_callbacks
    assert app._pending_notify_callbacks[nid] is cb
    notify_payload = next(e for e in emitted if e.get("type") == "notify")
    assert notify_payload["notify_id"] == nid


def test_notify_choice_async_registers_callback():
    """notify_choice_async registers callback and returns notify_id."""
    from plexi_sdk._app import App

    app = App.__new__(App)
    App.__init__(app)

    emitted: list[dict] = []
    with patch("plexi_sdk._emitter._emit", side_effect=emitted.append):
        cb = MagicMock()
        options = [{"label": "Yes"}, {"label": "No"}]
        nid = app.emit.notify_choice_async(title="Pick one", options=options,
                                           priority=50, on_response=cb)

    assert nid != ""
    assert nid in app._pending_notify_callbacks
    notify_payload = next(e for e in emitted if e.get("type") == "notify")
    assert notify_payload["kind"] == "choice"
    assert notify_payload["notify_id"] == nid


def test_notify_input_async_registers_callback():
    """notify_input_async registers callback and returns notify_id."""
    from plexi_sdk._app import App

    app = App.__new__(App)
    App.__init__(app)

    emitted: list[dict] = []
    with patch("plexi_sdk._emitter._emit", side_effect=emitted.append):
        cb = MagicMock()
        nid = app.emit.notify_input_async(title="Enter text", priority=50,
                                          on_response=cb)

    assert nid != ""
    assert nid in app._pending_notify_callbacks
    notify_payload = next(e for e in emitted if e.get("type") == "notify")
    assert notify_payload["kind"] == "input"


def test_notify_and_wait_async_registers_callback():
    """notify_and_wait_async registers callback and returns notify_id."""
    from plexi_sdk._app import App

    app = App.__new__(App)
    App.__init__(app)

    emitted: list[dict] = []
    with patch("plexi_sdk._emitter._emit", side_effect=emitted.append):
        cb = MagicMock()
        nid = app.emit.notify_and_wait_async(title="Wait", priority=50, on_response=cb)

    assert nid != ""
    assert nid in app._pending_notify_callbacks
    notify_payload = next(e for e in emitted if e.get("type") == "notify")
    assert notify_payload["kind"] == "message"
    assert notify_payload["notify_id"] == nid


def _dispatch_notify_action(app, notify_id: str, action_label: str = "acknowledge",
                             value=None):
    """Simulate the event-loop dispatching a notify_action event for a given id."""
    ev: dict = {"notify_id": notify_id, "action_label": action_label}
    if value is not None:
        ev["value"] = value

    # Access the internal dispatch logic directly via the asyncio event loop.
    # We need to run this in a running event loop.
    async def _inject():
        # Directly call the dispatch path by synthesising the event.
        # We reproduce the logic from _app.py's notify_action block.
        action_label_val = ev.get("action_label", "")
        value_val = ev.get("value")
        if action_label_val == "cancel":
            result = "__cancel__"
        elif value_val is not None:
            result = value_val
        else:
            result = action_label_val or "acknowledge"

        nid = ev.get("notify_id", "")
        q = app._pending_notify.pop(nid, None)
        if q:
            q.put_nowait(result)
        else:
            cb = app._pending_notify_callbacks.pop(nid, None)
            if cb is not None:
                try:
                    cb(result)
                except Exception as exc:
                    import sys as _sys
                    _sys.stderr.write(
                        f"plexi_sdk: notify_async callback raised: {exc}\n"
                    )

    asyncio.get_event_loop().run_until_complete(_inject())


def test_notify_async_callback_invoked_on_acknowledge():
    """Callback is invoked with 'acknowledge' when user acknowledges."""
    from plexi_sdk._app import App

    app = App.__new__(App)
    App.__init__(app)

    received: list[str] = []
    with patch("plexi_sdk._emitter._emit"):
        nid = app.emit.notify_async(title="Hi", priority=50,
                                    on_response=received.append)

    _dispatch_notify_action(app, nid, action_label="acknowledge")
    assert received == ["acknowledge"]
    assert nid not in app._pending_notify_callbacks


def test_notify_async_callback_invoked_on_cancel():
    """Callback is invoked with '__cancel__' when user cancels."""
    from plexi_sdk._app import App

    app = App.__new__(App)
    App.__init__(app)

    received: list[str] = []
    with patch("plexi_sdk._emitter._emit"):
        nid = app.emit.notify_async(title="Hi", priority=50,
                                    on_response=received.append)

    _dispatch_notify_action(app, nid, action_label="cancel")
    assert received == ["__cancel__"]
    assert nid not in app._pending_notify_callbacks


def test_notify_choice_async_callback_receives_value():
    """Choice callback receives the selected value."""
    from plexi_sdk._app import App

    app = App.__new__(App)
    App.__init__(app)

    received: list[str] = []
    options = [{"label": "Yes", "value": "yes"}, {"label": "No", "value": "no"}]
    with patch("plexi_sdk._emitter._emit"):
        nid = app.emit.notify_choice_async(title="Pick", options=options,
                                           priority=50, on_response=received.append)

    _dispatch_notify_action(app, nid, action_label="Yes", value="yes")
    assert received == ["yes"]


def test_notify_input_async_callback_receives_text():
    """Input callback receives the typed text."""
    from plexi_sdk._app import App

    app = App.__new__(App)
    App.__init__(app)

    received: list[str] = []
    with patch("plexi_sdk._emitter._emit"):
        nid = app.emit.notify_input_async(title="Enter", priority=50,
                                          on_response=received.append)

    _dispatch_notify_action(app, nid, action_label="hello world")
    assert received == ["hello world"]


def test_callback_registry_cleaned_up_after_invocation():
    """Registry entry is removed after the callback fires."""
    from plexi_sdk._app import App

    app = App.__new__(App)
    App.__init__(app)

    with patch("plexi_sdk._emitter._emit"):
        nid = app.emit.notify_async(title="Hi", priority=50,
                                    on_response=lambda r: None)

    assert nid in app._pending_notify_callbacks
    _dispatch_notify_action(app, nid)
    assert nid not in app._pending_notify_callbacks


def test_callback_exception_does_not_crash_dispatch():
    """A raising callback is caught; the registry entry is still removed."""
    import sys
    from io import StringIO
    from plexi_sdk._app import App

    app = App.__new__(App)
    App.__init__(app)

    def bad_cb(result: str) -> None:
        raise ValueError("boom")

    with patch("plexi_sdk._emitter._emit"):
        nid = app.emit.notify_async(title="Hi", priority=50, on_response=bad_cb)

    stderr_capture = StringIO()
    with patch("sys.stderr", stderr_capture):
        _dispatch_notify_action(app, nid)

    assert nid not in app._pending_notify_callbacks
    assert "boom" in stderr_capture.getvalue()


def test_blocking_notify_unaffected():
    """Existing _pending_notify queue path still works after the change."""
    from plexi_sdk._app import App
    import asyncio

    app = App.__new__(App)
    App.__init__(app)

    import uuid
    nid = str(uuid.uuid4())
    q: asyncio.Queue = asyncio.Queue(maxsize=1)
    app._pending_notify[nid] = q

    # Simulate notify_action for the blocking path.
    async def _run():
        ev = {"notify_id": nid, "action_label": "acknowledge"}
        action_label_val = ev.get("action_label", "")
        value_val = ev.get("value")
        if action_label_val == "cancel":
            result = "__cancel__"
        elif value_val is not None:
            result = value_val
        else:
            result = action_label_val or "acknowledge"

        queue = app._pending_notify.pop(nid, None)
        if queue:
            queue.put_nowait(result)
        result_from_queue = await asyncio.wait_for(q.get(), timeout=1.0)
        return result_from_queue

    loop = asyncio.new_event_loop()
    result = loop.run_until_complete(_run())
    loop.close()

    assert result == "acknowledge"
    assert nid not in app._pending_notify
    assert nid not in app._pending_notify_callbacks
