---
id: "0174"
title: "v1 cleanup: SessionStart hook reports idle, not working"
status: done
estimate: "1h"
completed_at: "2026-06-12T21:48:59Z"
sprint: "s11"
blocked_by: []
gh_issue:
  - "2220"
area:
  - "agents"
  - "cli/commands"
tags: []
---


## What

The generated Claude Code hook script maps SessionStart to "working", so new
sessions show a green pulsing pip before doing anything. Split the case so
SessionStart reports "idle"; UserPromptSubmit stays "working". Hooks must be
reinstalled to pick up the change.

## References

- GitHub issue #2220
- src/cli/agent.rs:456-464
