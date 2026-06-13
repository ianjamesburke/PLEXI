---
id: "0175"
title: "v1 cleanup: Cmd+Shift+Space always opens a fresh scratch pad"
status: done
estimate: "2h"
completed_at: "2026-06-12T22:27:26Z"
sprint: "s11"
blocked_by: []
gh_issue:
  - "2221"
area:
  - "host/pane-ops"
tags: []
---


## What

open_scratchpad() redirects to an open editor whose note is empty on disk,
which misfires under debounced auto-save and hijacks the shortcut. Delete the
dedupe (find_open_empty_inbox_editor) so every press creates a fresh inbox
note; empty notes already self-delete on close.

## References

- GitHub issue #2221
- src/pane_ops/create.rs:1181-1240
