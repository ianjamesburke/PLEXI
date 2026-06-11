---
id: "0046"
title: "v1 cleanup: minimap context restore"
status: done
estimate: "3h"
actual: "26m"
started_at: "2026-06-11T08:48:56Z"
completed_at: "2026-06-11T09:14:50Z"
sprint: "s11"
blocked_by: []
gh_issue:
  - "2052"
area:
  - "host/navigation"
  - "host/context"
tags:
  - "v1"
  - "cleanup"
  - "minimap"
---



Restore each context's saved minimap visibility when focus-history traversal crosses context boundaries.

## Why

Context navigation should restore the destination context's UI state consistently, regardless of which navigation path changed focus.

Variance: actual was much lower than estimate because the issue already identified the exact owning paths and the fix stayed within focused save/restore logic plus regression tests.
