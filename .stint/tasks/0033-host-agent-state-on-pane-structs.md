---
id: "0033"
title: "Host agents: pane-native agent state"
status: in-progress
estimate: "8h"
started_at: "2026-06-10T22:26:58Z"
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
