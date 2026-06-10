---
id: "0067"
title: "v1 UI: command palette aliases"
status: backlog
sprint: "s5"
estimate: 3h
blocked_by:
  - 147
gh_issue: ["1733"]
area: ["cli/commands"]
tags: ["v1", "ui", "command-palette"]
---

Add command-palette aliases so natural synonyms like `shell`, `console`, `hsplit`, and `config` resolve to the expected Plexi commands.

Sequenced after `0147` because validation handoff reliability is the immediate ship-pipeline CLI blocker.

## Note

The issue references old `poc/gpui-ui` paths; the current implementation lives in `src/overlays/command_palette.rs`.
