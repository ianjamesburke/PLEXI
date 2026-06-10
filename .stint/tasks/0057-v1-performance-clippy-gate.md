---
id: "0057"
title: "v1 performance: clippy perf gate"
status: backlog
sprint: "s14"
estimate: 4h
blocked_by: ["0049"]
blocked_by_gh: []
gh_issue: ["2026"]
area: ["infra/build", "infra/testing"]
tags: ["v1", "performance", "testing", "clippy"]
---

Make the Rust performance clippy pass actionable by fixing current all-target blockers and first-party perf lint warnings.

## Why

The PGAP performance sprint needs a repeatable lint gate so clone/allocation regressions are visible before optimization work lands.

## Gotchas

- Current blockers are stale `new_child_context` test call sites and `clippy::never_loop` in `src/main.rs`.
- Decide explicitly whether vendored `deps/egui_term` warnings are fixed locally or excluded/documented.
