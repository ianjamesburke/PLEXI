"""Tests for view() dispatch — declarative apps override view(), canvas uses on_render()."""

import asyncio
import json
import io
from unittest.mock import patch

from plexi_sdk import App
from plexi_sdk.ui import Column, AppBar, Label

INIT_EV = {
    "type": "init", "protocol": "pgap/3.", "app_id": "t",
    "workspace_root": "/tmp", "capabilities": [], "feature_flags": [],
    "args": [], "theme": {}
}
RENDER_EV = {"type": "render", "frame_id": 1, "rect": {"x": 0, "y": 0, "w": 400, "h": 300}}


def _drive(app, extra_events=None):
    events = [INIT_EV] + (extra_events or []) + [{"type": "shutdown"}]
    lines = "\n".join(json.dumps(e) for e in events) + "\n"

    with patch("sys.stdin", io.StringIO(lines)):
        with patch("plexi_sdk._app._emit"):
            with patch("os._exit"):
                try:
                    asyncio.run(app._async_main())
                except (SystemExit, Exception):
                    pass


def test_view_called_on_render_event():
    view_calls = []

    class MyApp(App):
        def view(self):
            view_calls.append(1)
            return Column([AppBar("Test"), Label("hello")])

    _drive(MyApp(), [RENDER_EV])
    assert view_calls, "view() should be called when a Render event arrives"


def test_on_render_fires_when_view_not_overridden():
    render_calls = []

    class MyApp(App):
        def on_render(self, ctx):
            render_calls.append(1)

    _drive(MyApp(), [RENDER_EV])
    assert render_calls, "on_render() should fire when view() is not overridden"


def test_view_overrides_on_render():
    calls = []

    class MyApp(App):
        def view(self):
            calls.append("view")
            return Column([AppBar("Test")])

        def on_render(self, ctx):
            calls.append("on_render")

    _drive(MyApp(), [RENDER_EV])
    assert "view" in calls
    assert "on_render" not in calls, "on_render must NOT fire when view() is overridden"
