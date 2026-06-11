---
id: "0055"
title: "v2 terminal perf: grid and event clone reduction"
status: backlog
sprint: "s18"
estimate: 6h
blocked_by:
  - 54
  - 30
  - 31
gh_issue: ["2025"]
area: ["host/terminal"]
tags: ["v2", "performance", "terminal"]
---

Reduce terminal backend data movement by avoiding full-grid, cursor, event, regex, and font clones when only small terminal state changes.

## Why

This is what #2025 is: terminal renderer/backend ownership cleanup, not PGAP.
