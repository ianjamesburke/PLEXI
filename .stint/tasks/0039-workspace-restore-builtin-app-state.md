---
id: "0039"
title: "Workspace restore: builtin app state path"
status: in-progress
estimate: "8h"
started_at: "2026-06-11T09:59:54Z"
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

## Why

Workspace restore should preserve app runtime and app state, not just pane shape or names.
