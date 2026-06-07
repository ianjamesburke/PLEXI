"""Tests for ctx-free on_init — v2 apps use def on_init(self) with no ctx param."""

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


def test_on_init_without_ctx_does_not_raise():
    """on_init(self) with no ctx param must not cause TypeError."""
    class MyApp(App):
        def on_init(self):
            self.x = 42

        def view(self):
            return Column([AppBar("Test")])

    app = MyApp()
    _drive(app)
    assert getattr(app, "x", None) == 42, "on_init body should have run"


def test_on_init_with_ctx_still_works():
    """on_init(self, ctx) old style must keep working."""
    ctx_types = []

    class MyApp(App):
        async def on_init(self, ctx):
            ctx_types.append(type(ctx).__name__)

        def view(self):
            return Column([AppBar("Test")])

    _drive(MyApp())
    assert ctx_types == ["RenderContext"]


def test_on_init_async_without_ctx():
    """async def on_init(self) with no ctx param."""
    ran = []

    class MyApp(App):
        async def on_init(self):
            ran.append(True)

        def view(self):
            return Column([AppBar("Test")])

    _drive(MyApp())
    assert ran
