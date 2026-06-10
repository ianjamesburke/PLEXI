---
id: "0147"
title: "CLI: non-blocking choice notifications for validation handoffs"
status: backlog
sprint: "s6"
estimate: 4h
blocked_by: []
gh_issue: ["2151"]
area: ["host/notifications", "cli/commands", "infra/skills"]
tags: ["v1", "workflow", "dispatch", "validation"]
---

Add a `plexi notify --no-wait` path so choice notifications can run host actions like `pane_focus:<id>` without keeping the sender process blocked on a response file.

This belongs ahead of agent status polish because PR validation is on the ship path. The notification is an attention cue; the official `[TESTING]` block remains the source of truth.
