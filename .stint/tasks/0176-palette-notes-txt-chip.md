---
id: "0176"
title: "v1 cleanup: notes in command palette with txt chip"
status: backlog
estimate: "3h"
sprint: "s11"
blocked_by: []
blocked_by_gh: []
gh_issue:
  - "2222"
area:
  - "ui/overlays"
  - "ui/widgets"
tags: []
---

## What

Add a Note palette entry type populated from the unified note store
(NotePickerEntry/scan_inbox reuse), rendered with a "txt" chip via existing
chip widgets; activation jumps to an open editor pane or launches text-editor
with the path, matching Cmd+O semantics.

## References

- GitHub issue #2222
- src/overlays/command_palette.rs
- src/notes.rs:106-149,327
