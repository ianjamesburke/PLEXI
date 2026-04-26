#!/usr/bin/env python3
"""Hot Reload Test — POC for #83.

Demonstrates the hot-reload loop: change DISPLAY_STRING below, save the file,
and the app reloads in-place. The frame counter resets to 0 (state loss is
expected — no live state transfer is part of the spec) and the new string
appears.

To exercise this: copy this directory under a workspace's `.plexi/apps/` and
launch Plexi from that workspace. The manifest's `watch = true` only engages
for workspace-local installs, never global.
"""
from __future__ import annotations

import os
import sys

# When running from the source tree, make the bundled SDK importable. When
# running from `~/.plexi-<channel>/apps/...`, PYTHONPATH is set by the host.
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "sdk", "python"))

from plexi_sdk import App, RenderContext, BG, ACCENT, FG, MUTED


# ---------------------------------------------------------------------------
# Edit this string and save — the app reloads automatically.
# ---------------------------------------------------------------------------
DISPLAY_STRING = "Hello, hot reload!"


class HotReloadApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        self._frame_count = 0

    def on_render(self, ctx: RenderContext) -> None:
        self._frame_count += 1

        ctx.rect(0, 0, ctx.w, ctx.h, fill=BG)

        # Header
        ctx.text(20, 24, "Hot Reload Test (#83)", size=18.0, color=ACCENT)
        ctx.text(20, 52, "Edit DISPLAY_STRING and save.", size=12.0, color=MUTED)

        # The watched value — re-renders fresh after every reload.
        ctx.rect(20, 80, ctx.w - 40, 60, fill="#1e1e2e", radius=6.0)
        ctx.text(32, 104, DISPLAY_STRING, size=16.0, color=FG)

        # Frame counter — resets to 0 on every reload, proves state is lost
        # as expected by the spec.
        ctx.text(
            20,
            ctx.h - 32,
            f"frames since last reload: {self._frame_count}",
            size=12.0,
            color=MUTED,
        )

    def on_shutdown(self) -> None:
        # No persistent resources — nothing to clean up.
        pass


if __name__ == "__main__":
    HotReloadApp().run()
