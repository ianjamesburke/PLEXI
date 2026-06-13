---
id: "0179"
title: "feat(sdk): apps optionally report their own pip status (red/yellow/green)"
status: todo
estimate: "3h"
sprint: "s2"
blocked_by: []
gh_issue:
  - "2230"
area:
  - "sdk/python"
  - "host/pane-ops"
  - "ui/overlays"
tags: []
---


## What

Add an optional SDK + host protocol surface so apps can declare their own pip
state (green/yellow/red). Host falls back to today's derived activity when an
app hasn't set a status — no behavior change for existing apps.

## References

- GitHub issue #2230
- src/protocol/commands.rs:462 (AgentState enum, AppRequest enum ~483)
- src/host/pane.rs:402 (AppPane struct), line ~114 (effective_activity)
- src/app/lifecycle.rs:1290 (SetAgentState handler — model for new handler)
- sdk/python/plexi_sdk/_app.py (App class, add set_pip_status method)
- src/overlays/command_palette.rs:116 (no change needed — reads effective_activity)
