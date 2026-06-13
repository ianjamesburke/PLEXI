---
id: "0060"
title: "v1 cleanup: context root env refresh"
status: todo
estimate: "3h"
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
