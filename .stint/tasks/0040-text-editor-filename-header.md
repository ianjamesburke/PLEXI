---
id: "0040"
title: "Text editor: note title header bar"
status: todo
estimate: "2h"
sprint: "s9"
blocked_by: []
gh_issue:
  - "2205"
area:
  - "apps/text-editor"
  - "ui/widgets"
tags:
  - "v1"
  - "text-editor"
  - "ui"
---


Restyle the note title row (added in PR #2197) as a centered pane-style header bar: full-width 20px strip flush with the top of the pane, `pane_header_bg()` fill, title (or filename-stem placeholder) painted centered in `text_dim` — matching the terminal pane name bar.

## Why

The current left-aligned, inset title reads as body text and the gap above it looks like a rendering bug. Supersedes the original filename-header scope (gh #2086, closed).
