---
id: "0044"
title: "v1 cleanup: terminal glyph padding"
status: backlog
sprint: "s11"
estimate: 6h
blocked_by: []
blocked_by_gh: []
gh_issue: ["2067"]
area: ["host/terminal", "egui_term"]
tags: ["v1", "cleanup", "terminal"]
---

Fix left-edge terminal glyph clipping with renderer-level padding that does not corrupt grid sizing or SIGWINCH behavior.

## Why

Terminal prompts should not clip powerline, box-drawing, or wide glyph ink at column zero.

## Gotchas

- Previous attempts failed by treating padding outside the terminal renderer.
- Keep clip rect, grid rect, resize math, and glyph origin conceptually separate.
