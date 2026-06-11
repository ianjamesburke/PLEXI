---
id: "0003"
title: "File Explorer: inspector and Quick Look overlay"
status: done
estimate: "12h"
actual: "12m"
started_at: "2026-06-11T00:02:05Z"
completed_at: "2026-06-11T00:13:53Z"
sprint: "s1"
blocked_by:
  - 1
gh_issue:
  - "2137"
area:
  - "apps/file-browser"
  - "ui/overlays"
  - "ui/widgets"
tags:
  - "file-explorer"
  - "preview"
  - "quick-look"
---



Replace the fixed preview threshold with an explicit inspector and Space-driven Quick Look overlay.

## Why

Preview and metadata should be reachable in narrow panes without depending on a hard wide-pane breakpoint.

## Gotchas

- Quick Look should use Host UI Kit modal primitives.
- Do not rewrite media players; keep audio/video handoff behavior.

## References

- GitHub issue #2137
- Blocks: #2137 is blocked by #2135
- `docs/prm/file-explorer-overhaul.md`

## Variance

Actual time was under estimate because the existing File Explorer layout, preview cache, Host UI Kit ModalShell, and HintBar primitives were already in place from prior tasks, so this slice mainly wired explicit state, shortcuts, and preview rendering instead of building new shared UI infrastructure.
