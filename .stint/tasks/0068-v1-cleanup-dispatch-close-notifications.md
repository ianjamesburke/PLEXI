---
id: "0068"
title: "v1 cleanup: dispatch close notification suppression"
status: done
estimate: "2h"
actual: "17m"
started_at: "2026-06-11T08:54:17Z"
completed_at: "2026-06-11T09:10:48Z"
sprint: "s11"
blocked_by: []
gh_issue:
  - "1692"
area:
  - "host/notifications"
  - "infra/skills"
tags:
  - "v1"
  - "cleanup"
  - "dispatch"
---



Suppress redundant pane-close notifications after successful dispatch merges while preserving useful failure or unexpected-close notifications.

Sequenced after `0147` so dispatch notification cleanup builds on the non-blocking validation handoff path instead of preserving the current blocking choice behavior.

Variance: faster than estimated because the redundant close notification was skill-emitted in `merge-pr`, not host-emitted.
