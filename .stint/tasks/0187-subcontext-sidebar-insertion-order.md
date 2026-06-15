---
id: "0187"
title: "fix: push-to-subcontext inserts new context under parent in sidebar"
status: in-progress
estimate: "1h"
started_at: "2026-06-15T17:52:04Z"
blocked_by: []
gh_issue:
  - "2255"
area:
  - "host/context"
tags: []
---


When pushing a subcontext from within a subcontext, the new nested context should be inserted immediately after its parent in the sidebar list, not appended at the end.

Fix: add `insert_after_subtree(parent_id, ctx)` to `WorkspaceRouter` and use it in `push_pane_to_subcontext` instead of `push`.
