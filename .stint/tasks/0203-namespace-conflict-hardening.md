---
id: "0203"
title: "Automation: namespace conflict hardening"
status: in-progress
estimate: "1h"
started_at: "2026-06-16T04:33:37Z"
blocked_by: []
gh_issue: []
area:
  - "host/ai"
  - "infra/agents"
  - "host/config"
tags:
  - "v1"
  - "automation"
  - "conflicts"
---


Make automatic iteration fail visibly on namespace conflicts instead of silently choosing a winner.

## Scope

- Audit current conflict checks for notes, commands, app connectors, Assistant tools, skills, and keybindings.
- Replace silent or implicit conflict resolution on model-visible tool names with explicit namespacing or a blocked/ambiguous state.
- Preserve existing unique filename behavior for scratchpad and note files; add a regression only if a real collision path is found.
- Add diagnostics that list conflicting owners and the namespace they collide in.
- Log conflict detection at `warn` or `error` level depending on whether the host can continue safely.

## Non-Scope

- Do not build a full package registry namespace policy in this task.
- Do not rename public tool IDs without a migration note in the task/PR.

## References

- `src/plexi_ai/tool_dispatch.rs`
- `src/host/keys.rs`
- `src/notes.rs`
- `src/assistant/mod.rs`
