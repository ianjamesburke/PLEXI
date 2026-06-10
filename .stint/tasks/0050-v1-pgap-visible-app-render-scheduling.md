---
id: "0050"
title: "v1 PGAP perf: visible app render scheduling"
status: backlog
sprint: "s13"
estimate: 12h
blocked_by: ["0049"]
blocked_by_gh: []
gh_issue: ["2020"]
area: ["host/pane-ops", "sdk/pgap"]
tags: ["v1", "pgap", "performance", "rendering"]
---

Stop visible idle ProcessApp panes from sending recurring `Render` events and 100ms repaint requests unless input, dirty state, async completion, or explicit `ScheduleRender` requires it.

## Why

L1 hardened render content, but the host still has a recurring visible-pane render scheduling loop.
