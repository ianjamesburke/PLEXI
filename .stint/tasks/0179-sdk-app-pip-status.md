---
id: "0179"
title: "feat(sdk): apps optionally report their own pip status (red/yellow/green)"
status: done
estimate: "3h"
actual: "0m"
completed_at: "2026-06-13T16:45:46Z"
sprint: "s2"
blocked_by: []
gh_issue:
  - "2230"
area:
  - "sdk/python"
  - "host/pane-ops"
  - "ui/overlays"
tags:
  - "v1"
  - "app-authoring"
---



## What

Add an optional SDK + host protocol surface so apps can declare their own pip
state (green/yellow/red). Host falls back to today's derived activity when an
app hasn't set a status. No behavior change for existing apps.

## Scope

- Add `SetPipStatus { status: PipStatus }` variant to `AppRequest` enum, modeled on the existing `SetAgentState` handler.
- Add `PipStatus` enum (green/yellow/red) to `src/protocol/commands.rs`.
- Store on `AppPane` struct. `effective_activity()` checks pip status first, falls back to derived activity.
- Add `App.set_pip_status(status)` method to the Python SDK.
- No overlay changes needed (command palette reads `effective_activity` which will pick up the new signal).

## References (verified line numbers as of v0.0.768)

- GitHub issue #2230
- `src/protocol/commands.rs:464` (AgentState enum), `:485` (AppRequest enum)
- `src/host/pane.rs:408` (AppPane struct), `:114` (effective_activity)
- `src/app/lifecycle.rs:1290` (SetAgentState handler, model for new handler)
- `sdk/python/plexi_sdk/_app.py:139` (App class, add set_pip_status method)
