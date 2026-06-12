---
id: "0059"
title: "v1 PGAP perf: portal and context metadata cache"
status: done
estimate: "4h"
actual: "15m"
started_at: "2026-06-12T19:18:06Z"
completed_at: "2026-06-12T19:31:28Z"
sprint: "s13"
blocked_by:
  - 50
  - 51
  - 52
gh_issue:
  - "2023"
area:
  - "ui/tile-tree"
  - "host/context"
tags:
  - "v1"
  - "pgap"
  - "performance"
---



Cache central-panel portal previews, pane metadata, context snapshots, pane names, and related maps behind explicit invalidation.

## Why

This is not the root idle repaint cause, so it follows the render scheduling work; once frame wakes are tighter, per-frame metadata churn should scale with real state changes.
