---
id: "0206"
title: "Input: shared Vim and Emacs keymap modes"
status: todo
estimate: "3h"
sprint: "s34"
blocked_by:
  - 0200
gh_issue: []
area:
  - "apps/text-editor"
  - "host/terminal"
  - "host/config"
tags:
  - "v1"
  - "keymap"
  - "vim"
  - "emacs"
---

Add a shared keymap-mode layer that can serve text editors first and terminal copy/scrollback navigation later.

## Scope

- Introduce a shared keymap mode model: `standard`, `vim`, and `emacs`.
- Add a saved config setting for the default keymap mode used by newly opened text editors and scratchpads.
- Add command palette actions for keymap changes, starting with `Text Editor: Use Vim Mode`, `Text Editor: Use Standard Mode`, and `Text Editor: Use Emacs Mode`.
- Persist the chosen mode so newly opened text editors and scratchpads use it.
- Do not add a direct keyboard shortcut in this task. A later task should unify keyboard shortcut mapping with command palette execution so shortcuts can target the same command actions.
- Apply the first implementation to text editor/scratchpad navigation and editing only.
- Design the resolver so terminal copy mode and scrollback navigation can consume the same keymap model in a later pass.
- Vim v1 scope: normal/insert mode, `Esc`, `i`, `a`, `o`, `h/j/k/l`, word/line motions, delete/change/yank/paste basics. Skip ex commands such as `:w` and `:q`.
- Emacs v1 scope: add a small default navigation/editing set behind the same model, documented as provisional until reviewed by someone who knows Emacs defaults well.
- Add tests for mode persistence, command palette dispatch, and representative Vim/Emacs key dispatch.

## Non-Scope

- Do not implement Vim macros, registers beyond basic yank/paste, visual block mode, search/replace, or ex command mode.
- Do not fully rework terminal copy mode in this task; only keep the abstraction ready for it.
- Do not add a mode-toggle shortcut before command palette actions and keybinding mappings share a command target model.

## References

- `src/app/text_editor_app.rs`
- `src/host/keys.rs`
- `src/config/mod.rs`
- `src/process_app/render_session.rs`
