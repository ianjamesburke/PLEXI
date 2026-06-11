---
id: "0147"
title: "CLI: non-blocking choice notifications for validation handoffs"
status: done
estimate: "4h"
actual: "21m"
started_at: "2026-06-10T23:48:39Z"
completed_at: "2026-06-11T00:09:06Z"
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

Variance: implementation was shorter than estimated because the host already supported `response_file: None` for choice actions; only CLI payload gating and validation skill wiring were needed.
