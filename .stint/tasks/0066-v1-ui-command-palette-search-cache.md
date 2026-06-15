---
id: "0066"
title: "v1 UI: command palette search cache"
status: done
estimate: "3h"
actual: "45m"
started_at: "2026-06-15T07:49:36Z"
completed_at: "2026-06-15T17:44:37Z"
blocked_by: []
gh_issue:
  - "1734"
area:
  - "cli/commands"
tags:
  - "v1"
  - "ui"
  - "command-palette"
  - "performance"
---





Remove avoidable per-keystroke command-palette allocations by precomputing searchable lowercase haystacks after aliases land.

## Note

The issue references old `poc/gpui-ui` paths; the current implementation lives in `src/overlays/command_palette.rs`.

## Variance

Completed with the aliases task by adding cached lowercase haystacks to the existing palette entries instead of introducing a separate cache layer.
