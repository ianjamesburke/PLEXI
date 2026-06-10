---
id: "0053"
title: "v2 performance: CLI renderer allocation cleanup"
status: backlog
sprint: "s16"
estimate: 6h
blocked_by:
  - 147
gh_issue: ["2027"]
area: ["cli/commands"]
tags: ["v2", "performance", "cli-renderer"]
---

Render the native CLI descriptor UI from borrowed descriptor data instead of rebuilding cloned row, argument, and flag vectors every frame.

Sequenced after `0147` because validation handoff reliability is the immediate ship-pipeline CLI blocker.

## Why

This is useful cleanup, but it is not a v1 blocker unless the CLI renderer becomes a visible release path.
