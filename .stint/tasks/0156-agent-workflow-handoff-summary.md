---
id: "0156"
title: "Agent workflow: handoff summary"
status: done
estimate: "1h"
actual: "17m"
started_at: "2026-06-11T08:54:20Z"
completed_at: "2026-06-11T09:10:48Z"
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

Variance: faster than estimated because the summary could be implemented as a skill helper over `pane list` slot metadata instead of adding a Rust CLI subcommand.
