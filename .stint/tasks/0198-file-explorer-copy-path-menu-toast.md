---
id: "0198"
title: "File Explorer: copy path menu and toast feedback"
status: backlog
estimate: "1h"
blocked_by: []
gh_issue: []
area:
  - "apps/file-browser"
  - "ui/widgets"
tags:
  - "v1"
  - "file-explorer"
  - "ui"
---


Add File Explorer actions for copying selected paths without overloading the existing file-operation copy buffer.

## Scope

- Add a right-click/context menu for selected File Explorer entries.
- Add `Copy Path` and `Copy Shell Path` actions for the current selection.
- Add keyboard access for path copy, preferring `Option+C` for raw path and `Option+Shift+C` for shell-escaped path if the shortcut audit confirms no conflict.
- Copy multi-selection paths as newline-separated text.
- Show transient File Explorer-local toast/status feedback such as `Copied 3 paths`.
- Log each copy-path action at `info` level with count and action kind, not full path contents.

## Non-Scope

- Do not change the existing Cmd+C/Cmd+X/Cmd+V file operation clipboard.
- Do not add recursive search or new file operations here.

## References

- `docs/prm/file-explorer-overhaul.md`
- `src/file_browser/mod.rs`
