---
id: "0002"
title: "File Explorer: column model sorting resizing and persistence"
status: done
estimate: "16h"
actual: "85m"
started_at: "2026-06-10T22:26:31Z"
completed_at: "2026-06-10T23:51:30Z"
sprint: "s1"
blocked_by:
  - 1
gh_issue:
  - "2136"
area:
  - "apps/file-browser"
  - "ui/widgets"
tags:
  - "file-explorer"
  - "details-table"
  - "metadata"
---



Add a real File Explorer column model on top of the adaptive shell.

## Why

Details view needs sortable and configurable metadata columns before search, view modes, and multi-select can share one durable data model.

## Estimate Variance

Actual was far under estimate because #2135 had already landed the adaptive shell, and this task stayed concentrated in the File Explorer model/render path instead of requiring a broader Host UI Kit table primitive.

## Gotchas

- Do not create a second file-state store for column persistence.
- Treat persisted widths and visibility as view preferences, not file metadata.

## References

- GitHub issue #2136
- Blocks: #2136 is blocked by #2135
- `docs/prm/file-explorer-overhaul.md`
