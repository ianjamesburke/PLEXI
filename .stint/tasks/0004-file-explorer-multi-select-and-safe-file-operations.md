---
id: "0004"
title: "File Explorer: multi-select and safe file operations"
status: backlog
sprint: "s1"
estimate: 16h
blocked_by: ["0002", "0003"]
blocked_by_gh: []
gh_issue: ["2138"]
area: ["apps/file-browser", "ui/overlays", "ui/widgets"]
tags: ["file-explorer", "selection", "file-ops"]
---

Add multi-select and common file operations with confirmation UI and logging.

## Why

File Explorer is not a daily-driver surface until users can select multiple files and perform safe filesystem operations without leaving Plexi.

## Gotchas

- Destructive operations need host-owned confirmation UI and info-level logs.
- Keep operation failures explicit; never swallow I/O errors.

## References

- GitHub issue #2138
- Blocks: #2138 is blocked by #2136 and #2137
- `docs/prm/file-explorer-overhaul.md`
