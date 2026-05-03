# Scrollback Navigation & Copy-Mode Design

**Issues:** #602 (navigation keys), #603 (copy-mode)  
**Replaces:** #192 (closed)  
**Scope:** `deps/egui_term/` only, except the copy-mode entry keybind which is fired from the Plexi host key handler.

---

## Part 1 — Scrollback Navigation Keys (#602)

### Goal

Let users navigate the terminal scrollback buffer entirely from the keyboard. Trackpad scroll already works via `BackendCommand::Scroll(i32)`; this wires the same command to keyboard combos.

### Bindings

| Key | Action | Backend call |
|-----|--------|--------------|
| `Shift+PgUp` | Scroll up one page | `Scroll::Delta(+viewport_lines - 1)` |
| `Shift+PgDn` | Scroll down one page | `Scroll::Delta(-(viewport_lines - 1))` |
| `Cmd+Up` | Scroll up one line | `Scroll::Delta(+1)` |
| `Cmd+Down` | Scroll down one line | `Scroll::Delta(-1)` |
| `Cmd+Home` | Jump to top of scrollback | `Scroll::Top` |
| `Cmd+End` | Jump to bottom | `Scroll::Bottom` |

Bare `PgUp`/`PgDn` (no modifier) continue to send `\x1b[5~`/`\x1b[6~` to the PTY — unchanged, required for vim/less.

### Scroll position indicator

When `display_offset > 0` (viewport is not at the live bottom), paint a dim label in the bottom-right corner of the terminal rect showing how many lines above the bottom the viewport is (e.g. `↑ 42`). Implemented as a single `painter.text(...)` call after the main grid render in `view.rs`. Disappears automatically when `display_offset == 0`.

### Files touched

- `deps/egui_term/src/bindings.rs` — add modifier-gated scroll bindings
- `deps/egui_term/src/view.rs` — scroll indicator render pass; wire new binding actions through the existing `InputAction::BackendCall` path

---

## Part 2 — Copy-Mode (#603)

### Goal

A dedicated keyboard-selection mode modelled on tmux copy-mode. While active, key events are fully intercepted (nothing reaches the PTY) and interpreted as cursor movement and selection commands.

### Entry / exit

| Key | Condition | Effect |
|-----|-----------|--------|
| `Cmd+Shift+[` | Normal mode, not alt-screen | Enter copy-mode; cursor starts at current viewport bottom-left |
| `Esc` / `q` | In copy-mode | Clear selection, exit copy-mode |
| `y` | In copy-mode, selection active | Copy selection to clipboard, exit copy-mode |
| `y` | In copy-mode, no selection | Exit copy-mode (no-op on clipboard) |

**Alt-screen guard:** if `terminal_content.mode.contains(TermMode::ALT_SCREEN)` at entry time, silently ignore `EnterCopyMode`. TUI apps (vim, htop, etc.) own their own cursor and selection.

### State

`TerminalViewState` gains:

```
copy_mode: Option<CopyModeState>
```

```rust
struct CopyModeState {
    cursor: Point,               // alacritty grid Point { line, column }
    selection_start: Option<Point>,
}
```

`CopyModeState` lives in `TerminalViewState` (egui temp memory), keyed to the widget ID — same pattern as `scroll_pixels` and `is_dragged`.

### Navigation (while in copy-mode)

| Key | Action |
|-----|--------|
| `h` / `←` | cursor.column -= 1 (clamp to 0) |
| `l` / `→` | cursor.column += 1 (clamp to cols-1) |
| `k` / `↑` | cursor.line -= 1; scroll viewport if cursor leaves top |
| `j` / `↓` | cursor.line += 1; scroll viewport if cursor leaves bottom |
| `PgUp` | Scroll one page up; cursor follows to stay on screen |
| `PgDn` | Scroll one page down; cursor follows |
| `g` | `Scroll::Top`; cursor moves to top line |
| `G` | `Scroll::Bottom`; cursor moves to bottom line |

### Selection

| Key | Effect |
|-----|--------|
| `v` | `selection_start = Some(cursor)` |
| movement | With `selection_start` set, call `SelectStart(cursor_px)` then `SelectUpdate(cursor_px)` each frame to drive the existing selection highlight |
| `V` | Set `selection_start` to start of cursor's line (line-wise) |
| `Esc` | `selection_start = None`; stay in copy-mode |
| `y` | `selectable_content()` → clipboard; exit copy-mode |

Pixel coordinate conversion (grid → screen):
```
x = rect.min.x + cursor.column * cell_w + cell_w / 2
y = rect.min.y + (cursor.line.0 + display_offset as i32) * cell_h + cell_h / 2
```

### Input intercept

In `view.rs`'s `process_input`, when `copy_mode.is_some()`, all `egui::Event::Key` and `egui::Event::Text` events are consumed and mapped to copy-mode commands before the PTY write path. Nothing reaches `process_keyboard_event`.

### Rendering

After the main grid render pass, when `copy_mode.is_some()`:
1. Paint an inverted cell rect at the block cursor position.
2. Paint a dim `[COPY]` badge in the pane name bar (or as a floating label if the pane has no name bar).

### New BackendCommand variants

```rust
BackendCommand::EnterCopyMode
BackendCommand::ExitCopyMode
```

`EnterCopyMode` is dispatched from the Plexi host key handler when the focused pane is a terminal pane and `Cmd+Shift+[` fires. `ExitCopyMode` is dispatched on the same shortcut pressed again, or via in-mode `Esc`/`q`/`y`.

### Files touched

- `deps/egui_term/src/backend/mod.rs` — add `EnterCopyMode` / `ExitCopyMode` to `BackendCommand`; handle them in `process_command`
- `deps/egui_term/src/view.rs` — `TerminalViewState` fields; input intercept; block cursor render; `[COPY]` badge
- `src/keys.rs` (Plexi host) — add `Cmd+Shift+[` binding → dispatch `EnterCopyMode` to focused terminal backend

---

## Out of scope (both issues)

- Full vim motions (`w`, `b`, `e`, `f{char}`, counts)
- Regex / incremental search (separate future issue)
- Block selection (`Ctrl+V`)
- Scrollback persistence across restarts

## Sequencing

Ship #602 first — it's a standalone PR with no state machine. #603 depends on #602 (reuses the same scroll path internally) but can be developed in parallel once the scroll bindings are confirmed working.
