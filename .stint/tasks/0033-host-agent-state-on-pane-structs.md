---
id: "0033"
title: "Host agents: pane-native agent state"
status: done
estimate: "8h"
actual: "27m"
started_at: "2026-06-10T22:26:58Z"
completed_at: "2026-06-10T22:52:59Z"
sprint: "s6"
blocked_by: []
gh_issue:
  - "2119"
area:
  - "host/pane-ops"
  - "cli/commands"
  - "agents"
tags:
  - "agents"
  - "host"
  - "state"
---



Move agent state onto pane structs so `pane info`, `pane list`, portals, and orchestration surfaces can read it without a side-channel join.

## Why

Agent state is pane metadata. Keeping it separate makes every status surface harder to reason about.

## Variance

Actual time was much lower than estimate because the issue already identified the exact fields, handlers, and response surfaces, and the implementation stayed localized to pane metadata plumbing plus regression tests.
