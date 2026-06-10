---
id: "0066"
title: "v1 UI: command palette search cache"
status: backlog
sprint: "s5"
estimate: 3h
blocked_by: ["0067"]
blocked_by_gh: []
gh_issue: ["1734"]
area: ["cli/commands"]
tags: ["v1", "ui", "command-palette", "performance"]
---

Remove avoidable per-keystroke command-palette allocations by precomputing searchable lowercase haystacks after aliases land.

## Note

The issue references old `poc/gpui-ui` paths; the current implementation lives in `src/overlays/command_palette.rs`.
