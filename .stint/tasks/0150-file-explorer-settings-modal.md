---
id: "0150"
title: "File Explorer: settings modal for column and view controls"
status: backlog
estimate: "8h"
sprint: "s1"
blocked_by:
  - 2
gh_issue:
  - "2153"
area:
  - "apps/file-browser"
  - "ui/overlays"
  - "ui/widgets"
tags:
  - "file-explorer"
  - "settings"
  - "columns"
  - "host-ui-kit"
---



Move File Explorer column and view controls into a dedicated settings modal instead of crowding the path toolbar.

## Why

Column visibility, sort, and folders-on-top are real File Explorer view settings. They should live behind a scalable File Explorer-owned settings modal, while the browsing toolbar stays focused on path/navigation/search.

## Gotchas

- Keep pane-state persistence as the source of truth for current view preferences.
- Do not add host `config.toml` settings in this task.
- Use Host UI Kit modal and hint primitives; do not hand-paint a File Explorer-only modal shell.

## References

- GitHub issue #2153
- Blocks: #2153 is blocked by #2136 / stint 0002
- `src/file_browser/mod.rs`
- `src/file_browser/helpers.rs`
- `website/src/content/docs/file-explorer.md`
