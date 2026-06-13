---
id: "0060"
title: "v1 cleanup: context root env refresh"
status: done
estimate: "3h"
actual: "15m"
started_at: "2026-06-13T19:47:19Z"
completed_at: "2026-06-13T20:02:09Z"
sprint: "s11"
blocked_by: []
gh_issue:
  - "2018"
area:
  - "host/context"
tags:
  - "v1"
  - "cleanup"
  - "context"
---




Make `PLEXI_CONTEXT_ROOT` behavior explicit after `plexi context set-root`, including either live env refresh for existing panes or a user-visible restart/new-pane affordance.

Variance: estimated 3h, took 15m. The fix was narrower than anticipated — three small routing changes and one CLI tip addition. The hard part (understanding the three separate code paths for setting context root) was mostly analysis; the actual edits were trivial once the paths were mapped.
