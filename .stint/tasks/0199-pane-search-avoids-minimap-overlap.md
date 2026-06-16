---
id: "0199"
title: "Pane search: avoid minimap overlap"
status: backlog
estimate: "1h"
blocked_by: []
gh_issue: []
area:
  - "host/terminal"
  - "ui/chrome"
tags:
  - "v1"
  - "search"
  - "minimap"
---


Move the Cmd+F pane search UI out of the minimap collision zone so search and the top-right minimap can be visible at the same time.

## Scope

- Reproduce the overlap with the minimap visible before changing layout.
- Position the pane search field as pane-local top-center chrome, or otherwise reserve space so it never overlaps the minimap.
- Keep Cmd+F search behavior and terminal search semantics unchanged.
- Add a focused UI or harness regression that covers minimap visible plus search open.
- Log the search overlay open path at `info` level with whether minimap was visible.

## Non-Scope

- Do not redesign terminal search behavior or recursive File Explorer search.

## References

- `src/render/minimap.rs`
- `src/overlays/setup.rs`
- `src/process_app/render_session.rs`
