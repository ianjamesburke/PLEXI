---
id: "0039"
title: "Workspace restore: builtin app state path"
status: backlog
sprint: "s10"
estimate: 8h
blocked_by: []
blocked_by_gh: []
gh_issue: ["2143"]
area: ["host/pane-ops", "apps/text-editor", "apps/file-browser"]
tags: ["v1", "workspace", "apps", "state"]
---

Restore saved builtin app panes through a unified app-id plus serialized-state path so text editor, File Explorer, Secrets Manager, and future builtins reopen correctly.

## Why

Workspace restore should preserve app runtime and app state, not just pane shape or names.
