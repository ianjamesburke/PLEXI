---
id: "0067"
title: "v1 UI: command palette aliases"
status: done
estimate: "3h"
actual: "30m"
started_at: "2026-06-15T07:49:33Z"
completed_at: "2026-06-15T17:44:37Z"
blocked_by:
  - 147
gh_issue:
  - "1733"
area:
  - "cli/commands"
tags:
  - "v1"
  - "ui"
  - "command-palette"
---





Add command-palette aliases so natural synonyms like `shell`, `console`, `hsplit`, and `config` resolve to the expected Plexi commands.

Sequenced after `0147` because validation handoff reliability is the immediate ship-pipeline CLI blocker.

## Note

The issue references old `poc/gpui-ui` paths; the current implementation lives in `src/overlays/command_palette.rs`.

## Variance

Completed with the search-cache task because both changes share the same command-palette entry model. The old path mismatch was the main audit cost.
