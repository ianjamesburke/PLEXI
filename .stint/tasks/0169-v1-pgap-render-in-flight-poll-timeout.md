---
id: "0169"
title: "v1 PGAP perf: bound render-in-flight poll and time out hung renders"
status: done
estimate: "4h"
actual: "15m"
started_at: "2026-06-12T18:09:49Z"
completed_at: "2026-06-12T18:25:08Z"
sprint: "s13"
blocked_by: []
gh_issue:
  - "2208"
area:
  - "sdk/pgap"
tags:
  - "v1"
  - "pgap"
  - "performance"
  - "rendering"
---




## What

`RENDER_IN_FLIGHT_POLL` (16ms) saturates to an immediate repaint after egui
subtracts predicted frame time, so a PGAP render in flight spins the host at
full frame rate, and an app that never sends FrameDone pins it there forever.
Raise the poll above one frame time and add a render-in-flight timeout that
clears the state and surfaces the hung status path.

## Why

Follow-up gap in the s13 / #2020 render scheduler: the remaining unbounded
idle-CPU path found during the 2026-06-12 performance audit (title repaint
loop fixed in d458cb4f; this is the second driver).

## Done When

A HostHarness test proves a pane whose render never completes stops
requesting immediate repaints after the timeout (gh #2208 Done When).
