---
id: "0187"
title: "fix: push-to-subcontext inserts new context under parent in sidebar"
status: backlog
sprint: s4
area: host/context
estimate: 1h
gh_issue: ["2255"]
---

When pushing a subcontext from within a subcontext, the new nested context should be inserted immediately after its parent in the sidebar list, not appended at the end.

Fix: add `insert_after_subtree(parent_id, ctx)` to `WorkspaceRouter` and use it in `push_pane_to_subcontext` instead of `push`.
