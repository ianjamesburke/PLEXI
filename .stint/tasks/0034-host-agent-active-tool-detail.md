---
id: "0034"
title: "Host agents: active tool detail in status"
status: done
estimate: "6h"
actual: "140m"
started_at: "2026-06-11T03:36:32Z"
completed_at: "2026-06-11T05:56:19Z"
sprint: "s6"
blocked_by:
  - 33
  - 147
gh_issue:
  - "2120"
area:
  - "host/pane-ops"
  - "cli/commands"
  - "agents"
tags:
  - "agents"
  - "host"
  - "status"
---



Surface concise active-tool detail from Claude Code hooks so agent panes show what work is happening, not just that work is happening.

Sequenced after `0147` because validation notification reliability is higher-priority workflow infrastructure in the same CLI/agent operations lane.

## Why

`working` is not enough for orchestration. Tool detail makes agent status inspectable without scraping terminal output.

## Variance

Finished under estimate because the issue already carried a concrete implementation map and the existing agent-state path only needed a narrow optional field plus hook formatting.
