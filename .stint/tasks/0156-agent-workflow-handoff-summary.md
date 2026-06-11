---
id: "0156"
title: "Agent workflow: handoff summary"
status: in-progress
estimate: "1h"
started_at: "2026-06-11T08:54:20Z"
sprint: "s11"
blocked_by:
  - 155
gh_issue: []
area:
  - "cli/commands"
  - "infra/skills"
tags:
  - "workflow"
  - "dispatch"
  - "agents"
---


Add a CLI command or skill helper that reads `pane info` slot metadata across panes and prints what each agent is doing, waiting on, and needs from Ian.

The output should be optimized for handoff scanning, not raw JSON inspection. It should consume the standard pipeline slots from `0155` and degrade gracefully when panes have no agent slots yet.
