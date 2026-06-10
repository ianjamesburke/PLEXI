---
id: "0068"
title: "v1 cleanup: dispatch close notification suppression"
status: backlog
sprint: "s11"
estimate: 2h
blocked_by:
  - 147
gh_issue: ["1692"]
area: ["host/notifications", "infra/skills"]
tags: ["v1", "cleanup", "dispatch"]
---

Suppress redundant pane-close notifications after successful dispatch merges while preserving useful failure or unexpected-close notifications.

Sequenced after `0147` so dispatch notification cleanup builds on the non-blocking validation handoff path instead of preserving the current blocking choice behavior.
