#!/usr/bin/env python3
"""
hello-app — minimal Plexi app example.

Demonstrates the three media draw commands:
  - image             — decoded once and cached by Plexi
  - video_thumbnail   — ffmpeg extracts a frame, cached on disk
  - file_grid         — mixed-media directory browser with thumbnails

Also shows the core draw commands (rect/text/list) and key handling.

Layout:
    manifest.toml
    hello_app.py      <-- entry (this file)
    plexi_sdk.py      <-- copied from sdk/python/
    media/
        red.png
        green.png
        blue.png
        purple.png

If you want to see a real video_thumbnail, drop an mp4/mov into media/ — the
file_grid picks it up automatically. Otherwise the video_thumbnail demo points
at the first video it can find and renders an error placeholder if none
exists.
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import App  # noqa: E402

APP_DIR = os.path.dirname(os.path.abspath(__file__))
MEDIA_DIR = os.path.join(APP_DIR, "media")

app = App(app_id="hello-app")

# ─── State ──────────────────────────────────────────────────────────────────

# Tabs let you see each draw command on its own without scrolling.
TABS = ["Image", "Video", "File Grid", "All"]
selected_tab = 3  # start on "All"


def _sample_video() -> str | None:
    """Pick the first video file in media/ (if any) for the video_thumbnail demo."""
    if not os.path.isdir(MEDIA_DIR):
        return None
    for name in sorted(os.listdir(MEDIA_DIR)):
        if name.lower().endswith((".mp4", ".mov", ".webm", ".mkv", ".m4v")):
            return os.path.join(MEDIA_DIR, name)
    return None


# ─── Rendering ──────────────────────────────────────────────────────────────


def draw_header(ctx):
    """Title bar + tab row."""
    ctx.rect(0, 0, ctx.width, 52, fill="#181825")
    ctx.text(20, 12, "Hello App", size=16.0, color="#cdd6f4", bold=True)
    ctx.text(
        20, 30,
        "Tab: switch view   ←/→: cycle   q: quit",
        size=10.0, color="#6c7086",
    )

    tab_w = 96.0
    tab_y = 56.0
    for i, name in enumerate(TABS):
        x = 20.0 + i * (tab_w + 6)
        is_sel = i == selected_tab
        ctx.rect(x, tab_y, tab_w, 26, fill="#313244" if is_sel else "#1e1e2e", radius=4)
        ctx.text(
            x + tab_w / 2 - len(name) * 3, tab_y + 7,
            name, size=12.0,
            color="#cdd6f4" if is_sel else "#6c7086",
        )


def draw_image_demo(ctx, top: float):
    """Four sample PNGs, one per fit mode."""
    fits = ["contain", "cover", "fill"]
    size = min(160.0, (ctx.width - 80) / 4)
    gap = 20.0
    x = 20.0
    ctx.text(x, top, "image — fit modes", size=12.0, color="#cdd6f4", bold=True)
    y = top + 20
    ctx.image(
        os.path.join(MEDIA_DIR, "red.png"),
        x, y, size, size,
        rounding=8.0,
    )
    ctx.text(x, y + size + 4, "contain", size=10.0, color="#6c7086")

    for i, fit in enumerate(["cover", "fill"]):
        xi = x + (size + gap) * (i + 1)
        ctx.image(
            os.path.join(MEDIA_DIR, "green.png" if i == 0 else "blue.png"),
            xi, y, size, size * 0.7,
            fit=fit,
            rounding=8.0,
        )
        ctx.text(xi, y + size * 0.7 + 4, fit, size=10.0, color="#6c7086")

    # Missing file — should render an error placeholder.
    xi = x + (size + gap) * 3
    ctx.image(
        os.path.join(MEDIA_DIR, "does-not-exist.png"),
        xi, y, size, size,
    )
    ctx.text(xi, y + size + 4, "missing", size=10.0, color="#f38ba8")


def draw_video_demo(ctx, top: float):
    """Render a video_thumbnail for a file in media/, if any."""
    x = 20.0
    ctx.text(x, top, "video_thumbnail", size=12.0, color="#cdd6f4", bold=True)
    video = _sample_video()
    y = top + 20
    if video is None:
        ctx.rect(x, y, 320, 180, fill="#313244", radius=6)
        ctx.text(
            x + 20, y + 80,
            "Drop a .mp4/.mov into examples/hello-app/media/",
            size=11.0, color="#a6adc8",
        )
        return
    ctx.video_thumbnail(
        video, x, y, 320, 180,
        show_play_button=True,
        timestamp_seconds=1.0,
    )
    ctx.text(
        x, y + 186,
        f"click to open → {os.path.basename(video)}",
        size=10.0, color="#6c7086",
    )


def draw_file_grid_demo(ctx, top: float):
    """All media/ files in a grid, filtered to images + videos."""
    x = 20.0
    ctx.text(x, top, "file_grid — directory mode", size=12.0, color="#cdd6f4", bold=True)
    ctx.file_grid(
        x, top + 20, ctx.width - 40, 220,
        path=MEDIA_DIR,
        filter=["*.png", "*.jpg", "*.jpeg", "*.mp4", "*.mov", "*.webm"],
        item_size=100.0,
    )


@app.on_render
def render(ctx):
    # Background
    ctx.rect(0, 0, ctx.width, ctx.height, fill="#1e1e2e")
    draw_header(ctx)

    content_top = 96.0
    if selected_tab == 0:
        draw_image_demo(ctx, content_top)
    elif selected_tab == 1:
        draw_video_demo(ctx, content_top)
    elif selected_tab == 2:
        draw_file_grid_demo(ctx, content_top)
    else:
        # "All" stacks the three demos.
        draw_image_demo(ctx, content_top)
        draw_video_demo(ctx, content_top + 220)
        draw_file_grid_demo(ctx, content_top + 440)


@app.on_key
def on_key(key, mods, emit):
    global selected_tab
    if key == "Tab" or key == "ArrowRight":
        selected_tab = (selected_tab + 1) % len(TABS)
    elif key == "ArrowLeft":
        selected_tab = (selected_tab - 1) % len(TABS)
    elif key == "q":
        emit.info("hello-app: quit requested")


if __name__ == "__main__":
    app.run()
