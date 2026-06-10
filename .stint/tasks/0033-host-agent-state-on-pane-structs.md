---
id: "0033"
title: "Host agents: pane-native agent state"
status: backlog
sprint: "s7"
estimate: 8h
blocked_by: []
blocked_by_gh: []
gh_issue: ["2119"]
area: ["host/pane-ops", "cli/commands", "agents"]
tags: ["agents", "host", "state"]
---

Move agent state onto pane structs so `pane info`, `pane list`, portals, and orchestration surfaces can read it without a side-channel join.

## Why

Agent state is pane metadata. Keeping it separate makes every status surface harder to reason about.
