---
id: "0056"
title: "Assistant refactor: AI broker clone reduction"
status: in-progress
estimate: "6h"
started_at: "2026-06-13T19:38:40Z"
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
