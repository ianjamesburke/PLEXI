---
id: "0052"
title: "v1 PGAP perf: render allocation reduction"
status: done
estimate: "8h"
actual: "15m"
started_at: "2026-06-12T19:03:07Z"
completed_at: "2026-06-12T19:18:00Z"
sprint: "s13"
blocked_by:
  - 49
gh_issue:
  - "2024"
area:
  - "sdk/pgap"
  - "ui/widgets"
tags:
  - "v1"
  - "pgap"
  - "performance"
  - "rendering"
---



Reduce PGAP/component render hot-path clones, repeated text layout work, and throwaway per-frame render caches.

## Why

Once render scheduling is tighter, every legitimate app render frame should still be as cheap as practical.
