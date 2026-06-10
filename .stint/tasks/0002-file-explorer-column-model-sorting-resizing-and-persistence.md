---
id: "0002"
title: "File Explorer: column model sorting resizing and persistence"
status: backlog
estimate: 16h
sprint: "s1"
blocked_by:
  - 1
gh_issue: ["2136"]
area: ["apps/file-browser", "ui/widgets"]
tags: ["file-explorer", "details-table", "metadata"]
---

Add a real File Explorer column model on top of the adaptive shell.

## Why

Details view needs sortable and configurable metadata columns before search, view modes, and multi-select can share one durable data model.

## Gotchas

- Do not create a second file-state store for column persistence.
- Treat persisted widths and visibility as view preferences, not file metadata.

## References

- GitHub issue #2136
- Blocks: #2136 is blocked by #2135
- `docs/prm/file-explorer-overhaul.md`
