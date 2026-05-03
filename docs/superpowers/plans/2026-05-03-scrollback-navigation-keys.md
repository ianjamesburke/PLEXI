# Scrollback Navigation Keys Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add keyboard shortcuts for navigating the terminal scrollback buffer: `Shift+PgUp/PgDn` (page), `Cmd+Up/Down` (1 line, fix existing 3-line increment), `Cmd+Home/End` (top/bottom), plus a scroll position indicator when not at the bottom.

**Architecture:** All scroll commands route through the existing `BackendCommand::Scroll(i32)` or the new `BackendCommand::ScrollTop`/`ScrollBottom` variants. `Shift+PgUp/PgDn` and `Cmd+Home/End` are handled via new `BindingAction` variants in `egui_term`'s bindings system. `Cmd+Up/Down` are already consumed by the Plexi host and just need their line count fixed from 3 → 1.

**Tech Stack:** Rust, egui, egui_term (vendored at `deps/egui_term/`), alacritty_terminal 0.25 (`Scroll::PageUp`, `Scroll::PageDown`, `Scroll::Top`, `Scroll::Bottom` all confirmed available)

---

## File Map

- **Modify:** `deps/egui_term/src/bindings.rs` — add `BindingAction::ScrollPage(i32)`, `BindingAction::ScrollTop`, `BindingAction::ScrollBottom`; add bindings for `Shift+PgUp/PgDn` (non-alt-screen only) and `Cmd+Home/End`
- **Modify:** `deps/egui_term/src/backend/mod.rs` — add `BackendCommand::ScrollPage(i32)`, `BackendCommand::ScrollTop`, `BackendCommand::ScrollBottom` variants; handle them in `process_command`
- **Modify:** `deps/egui_term/src/view.rs` — handle new `BindingAction` variants in `process_keyboard_key`; add scroll position indicator in `show()`
- **Modify:** `src/pane_ops/layout.rs:617` — change `scroll_focused_pane(3)` caller amounts from 3 to 1

---

## Task 1: Add new BackendCommand variants and handle them

**Files:**
- Modify: `deps/egui_term/src/backend/mod.rs:35-43` (BackendCommand enum)
- Modify: `deps/egui_term/src/backend/mod.rs:240-267` (process_command match)

- [ ] **Step 1: Add the new variants to `BackendCommand`**

In `deps/egui_term/src/backend/mod.rs`, replace:
```rust
#[derive(Debug, Clone)]
pub enum BackendCommand {
    Write(Vec<u8>),
    Scroll(i32),
    Resize(Size, Size),
    SelectStart(SelectionType, f32, f32, f32),
    SelectUpdate(f32, f32, f32),
    ProcessLink(LinkAction, Point),
    MouseReport(MouseButton, Modifiers, Point, bool),
}
```
with:
```rust
#[derive(Debug, Clone)]
pub enum BackendCommand {
    Write(Vec<u8>),
    Scroll(i32),
    ScrollPage(i32),   // +1 = page up, -1 = page down (uses Scroll::PageUp/PageDown)
    ScrollTop,
    ScrollBottom,
    Resize(Size, Size),
    SelectStart(SelectionType, f32, f32, f32),
    SelectUpdate(f32, f32, f32),
    ProcessLink(LinkAction, Point),
    MouseReport(MouseButton, Modifiers, Point, bool),
}
```

- [ ] **Step 2: Handle the new variants in `process_command`**

`process_command` already holds `let mut term = term.lock()` at the top of the function. Add these arms inside the existing `match cmd { ... }`, reusing that `term` binding (same pattern as the existing `Scroll` arm):

```rust
BackendCommand::ScrollPage(sign) => {
    if sign > 0 {
        term.grid_mut().scroll_display(Scroll::PageUp);
    } else {
        term.grid_mut().scroll_display(Scroll::PageDown);
    }
},
BackendCommand::ScrollTop => {
    term.grid_mut().scroll_display(Scroll::Top);
},
BackendCommand::ScrollBottom => {
    term.grid_mut().scroll_display(Scroll::Bottom);
},
```

- [ ] **Step 3: Verify it compiles**

```bash
cd /path/to/worktrees/alpha && cargo build 2>&1 | grep -E "^error"
```
Expected: no `error` lines (warnings OK).

- [ ] **Step 4: Commit**

```bash
git add deps/egui_term/src/backend/mod.rs
git commit -m "feat(egui_term): add ScrollPage/ScrollTop/ScrollBottom BackendCommand variants"
```

---

## Task 2: Add BindingAction variants and keyboard bindings

**Files:**
- Modify: `deps/egui_term/src/bindings.rs:5-12` (BindingAction enum)
- Modify: `deps/egui_term/src/bindings.rs:138-349` (binding tables)

- [ ] **Step 1: Add new variants to `BindingAction`**

In `deps/egui_term/src/bindings.rs`, replace:
```rust
#[derive(Clone, Hash, Debug, PartialEq, Eq)]
pub enum BindingAction {
    Copy,
    Paste,
    Char(char),
    Esc(String),
    LinkOpen,
    Ignore,
}
```
with:
```rust
#[derive(Clone, Hash, Debug, PartialEq, Eq)]
pub enum BindingAction {
    Copy,
    Paste,
    Char(char),
    Esc(String),
    LinkOpen,
    Ignore,
    ScrollPage(i32),  // +1 = page up, -1 = page down
    ScrollTop,
    ScrollBottom,
}
```

- [ ] **Step 2: Write a failing test**

At the bottom of `deps/egui_term/src/bindings.rs`, add:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::TerminalMode;

    fn normal_mode() -> TerminalMode {
        TerminalMode::empty()
    }

    fn alt_screen_mode() -> TerminalMode {
        TerminalMode::ALT_SCREEN
    }

    fn shift() -> Modifiers {
        Modifiers::SHIFT
    }

    #[test]
    fn shift_pageup_scrolls_in_normal_mode() {
        let layout = BindingsLayout::new();
        let action = layout.get_action(
            InputKind::KeyCode(Key::PageUp),
            shift(),
            normal_mode(),
        );
        assert_eq!(action, BindingAction::ScrollPage(1));
    }

    #[test]
    fn shift_pageup_sends_escape_in_alt_screen() {
        let layout = BindingsLayout::new();
        let action = layout.get_action(
            InputKind::KeyCode(Key::PageUp),
            shift(),
            alt_screen_mode(),
        );
        assert_eq!(action, BindingAction::Esc("\x1b[5;2~".into()));
    }

    #[test]
    fn shift_pagedown_scrolls_in_normal_mode() {
        let layout = BindingsLayout::new();
        let action = layout.get_action(
            InputKind::KeyCode(Key::PageDown),
            shift(),
            normal_mode(),
        );
        assert_eq!(action, BindingAction::ScrollPage(-1));
    }
}
```

- [ ] **Step 3: Run test to confirm it fails**

```bash
cd /path/to/worktrees/alpha && cargo test -p egui_term 2>&1 | grep -E "FAILED|error"
```
Expected: `shift_pageup_scrolls_in_normal_mode` FAILED (BindingAction::Ignore returned, not ScrollPage).

- [ ] **Step 4: Add the scroll bindings**

In `default_keyboard_bindings()` in `bindings.rs`, after the existing `Shift+PgUp` alt-screen entry (line ~245), add the non-alt-screen scroll variants. The binding table uses `~` for "mode must NOT be set". Add after the existing `PageDown, Modifiers::SHIFT, +TerminalMode::ALT_SCREEN` line:

```rust
// Scrollback navigation — only when NOT in alt-screen (TUI apps get the escape seq above)
PageUp,   Modifiers::SHIFT, ~TerminalMode::ALT_SCREEN; BindingAction::ScrollPage(1);
PageDown, Modifiers::SHIFT, ~TerminalMode::ALT_SCREEN; BindingAction::ScrollPage(-1);
```

And for Home/End with Cmd — these use `Modifiers::COMMAND` (= Cmd on macOS). Add in `platform_keyboard_bindings()` (macOS section) after the Copy/Paste entries:
```rust
Home, Modifiers::MAC_CMD; BindingAction::ScrollTop;
End,  Modifiers::MAC_CMD; BindingAction::ScrollBottom;
```

Note: use `Modifiers::MAC_CMD` (not `Modifiers::COMMAND`) to avoid conflicting with the Ctrl+Home binding in the default table. On macOS, `MAC_CMD` is Cmd, `COMMAND` is also Cmd — but `MAC_CMD` is more specific and avoids the general COMMAND modifier collision with Ctrl bindings on Linux.

Actually verify which modifier constant egui uses for Cmd-only on macOS by checking the existing Copy binding:
```
C, Modifiers::MAC_CMD; BindingAction::Copy;
```
Use the same constant: `Modifiers::MAC_CMD`.

- [ ] **Step 5: Run test to confirm it passes**

```bash
cd /path/to/worktrees/alpha && cargo test -p egui_term 2>&1 | grep -E "test.*ok|FAILED"
```
Expected: all three new tests pass.

- [ ] **Step 6: Compile check**

```bash
cargo build 2>&1 | grep "^error"
```

- [ ] **Step 7: Commit**

```bash
git add deps/egui_term/src/bindings.rs
git commit -m "feat(egui_term): add ScrollPage/ScrollTop/ScrollBottom BindingAction variants and bindings"
```

---

## Task 3: Wire BindingAction → BackendCommand in view.rs

**Files:**
- Modify: `deps/egui_term/src/view.rs:621-633` (`process_keyboard_key` match)

- [ ] **Step 1: Handle the new BindingAction variants in `process_keyboard_key`**

In `deps/egui_term/src/view.rs`, find `process_keyboard_key` and its match block:
```rust
match binding_action {
    BindingAction::Char(c) => { ... },
    BindingAction::Esc(seq) => { ... },
    _ => InputAction::Ignore,
}
```

Replace the `_ => InputAction::Ignore` arm with explicit handling plus a final catch-all:
```rust
    BindingAction::ScrollPage(sign) => {
        InputAction::BackendCall(BackendCommand::ScrollPage(sign))
    },
    BindingAction::ScrollTop => {
        InputAction::BackendCall(BackendCommand::ScrollTop)
    },
    BindingAction::ScrollBottom => {
        InputAction::BackendCall(BackendCommand::ScrollBottom)
    },
    _ => InputAction::Ignore,
```

- [ ] **Step 2: Compile and verify**

```bash
cargo build 2>&1 | grep "^error"
```
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add deps/egui_term/src/view.rs
git commit -m "feat(egui_term): wire ScrollPage/ScrollTop/ScrollBottom through process_keyboard_key"
```

---

## Task 4: Fix Cmd+Up/Down line increment (host side)

**Files:**
- Modify: `src/app/mod.rs` — change the `scroll_focused_pane` call amounts

Currently `Action::ScrollUp` calls `self.scroll_focused_pane(3)` and `Action::ScrollDown` calls `self.scroll_focused_pane(-3)`. These scroll 3 lines per keypress, which is too coarse for line-by-line navigation.

- [ ] **Step 1: Find the call sites**

```bash
grep -n "scroll_focused_pane" src/app/mod.rs
```
Expected output: two lines around 1934-1938 with `scroll_focused_pane(3)` and `scroll_focused_pane(-3)`.

- [ ] **Step 2: Change to 1 line**

In `src/app/mod.rs`, change:
```rust
Action::ScrollUp => {
    self.scroll_focused_pane(3);
}
Action::ScrollDown => {
    self.scroll_focused_pane(-3);
}
```
to:
```rust
Action::ScrollUp => {
    self.scroll_focused_pane(1);
}
Action::ScrollDown => {
    self.scroll_focused_pane(-1);
}
```

- [ ] **Step 3: Compile and verify**

```bash
cargo build 2>&1 | grep "^error"
```

- [ ] **Step 4: Commit**

```bash
git add src/app/mod.rs
git commit -m "fix(scrollback): Cmd+Up/Down scrolls 1 line instead of 3"
```

---

## Task 5: Add scroll position indicator

**Files:**
- Modify: `deps/egui_term/src/view.rs` — `show()` method, after `painter.extend(shapes)`

- [ ] **Step 1: Add the indicator after `painter.extend(shapes)`**

In `deps/egui_term/src/view.rs`, find `painter.extend(shapes);` in `show()`. After that line, add:

```rust
// Scroll position indicator: dim "↑ N" in bottom-right when not at live bottom.
let display_offset = content.grid.display_offset();
if display_offset > 0 {
    let label = format!("↑ {}", display_offset);
    let font = egui::FontId::monospace(11.0);
    let color = Color32::from_rgba_unmultiplied(255, 255, 255, 80);
    let galley = painter.layout_no_wrap(label, font, color);
    let pad = 6.0;
    let pos = egui::Pos2::new(
        layout_max.x - galley.size().x - pad,
        layout_max.y - galley.size().y - pad,
    );
    painter.galley(pos, galley, color);
}
```

- [ ] **Step 2: Compile**

```bash
cargo build 2>&1 | grep "^error"
```

- [ ] **Step 3: Commit**

```bash
git add deps/egui_term/src/view.rs
git commit -m "feat(egui_term): scroll position indicator when viewport is in scrollback"
```

---

## Task 6: Manual smoke test

- [ ] **Step 1: Build and install the alpha build**

From inside `worktrees/alpha/`:
```bash
just bump-and-install
```

- [ ] **Step 2: Verify each binding**

Open Plexi Alpha with a terminal pane. Run `git log` or `seq 1 200` to populate scrollback. Then verify:

| Action | Expected |
|--------|----------|
| `Shift+PgUp` | Viewport scrolls up one full page |
| `Shift+PgDn` | Viewport scrolls down one full page |
| `Cmd+Up` | Viewport scrolls up exactly one line |
| `Cmd+Down` | Viewport scrolls down exactly one line |
| `Cmd+Home` (Fn+←) | Jumps to top of scrollback |
| `Cmd+End` (Fn+→) | Jumps back to live bottom |
| Scroll indicator | "↑ N" appears dim in bottom-right when scrolled up; disappears at bottom |

- [ ] **Step 3: Verify alt-screen guard**

Open `vim` in the terminal. With vim open, press `Shift+PgUp`. Expected: vim processes the keypress (scrolls within vim's internal view, or does nothing) — Plexi does NOT scroll the scrollback. The `+TerminalMode::ALT_SCREEN` binding should fire instead.

---

## Task 7: Open PR

Follow the standard PLEXI branch workflow from `docs/superpowers/specs/2026-05-03-scrollback-design.md`. PR targets `alpha`. Title: `feat(terminal): scrollback navigation keys — Shift+PgUp/Dn, Cmd+Up/Down, Cmd+Home/End (#602)`.

DEV_LOG entry required before merge. `Breaks if:` — `Shift+PgUp` in a normal terminal does nothing (doesn't scroll), or pressing `Shift+PgUp` inside vim unexpectedly scrolls the Plexi scrollback buffer instead of letting vim handle it.
