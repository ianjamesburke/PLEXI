---
id: "0167"
title: "Host Assistant pane UI polish — picker clipping, stub-command feedback"
status: backlog
estimate: "2h"
sprint: "s5"
blocked_by: []
blocked_by_gh:
  - "2187"
gh_issue:
  - "2201"
area:
  - "ui/widgets"
  - "host/ai"
tags: []
---

## What

Polish the Phase D1 host Assistant pane: bound the slash-command picker to the
visible pane rect (scrollable list, selected item kept in view), render
not-yet-implemented commands as a distinct planned-command row instead of a
plain error-looking message, and sweep transcript/composer spacing against
style.rs tokens.

## Why

The Assistant is the flagship surface of the agent platform; a picker that
clips off screen reads as broken even though selection works.

## References

- GitHub issue #2201
- src/assistant/render.rs (draw_picker)
- src/assistant/model.rs
