---
id: "0044"
title: "v1 cleanup: terminal glyph padding"
status: in-progress
estimate: "6h"
started_at: "2026-06-11T08:48:11Z"
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
