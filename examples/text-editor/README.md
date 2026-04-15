# text-editor

A pure-Python Plexi external app that mirrors the in-process Rust text editor's
UX through the JSON draw protocol. Proof-of-concept that the same feature set
is buildable in any language that speaks the Plexi protocol.

## Run standalone

```sh
python3 text_editor.py              # start with an empty scratch buffer
python3 text_editor.py file.py      # open an existing file
```

The subprocess speaks JSON-over-stdio; you'll normally launch it from Plexi
(command palette or via file-browser spawn), not at the shell, but running it
raw prints the draw commands to stdout for debugging.

## Install into Plexi

```sh
cp -r /path/to/PLEXI/examples/text-editor ~/.plexi-alpha/apps/
chmod +x ~/.plexi-alpha/apps/text-editor/text_editor.py
```

Then launch from the Plexi command palette: **Text Editor**.

## Dependencies

Standard library plus **[pygments](https://pygments.org/)** for syntax
highlighting (optional — the editor falls back to plain text if pygments is
missing).

```sh
pip install -r requirements.txt
```

## Keybindings

| Shortcut             | Action                             |
|----------------------|------------------------------------|
| `Cmd+F`              | Toggle find bar                    |
| `Cmd+R`              | Toggle find + replace bar          |
| `Cmd+;`              | Toggle goto-line bar               |
| `Cmd+S`              | Save                               |
| `Cmd+Shift+S`        | Save as…                           |
| `Cmd+Z`              | Undo                               |
| `Cmd+Shift+Z`        | Redo                               |
| `Cmd+Shift+L`        | Toggle line numbers                |
| `Cmd+Shift+W`        | Toggle word wrap                   |
| `Arrow keys`         | Move cursor                        |
| `Home` / `End`       | Jump to line start / end           |
| `PageUp` / `PageDown`| Page up / down                     |
| `Tab`                | Insert four spaces                 |
| `Backspace` / `Delete`| Delete char back / forward        |
| `Enter`              | Newline                            |

### Find / Replace bar

| Shortcut     | Action                  |
|--------------|-------------------------|
| `Enter`      | Next match              |
| `Shift+Enter`| Previous match          |
| `Tab`        | Switch find ↔ replace   |
| `Esc`        | Close                   |

Inside the replace field:

| Shortcut     | Action                  |
|--------------|-------------------------|
| `Enter`      | Replace current match   |
| `Shift+Enter`| Replace all matches     |

### Disk-conflict prompt

When the editor detects a file changed on disk while the buffer is dirty:

| Shortcut     | Action                  |
|--------------|-------------------------|
| `R`          | Reload from disk        |
| `K` / `Esc`  | Keep my buffer          |

## Features

- Find / replace with all-match highlighting
- Line numbers gutter
- Syntax highlighting (pygments — 12 built-in languages)
- Undo / redo (200-entry stack, 500ms coalesce window)
- Goto line
- Save / Save As (with `~/` expansion and overwrite prompt)
- Word wrap toggle with smart default per language
- Status bar: path, dirty indicator, line:col, totals, language, transient messages
- Auto-save for unnamed scratch buffers (every 30s or 100 keystrokes)
- File-changed-on-disk detection on focus regain

## Relationship to the in-process editor

Plexi ships with a Rust in-process text editor at `src/text_editor_app.rs`.
This Python app mirrors its behaviour but runs as an external subprocess
speaking the draw protocol. The two coexist: use the in-process one for
fast scratch editing, this one as the "you can build your own editor in any
language" reference.
