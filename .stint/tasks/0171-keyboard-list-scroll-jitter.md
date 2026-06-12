---
id: "0171"
title: "v1 cleanup: keyboard list scroll jitter — same-frame selection scrolling"
status: backlog
estimate: "3h"
sprint: "s11"
blocked_by: []
blocked_by_gh: []
gh_issue:
  - "2217"
area:
  - "ui/overlays"
  - "ui/widgets"
tags: []
---

## What

Keyboard navigation in list overlays (Cmd+P palette, notes picker, file
browser) jitters: selection renders below the viewport for a frame before
scroll_to_me catches up. Replace render-time scroll_to_me with a shared
same-frame keyboard-list scroll helper in src/ui/list.rs.

## References

- GitHub issue #2217
- src/overlays/command_palette.rs:495,531,573
- src/overlays/notes_picker.rs:490
- src/file_browser/mod.rs:1197,1234
