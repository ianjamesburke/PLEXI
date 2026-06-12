---
id: "0162"
title: "notes: unified store — global inbox capture, retire backlog + quick-note destinations"
status: in-progress
estimate: "8h"
started_at: "2026-06-12T00:50:15Z"
sprint: "s9"
blocked_by: []
gh_issue:
  - "2193"
area:
  - "host/pane-ops"
  - "ui/overlays"
  - "host/config"
  - "cli/commands"
tags:
  - "notes"
---


Unify note storage under `<config_dir>/notes/`: global `inbox/` of per-capture markdown files with frontmatter, workspace dirs for kept notes, `trash/`. Cmd+0 = pure capture to inbox. Delete `[[quick_note.destinations]]` config + destination picker, dissolve the `backlog` line-file (with migration), retire `apps/backlog/`. Directory IS status; frontmatter is immutable capture metadata only. Full spec in GH #2193.
