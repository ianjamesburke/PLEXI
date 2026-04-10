# hello-app

Example Plexi app demonstrating the draw protocol.

## Install

```bash
mkdir -p ~/.plexi/apps/hello-app/bin
cp manifest.toml ~/.plexi/apps/hello-app/
cp bin/plexi-app ~/.plexi/apps/hello-app/bin/
chmod +x ~/.plexi/apps/hello-app/bin/plexi-app
```

Then restart Plexi and press `Cmd+E` to open the file browser, or run an app that
opens `.hello` files.

## How it works

Plexi spawns the binary and communicates via newline-delimited JSON on stdin/stdout.

**Plexi → App** (events):
- `{"type": "init", "width": 800, "height": 600, "pixels_per_point": 2.0}`
- `{"type": "render", "width": 800, "height": 600}`
- `{"type": "key", "key": "ArrowDown", "modifiers": {}}`
- `{"type": "command", "text": "cd /foo"}`
- `{"type": "shutdown"}`

**App → Plexi** (draw commands):
- `{"type": "rect", "x": 0, "y": 0, "w": 400, "h": 300, "fill": "#1e1e2e"}`
- `{"type": "text", "x": 20, "y": 20, "text": "Hello!", "size": 14.0, "color": "#cdd6f4"}`
- `{"type": "list", "items": [...], "selected": 0, "item_height": 28.0}`
- `{"type": "run_in_terminal", "command": "ls -la"}`
- `{"type": "frame_done"}`

The app can be written in any language. See `bin/plexi-app` for a Python example.
