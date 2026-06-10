---
id: "0052"
title: "v1 PGAP perf: render allocation reduction"
status: backlog
sprint: "s13"
estimate: 8h
blocked_by: ["0049"]
blocked_by_gh: []
gh_issue: ["2024"]
area: ["sdk/pgap", "ui/widgets"]
tags: ["v1", "pgap", "performance", "rendering"]
---

Reduce PGAP/component render hot-path clones, repeated text layout work, and throwaway per-frame render caches.

## Why

Once render scheduling is tighter, every legitimate app render frame should still be as cheap as practical.
