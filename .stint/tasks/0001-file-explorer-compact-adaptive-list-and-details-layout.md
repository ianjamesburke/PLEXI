---
id: "0001"
title: "File Explorer: compact adaptive list and details layout"
status: todo
estimate: 16h
sprint: "s1"
blocked_by: []
blocked_by_gh: []
gh_issue: ["2135"]
area: ["apps/file-browser", "ui/widgets"]
tags: ["file-explorer", "layout", "host-ui-kit"]
---

Build the responsive File Explorer foundation from `docs/prm/file-explorer-overhaul.md`.

## Why

The current File Explorer row/card layout and fixed preview breakpoint make the app weak in split panes. This task creates the shared shell every later File Explorer task depends on.

## Gotchas

- Consume Host UI Kit primitives before adding any File Explorer-specific chrome.
- Keep terminal CWD sync and existing activation behavior working while layout changes.

## References

- GitHub issue #2135
- `docs/prm/file-explorer-overhaul.md`
- `src/file_browser/mod.rs`
