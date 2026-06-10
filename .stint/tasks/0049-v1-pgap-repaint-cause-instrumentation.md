---
id: "0049"
title: "v1 PGAP perf: repaint cause instrumentation"
status: backlog
sprint: "s13"
estimate: 4h
blocked_by: []
blocked_by_gh: []
gh_issue: ["2019"]
area: ["host/events", "ui/tile-tree"]
tags: ["v1", "pgap", "performance", "instrumentation"]
---

Add host-side frame/repaint cause instrumentation so idle CPU/GPU regressions can be attributed before changing scheduling behavior.

## Why

The visible-app render loop and background ticks need measurement before they are safely tightened.
