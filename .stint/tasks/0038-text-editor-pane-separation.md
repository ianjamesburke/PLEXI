---
id: "0038"
title: "Text editor: visible pane separation"
status: done
estimate: "3h"
completed_at: "2026-06-13T19:06:23Z"
sprint: "s9"
blocked_by: []
gh_issue:
  - "2142"
area:
  - "apps/text-editor"
  - "ui/widgets"
tags:
  - "v1"
  - "text-editor"
  - "ui"
---



Add a subtle boundary treatment so stacked text editor panes are visibly separate instead of colliding into one full-bleed surface.

## Why

The text editor must remain readable and pane-safe in common tiled layouts.
