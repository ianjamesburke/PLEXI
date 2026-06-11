---
id: "0049"
title: "v1 PGAP perf: repaint cause instrumentation"
status: in-progress
estimate: "4h"
started_at: "2026-06-11T03:37:55Z"
sprint: "s13"
blocked_by: []
gh_issue:
  - "2019"
area:
  - "host/events"
  - "ui/tile-tree"
tags:
  - "v1"
  - "pgap"
  - "performance"
  - "instrumentation"
---


Add host-side frame/repaint cause instrumentation so idle CPU/GPU regressions can be attributed before changing scheduling behavior.

## Why

The visible-app render loop and background ticks need measurement before they are safely tightened.
