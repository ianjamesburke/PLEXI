---
id: "0044"
title: "v1 cleanup: terminal glyph padding"
status: done
estimate: "6h"
actual: "21m"
started_at: "2026-06-11T08:48:11Z"
completed_at: "2026-06-11T09:08:12Z"
sprint: "s11"
blocked_by: []
gh_issue:
  - "2067"
area:
  - "host/terminal"
  - "egui_term"
tags:
  - "v1"
  - "cleanup"
  - "terminal"
---



Fix left-edge terminal glyph clipping with renderer-level padding that does not corrupt grid sizing or SIGWINCH behavior.

## Why

Terminal prompts should not clip powerline, box-drawing, or wide glyph ink at column zero.

## Gotchas

- Previous attempts failed by treating padding outside the terminal renderer.
- Keep clip rect, grid rect, resize math, and glyph origin conceptually separate.

## Variance

Actual was much lower than estimate because the task resolved to a focused `TerminalView` API change plus one caller update, with existing resize math already isolated enough to reuse.
