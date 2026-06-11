---
id: "0050"
title: "v1 PGAP perf: visible app render scheduling"
status: done
estimate: "12h"
actual: "224m"
started_at: "2026-06-11T06:05:50Z"
completed_at: "2026-06-11T09:49:23Z"
sprint: "s13"
blocked_by:
  - 49
gh_issue:
  - "2020"
area:
  - "host/pane-ops"
  - "sdk/pgap"
tags:
  - "v1"
  - "pgap"
  - "performance"
  - "rendering"
---



Stop visible idle ProcessApp panes from sending recurring `Render` events and 100ms repaint requests unless input, dirty state, async completion, or explicit `ScheduleRender` requires it.

## Why

L1 hardened render content, but the host still has a recurring visible-pane render scheduling loop.

## Variance

Actual 224m vs 12h estimate: the runtime/scheduler/transport split landed faster than planned, but most of the elapsed time went to diagnosing a continuous-mode 30fps phase-lock (relative followup deadlines vs vsync-quantized FrameDone RTT) rather than the idle-polling removal the estimate covered.
