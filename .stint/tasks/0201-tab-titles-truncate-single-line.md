---
id: "0201"
title: "Tabs: truncate titles to one line"
status: todo
estimate: "45m"
sprint: "s34"
blocked_by: []
gh_issue: []
area:
  - "ui/tile-tree"
  - "ui/chrome"
tags:
  - "v1"
  - "tabs"
  - "ui"
---

Make tab labels elide or clip to a single line instead of wrapping and breaking fixed-height tab bars.

## Scope

- Reproduce a long tab title wrapping inside the current tab bar.
- Render tab labels with single-line truncation/elision inside the tab rect.
- Preserve activity pips, active styling, dividers, and drag reorder behavior.
- Add a focused rendering or unit regression for long titles.

## Non-Scope

- Do not change tab drag behavior or the v2 docking model.

## References

- `src/spatial/tiling.rs`
- `.stint/tasks/0189-v1-shallow-tab-drag-reorder.md`
