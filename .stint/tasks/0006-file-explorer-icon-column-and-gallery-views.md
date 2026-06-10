---
id: "0006"
title: "File Explorer: icon column and gallery views"
status: backlog
sprint: "s1"
estimate: 16h
blocked_by: ["0002", "0003"]
blocked_by_gh: []
gh_issue: ["2140"]
area: ["apps/file-browser", "ui/widgets"]
tags: ["file-explorer", "view-modes"]
---

Add icon grid, column browser, and gallery views once list, details, and preview foundations are stable.

## Why

Richer view modes should reuse the same selection, metadata, preview, and activation model rather than forking File Explorer state.

## Gotchas

- Do not introduce a separate preview cache in this pass unless the existing preview model cannot support the mode.
- View switching must not lose selection or navigation state.

## References

- GitHub issue #2140
- Blocks: #2140 is blocked by #2136 and #2137
- `docs/prm/file-explorer-overhaul.md`
