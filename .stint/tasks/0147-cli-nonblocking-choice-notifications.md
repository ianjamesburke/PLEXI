---
id: "0147"
title: "CLI: non-blocking choice notifications for validation handoffs"
status: in-progress
estimate: "4h"
started_at: "2026-06-10T23:48:39Z"
sprint: "s6"
blocked_by: []
gh_issue:
  - "2151"
area:
  - "host/notifications"
  - "cli/commands"
  - "infra/skills"
tags:
  - "v1"
  - "workflow"
  - "dispatch"
  - "validation"
---


Add a `plexi notify --no-wait` path so choice notifications can run host actions like `pane_focus:<id>` without keeping the sender process blocked on a response file.

This belongs ahead of agent status polish because PR validation is on the ship path. The notification is an attention cue; the official `[TESTING]` block remains the source of truth.
