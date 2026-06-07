"""Tests for on_init lifecycle hook."""

import asyncio
import json
import io
from unittest.mock import patch

from plexi_sdk import App
from plexi_sdk.ui import Column, AppBar

INIT_EV = {
    "type": "init", "protocol": "pgap/3.", "app_id": "t",
    "workspace_root": "/tmp", "capabilities": [], "feature_flags": [],
    "args": [], "theme": {}
}


def _drive(app):
    lines = "\n".join(json.dumps(e) for e in [INIT_EV, {"type": "shutdown"}]) + "\n"
    with patch("sys.stdin", io.StringIO(lines)):
        with patch("plexi_sdk._app._emit"):
            with patch("os._exit"):
                try:
                    asyncio.run(app._async_main())
                except (SystemExit, Exception):
                    pass


def test_on_init_runs():
    """on_init(self) runs and can set state."""
    class MyApp(App):
        def on_init(self):
            self.x = 42

        def view(self):
            return Column([AppBar("Test")])

    app = MyApp()
    _drive(app)
    assert getattr(app, "x", None) == 42, "on_init body should have run"


def test_on_init_async():
    """async def on_init(self) works."""
    ran = []

    class MyApp(App):
        async def on_init(self):
            ran.append(True)

        def view(self):
            return Column([AppBar("Test")])

    _drive(MyApp())
    assert ran
