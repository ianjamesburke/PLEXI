---
id: "0056"
title: "Assistant refactor: AI broker clone reduction"
status: done
estimate: "6h"
actual: "31m"
started_at: "2026-06-13T19:38:40Z"
completed_at: "2026-06-13T20:08:58Z"
sprint: "s12"
blocked_by: []
gh_issue:
  - "2028"
area:
  - "host/ai"
tags:
  - "v1"
  - "assistant"
  - "ai"
  - "performance"
---




Reduce AI broker pane snapshot and tool-loop cloning as part of the broader Assistant app refactor lane.

## Why

The first-party Assistant needs UI, capability, instrumentation, and broker cleanup as one product lane before v1; #2028 is the broker-performance slice.

## Variance Note

Estimate 6h vs actual 31m. The task was well-scoped before implementation — Arc types, RwLock swap pattern, and serde `rc` feature were clear from the start. The 6h estimate assumed deeper discovery and possible refactor cascades; none materialized. Gemini review found two real issues (unsafe raw ptr test, Mutex vs RwLock) but both were straightforward fixes.
