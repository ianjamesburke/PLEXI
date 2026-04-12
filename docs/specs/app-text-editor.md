# File Explorer + Text Editor Integration

**Status:** Spec  
**Last updated:** 2026-04-11  
**Depends on:** Text editor primitive, existing file browser app  
**App type:** Out-of-process (Python) or built-in

---

## Summary

When you click a file in a file explorer pane, it opens in a text editor pane as a right split. The text editor is a standalone Plexi app that uses the text editor primitive to display and edit file contents. This proves two things: the text editor primitive works as a real app surface, and Plexi's pane system can support coordinated app-to-app workflows.

---

## How It Works

### Flow

1. User has a file browser app open in a pane.
2. User selects a file (e.g., `utils.py`) and presses Enter (or double-clicks).
3. The file browser emits an open command: `{"type": "open_file", "path": "/abs/path/to/utils.py"}`.
4. Plexi receives this, opens a new pane (right split), and launches the text editor app with the file path as a launch argument.
5. The text editor app reads the file, renders it in a `text_editor` primitive in code mode (language auto-detected from extension), and handles edits.
6. On save (Cmd+S), the text editor writes the file back to disk.

### The Text Editor App

A minimal app (~100 lines) whose entire job is:

1. Read a file path from launch args.
2. Read the file content.
3. Render a full-pane `text_editor` primitive.
4. On `text_editor_changed`, update internal content state.
5. On Cmd+S (key event or `text_editor_submit` if we map it), write content to disk via `WriteFile`.
6. Show a subtle "saved" indicator that fades after 1 second.

```python
from plexi_sdk import App

app = App()
file_path = sys.argv[1]
content = ""  # loaded on init
modified = False
saved_flash = 0

@app.on_init
def init(ctx):
    global content
    result = ctx.read_file(file_path)
    content = result

@app.on_render
def render(ctx):
    # Title bar
    ctx.rect(0, 0, ctx.width, 32, fill="#181825")
    basename = file_path.split("/")[-1]
    label = f"{'● ' if modified else ''}{basename}"
    ctx.text(12, 8, label, size=14, color="#cdd6f4", bold=True)

    # Editor fills the rest
    ctx.text_editor(
        id="main",
        x=0, y=32, w=ctx.width, h=ctx.height - 32,
        content=content,
        config={
            "mode": "code",
            "language": detect_language(file_path),
            "line_numbers": True,
        },
        focused=True,
    )
```

### Language Detection

Simple extension mapping:

| Extension | Language |
|-----------|----------|
| `.py` | `python` |
| `.rs` | `rust` |
| `.js`, `.mjs` | `javascript` |
| `.ts`, `.tsx` | `typescript` |
| `.json` | `json` |
| `.toml` | `toml` |
| `.md` | `markdown` |
| `.sh`, `.bash`, `.zsh` | `bash` |
| `.yaml`, `.yml` | `yaml` |
| `.html` | `html` |
| `.css` | `css` |
| anything else | `plain` |

---

## App-to-App Communication

This spec surfaces a question: how does the file browser tell Plexi to open a file in a new pane?

### Option A: Plexi Command (Recommended for MVP)

The file browser emits a Plexi-level command:

```json
{"type": "open_app", "app_id": "text-editor", "args": ["/path/to/file.py"], "split": "right"}
```

Plexi handles the pane split and app launch. The file browser doesn't need to know about the text editor app — it just says "open this file" and Plexi routes it.

This is the clean approach: Plexi is the coordinator. Apps don't talk directly to each other.

### Option B: Direct IPC (Future)

Apps can send messages to other running apps via Plexi as a broker:

```json
{"type": "send_to_app", "target": "text-editor", "message": {"action": "open", "path": "/path/to/file.py"}}
```

More flexible but more complex. Not needed for MVP.

---

## Manifest

```toml
[app]
id = "text-editor"
name = "Text Editor"
version = "0.1.0"
description = "Edit text and code files"

[capabilities]
filesystem = "read_write"

[app.handles]
file_types = ["*"]  # default handler for opening any file
```

The `handles.file_types = ["*"]` declaration tells Plexi this app can open any file type. When another app emits an `open_file` command, Plexi checks for a handler and routes to this app.

---

## MVP Scope

1. **Text editor app** — reads a file, renders via `text_editor` primitive, saves on Cmd+S.
2. **`open_app` command** — file browser (or any app) can request Plexi open another app in a split.
3. **Language detection** — auto-detect from file extension, pass to `text_editor` config.
4. **Modified indicator** — dot in title bar when content differs from last save.

**Defer:** Multiple open files (tabs), find/replace, go-to-line, file tree sidebar, unsaved changes warning on close.

---

## Integration with PyFlow

PyFlow's "Open .py" toolbar button and "View Source" context menu item both emit `open_app` for the text editor. This means a user can:

1. Work visually in PyFlow for function wiring and signatures.
2. Pop open the raw `.py` in the text editor for full-file editing.
3. PyFlow detects the external change and updates its canvas.

Two views of the same file, each good at different things. The file on disk is the shared source of truth.
