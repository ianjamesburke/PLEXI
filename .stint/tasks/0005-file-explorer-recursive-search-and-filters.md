---
id: "0005"
title: "File Explorer: recursive search and filters"
status: backlog
sprint: "s1"
estimate: 12h
blocked_by: ["0002"]
blocked_by_gh: []
gh_issue: ["2139"]
area: ["apps/file-browser", "ui/widgets"]
tags: ["file-explorer", "search", "filters"]
---

Expand current-directory fuzzy filtering into recursive scoped search with metadata filters.

## Why

Users need to find files inside a project or context without leaving Plexi, but the first pass should avoid a separate indexing system.

## Gotchas

- Reuse the column metadata vocabulary from task `0002`.
- Do not add a long-lived indexer in this sprint.

## References

- GitHub issue #2139
- Blocks: #2139 is blocked by #2136
- `docs/prm/file-explorer-overhaul.md`
