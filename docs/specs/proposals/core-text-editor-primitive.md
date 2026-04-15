# Text Editor Primitive

**Status:** Spec  
**Last updated:** 2026-04-11  
**Depends on:** App infrastructure (Phase 2+), draw protocol  
**Blocks:** PyFlow, file explorer, any app needing text input beyond single-line

---

## Summary

A native text editor widget rendered by Plexi's Rust/egui renderer, exposed to apps as a single draw command. The app says "put a text editor here with this content" — Plexi handles cursor, selection, scrolling, line numbers, and optionally syntax highlighting. Edit events flow back to the app as structured JSON.

This is the equivalent of egui's `TextEdit` but exposed over the draw protocol. Apps never manually render cursor blinking, text selection rectangles, or scroll offsets. Plexi owns all of that.

---

## Why This Is a Primitive

Every app that needs multi-line text input would otherwise need to reimplement:
- Cursor positioning and movement (arrow keys, Home/End, Cmd+Left/Right)
- Text selection (Shift+arrow, Shift+Cmd+arrow, double-click word select, triple-click line select)
- Scrolling (vertical, horizontal for long lines)
- Clipboard (Cmd+C/V/X)
- Undo/redo within the editor
- Line wrapping or horizontal scroll
- IME support for non-ASCII input

Building this once in Rust (backed by egui's existing text primitives or a purpose-built text buffer) and exposing it as a draw command means every app gets a good text editor for free.

---

## Draw Command

### App → Plexi

```json
{
  "type": "text_editor",
  "id": "fn_body_editor",
  "x": 40,
  "y": 100,
  "w": 600,
  "h": 400,
  "content": "def greet(name: str) -> str:\n    return f\"Hello, {name}\"",
  "cursor_pos": 42,
  "selection": null,
  "config": {
    "mode": "code",
    "language": "python",
    "line_numbers": true,
    "read_only": false,
    "tab_size": 4,
    "word_wrap": false,
    "placeholder": "Enter code here..."
  }
}
```

**Required fields:**

| Field | Type | Description |
|-------|------|-------------|
| `id` | string | Unique identifier for this editor instance within the app. Plexi uses this to maintain internal state (cursor blink phase, scroll offset, undo history) across frames. |
| `x, y, w, h` | number | Bounding box in logical pixels. |
| `content` | string | The full text content. The app is the source of truth — Plexi renders this and reports edits back. |

**Optional fields:**

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `cursor_pos` | number | end of content | Byte offset for cursor position. App can set this to move cursor programmatically. |
| `selection` | `[start, end]` or null | null | Byte offset range for selection highlight. |
| `config.mode` | `"plain"` / `"code"` | `"plain"` | Code mode enables syntax highlighting and monospace font. |
| `config.language` | string | none | Language identifier for syntax highlighting (e.g., `"python"`, `"rust"`, `"json"`, `"toml"`). Ignored if mode is `"plain"`. |
| `config.line_numbers` | bool | false | Show line number gutter. |
| `config.read_only` | bool | false | Disables editing; still allows selection and copy. |
| `config.tab_size` | number | 4 | Spaces per tab. |
| `config.word_wrap` | bool | true in plain, false in code | Wrap long lines vs. horizontal scroll. |
| `config.placeholder` | string | none | Greyed-out placeholder text when content is empty. |
| `config.font_size` | number | 14 | Font size in logical pixels. |
| `config.highlight_line` | bool | true in code mode | Subtle highlight on the line containing the cursor. |

### Plexi → App (Edit Events)

When the user edits text inside a `text_editor` widget, Plexi sends events back to the app:

```json
{
  "type": "text_editor_changed",
  "id": "fn_body_editor",
  "content": "def greet(name: str) -> str:\n    return f\"Hello, {name}!\"",
  "cursor_pos": 56,
  "selection": null
}
```

This fires on every meaningful change (keystroke, paste, cut, undo/redo). The app receives the full updated content plus cursor state.

**Additional events:**

| Event | Fields | When |
|-------|--------|------|
| `text_editor_changed` | `id, content, cursor_pos, selection` | Any content change |
| `text_editor_focus` | `id` | Editor gains focus (user clicked in or tabbed to it) |
| `text_editor_blur` | `id` | Editor loses focus |
| `text_editor_submit` | `id, content` | User presses Cmd+Enter (configurable "submit" action) |
| `text_editor_escape` | `id` | User presses Escape while editor is focused |

---

## Internal State (Plexi-side)

Plexi maintains per-editor-instance state keyed by `id`:

- **Scroll offset** — vertical and horizontal scroll position, preserved across frames
- **Cursor blink phase** — cosmetic, resets on keystroke
- **Undo/redo stack** — internal to the editor, not exposed to the app. The app can implement its own higher-level undo on top of the content snapshots it receives.
- **Selection state** — while dragging, Plexi tracks the anchor point internally
- **IME composition** — handled entirely by Plexi, committed text appears in `text_editor_changed`

When the app sends a `text_editor` command with the same `id` but different `content`, Plexi detects the external content change and resets the undo stack (the app took over). If `content` matches what Plexi already has (i.e., the app is echoing back what Plexi sent), Plexi preserves internal state.

---

## Syntax Highlighting

Built into Plexi's renderer. Uses a lightweight tokenizer (not a full language server).

**MVP languages:** Python, Rust, JSON, TOML, Markdown, JavaScript/TypeScript, shell/bash.

Tokenization approach: regex-based lexer per language (similar to tree-sitter's highlight queries but simpler). Tokens map to a fixed palette of semantic colors:

| Token type | Example |
|------------|---------|
| `keyword` | `def`, `return`, `if`, `fn`, `let` |
| `string` | `"hello"`, `f"..."`, `r#"..."#` |
| `number` | `42`, `3.14`, `0xFF` |
| `comment` | `# ...`, `// ...` |
| `type` | `str`, `int`, `Vec`, `Option` |
| `function` | function names in definitions and calls |
| `operator` | `+`, `->`, `=>`, `::` |
| `punctuation` | `(`, `)`, `{`, `}` |
| `variable` | everything else |

Colors are derived from the app's theme (or Plexi's global theme). The editor doesn't hardcode colors — it maps token types to theme slots.

**Future:** tree-sitter integration for accurate parsing. The draw command doesn't change — only the internal tokenizer improves.

---

## Keyboard Handling

When a `text_editor` has focus, Plexi intercepts all key events and handles them internally. The app does NOT receive `key` events for keystrokes consumed by the editor.

**Standard key bindings (macOS-native):**

| Action | Key |
|--------|-----|
| Move cursor | Arrow keys |
| Word jump | Option+Left/Right |
| Line start/end | Cmd+Left/Right, Home/End |
| Select | Shift+any movement key |
| Select all | Cmd+A |
| Copy/Cut/Paste | Cmd+C/X/V |
| Undo/Redo | Cmd+Z / Cmd+Shift+Z |
| Delete word | Option+Backspace |
| Delete line | Cmd+Backspace |
| New line | Enter |
| Indent/Dedent (code mode) | Tab / Shift+Tab |
| Submit | Cmd+Enter (sends `text_editor_submit` event) |
| Exit editor | Escape (sends `text_editor_escape` event) |

The app can override the submit/escape behavior by not handling those events — but the default is that Escape returns focus to the app's general key handler.

---

## Focus Model

- Only one `text_editor` can be focused at a time within a pane.
- Clicking inside an editor focuses it. Clicking outside blurs it.
- Tab key within a focused code editor inserts indentation. Tab to move between editors requires the app to implement focus cycling via `text_editor_escape` → programmatic focus.
- When no editor is focused, key events go to the app's `on_key` handler as normal.

An app can programmatically focus an editor by including `"focused": true` in the draw command:

```json
{
  "type": "text_editor",
  "id": "fn_body_editor",
  "focused": true,
  ...
}
```

---

## Modes

### Plain Mode (`"mode": "plain"`)

- Proportional font (system default or Plexi's configured font)
- Word wrap on by default
- No line numbers
- Good for: notes, descriptions, commit messages, chat input

### Code Mode (`"mode": "code"`)

- Monospace font
- Syntax highlighting (if `language` is set)
- Line numbers on by default
- No word wrap (horizontal scroll instead)
- Tab inserts spaces (per `tab_size`)
- Auto-indent on Enter (matches previous line's indentation)
- Bracket/quote auto-close (configurable)
- Good for: code editors, config files, scripts

---

## MVP Scope

Ship in this order:

1. **Plain text editor** — rect, monospace text rendering, cursor, basic editing (insert/delete/arrow keys), clipboard, scroll. No syntax highlighting, no line numbers.
2. **Code mode** — monospace, line numbers, horizontal scroll, tab handling.
3. **Syntax highlighting** — Python first (PyFlow needs it), then Rust/JSON/TOML.
4. **Selection** — click-drag, shift-arrow, double-click word select.
5. **Undo/redo** — internal stack, Cmd+Z/Shift+Cmd+Z.

Each phase is independently useful. Phase 1 alone unblocks a huge number of apps.

---

## Future Extensions (Not in MVP)

- **Autocomplete dropdown** — app sends `{"type": "text_editor", ..., "completions": [{"label": "print", "detail": "builtin"}, ...]}` and Plexi renders the dropdown at cursor position. App computes completions; Plexi renders them.
- **Inline diagnostics** — app sends `{"type": "text_editor", ..., "diagnostics": [{"line": 3, "col": 10, "message": "undefined name", "severity": "error"}]}` and Plexi renders squiggly underlines + hover tooltips.
- **Minimap** — condensed overview for long files.
- **Multi-cursor** — Cmd+D style editing.
- **Language server bridge** — Plexi manages an LSP process, routes completions/diagnostics to the editor. The app just sets `"language": "python"` and gets full IDE features.
