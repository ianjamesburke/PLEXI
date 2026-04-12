# hello-app

Example Plexi app demonstrating the draw protocol, including the media
primitives (`image`, `video_thumbnail`, `file_grid`).

## Install

```bash
mkdir -p ~/.plexi-alpha/apps/hello-app/media
cp manifest.toml hello_app.py plexi_sdk.py ~/.plexi-alpha/apps/hello-app/
cp media/*.png ~/.plexi-alpha/apps/hello-app/media/
```

(Use `~/.plexi/apps/` if you're on the stable build.)

Then restart Plexi and open the hello-app from the command palette, or open a
file ending in `.hello`.

Use Tab / ← / → to cycle between the four demo tabs:
- **Image** — four sample PNGs showing `contain`, `cover`, `fill`, and a
  missing-file error placeholder
- **Video** — a `video_thumbnail` of the first video found in `media/`
  (drop in any `.mp4`/`.mov` to try it)
- **File Grid** — `file_grid` in directory mode, filtered to image + video
  extensions
- **All** — stacks the three above

## How it works

Plexi spawns `hello_app.py` and communicates via newline-delimited JSON on
stdin/stdout. `hello_app.py` uses the Python SDK (`plexi_sdk.py` alongside it)
for the render context and event loop.

**Plexi → App** (events):
- `{"type": "init", "width": 800, "height": 600, "pixels_per_point": 2.0}`
- `{"type": "render", "width": 800, "height": 600}`
- `{"type": "key", "key": "ArrowDown", "modifiers": {}}`
- `{"type": "command", "text": "cd /foo"}`
- `{"type": "shutdown"}`

**App → Plexi** (draw commands):
- `{"type": "rect", "x": 0, "y": 0, "w": 400, "h": 300, "fill": "#1e1e2e"}`
- `{"type": "text", "x": 20, "y": 20, "text": "Hello!", "size": 14.0, "color": "#cdd6f4"}`
- `{"type": "image", "path": "media/red.png", "x": 20, "y": 80, "w": 160, "h": 160, "fit": "contain"}`
- `{"type": "video_thumbnail", "path": "media/clip.mp4", "x": 20, "y": 80, "w": 320, "h": 180}`
- `{"type": "file_grid", "path": "media", "filter": ["*.png", "*.mp4"], "x": 20, "y": 80, "w": 800, "h": 220}`
- `{"type": "frame_done"}`

The legacy raw-JSON entry at `bin/plexi-app` is kept as a language-agnostic
example — any executable that speaks the protocol will work.
