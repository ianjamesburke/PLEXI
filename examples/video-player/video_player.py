#!/usr/bin/env python3
"""Video Player — video.playback + VideoPlayer draw command for PGAP v3.

Usage: python video_player.py [/path/to/video.mp4]
Space = play/pause. Left/right = seek ±5s.
"""
from __future__ import annotations

import sys
import os
sys.path.insert(0, os.path.dirname(__file__))

from plexi_sdk import App, RenderContext, BG, FG, MUTED, ACCENT, SURFACE, HINT

DEFAULT_SOURCE = os.environ.get("PLEXI_VIDEO_FIXTURE", "")


class VideoPlayerApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        args = sys.argv[1:]
        self._source = args[0] if args else DEFAULT_SOURCE
        self._playing = False
        self._position_ms = 0
        self._seek_pending: int | None = None  # ms to seek to

    def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
        if not self._source:
            return
        if key == " ":
            self._playing = not self._playing
            self._seek_pending = None
        elif key == "ArrowLeft":
            self._position_ms = max(0, self._position_ms - 5000)
            self._seek_pending = self._position_ms
        elif key == "ArrowRight":
            self._position_ms += 5000
            self._seek_pending = self._position_ms

    def _state_str(self) -> str:
        if self._seek_pending is not None:
            s = f"seek:{self._seek_pending}"
            self._seek_pending = None
            return s
        return "play" if self._playing else "pause"

    def on_render(self, ctx: RenderContext) -> None:
        ctx.rect(0, 0, ctx.w, ctx.h, fill=BG)

        if not self._source:
            ctx.text(16, ctx.h / 2 - 10, "No video source. Pass path as argv[1].",
                     size=14.0, color=MUTED)
            return

        # VideoPlayer fills entire pane (minus status bar at bottom)
        bar_h = 36.0
        ctx.video_player(
            source=self._source,
            x=0, y=0,
            w=ctx.w, h=ctx.h - bar_h,
            state=self._state_str(),
        )

        # Status bar
        ctx.rect(0, ctx.h - bar_h, ctx.w, bar_h, fill=SURFACE)
        name = os.path.basename(self._source)
        status = "▶" if self._playing else "❚❚"
        ctx.text(16, ctx.h - bar_h + 10,
                 f"{status}  {name}", size=13.0, color=FG)
        ctx.text(ctx.w - 200, ctx.h - bar_h + 10,
                 "Space play/pause  ◀ ▶ ±5s", size=HINT, color=MUTED)


if __name__ == "__main__":
    VideoPlayerApp().run()
