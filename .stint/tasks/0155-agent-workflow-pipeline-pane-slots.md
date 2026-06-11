---
id: "0155"
title: "Agent workflow: pipeline pane slots"
status: in-progress
estimate: "1h"
started_at: "2026-06-11T08:54:17Z"
sprint: "s11"
blocked_by: []
gh_issue: []
area:
  - "infra/skills"
  - "cli/commands"
tags:
  - "workflow"
  - "dispatch"
  - "agents"
---


Have `implement-*`, `open-pr`, `validate-pr`, and `merge-pr` publish standard host pane slots so agent state survives PTY scrollback and can be summarized without scraping terminal output.

Required slots:

- `pipeline_phase`
- `issue`
- `pr`
- `status`
- `test_instructions`
- `last_error`

Keep the implementation small and skill-local where possible. Prefer shell snippets that use the existing `plexi pane slot write` command and current pane id resolution instead of adding new host protocol.
