#!/usr/bin/env python3
"""Minimal Plexi app fixture for spiral_viewer tests. Emits one rect + one text."""
from __future__ import annotations

import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", ".."))
from plexi_sdk import App  # noqa: E402

app = App(app_id="mock-target")


@app.on_render
def _render(ctx):
    ctx.rect(10, 10, ctx.width - 20, ctx.height - 20, fill="#ff00aa")
    ctx.text(20, 20, "MOCK", size=14, color="#ffffff")


app.run()
