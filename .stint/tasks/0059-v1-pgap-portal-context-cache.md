---
id: "0059"
title: "v1 PGAP perf: portal and context metadata cache"
status: backlog
sprint: "s14"
estimate: 4h
blocked_by: ["0050", "0051", "0052"]
blocked_by_gh: []
gh_issue: ["2023"]
area: ["ui/tile-tree", "host/context"]
tags: ["v1", "pgap", "performance"]
---

Cache central-panel portal previews, pane metadata, context snapshots, pane names, and related maps behind explicit invalidation.

## Why

This is not the root idle repaint cause, so it follows the render scheduling work; once frame wakes are tighter, per-frame metadata churn should scale with real state changes.
