---
id: "0039"
title: "Workspace restore: builtin app state path"
status: done
estimate: "8h"
actual: "22m"
started_at: "2026-06-11T09:59:54Z"
completed_at: "2026-06-11T10:21:24Z"
sprint: "s9"
blocked_by: []
gh_issue:
  - "2143"
area:
  - "host/pane-ops"
  - "apps/text-editor"
  - "apps/file-browser"
tags:
  - "v1"
  - "workspace"
  - "apps"
  - "state"
---

Restore saved builtin app panes through a unified app-id plus serialized-state path so text editor, File Explorer, Secrets Manager, and future builtins reopen correctly.

Variance note: the task finished much faster than estimated because the restore path already had a single fallback seam; the work was limited to builtin constructor argument handling plus targeted regression coverage.

## Why

Workspace restore should preserve app runtime and app state, not just pane shape or names.
