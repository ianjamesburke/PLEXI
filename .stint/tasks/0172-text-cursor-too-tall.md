---
id: "0172"
title: "v1 cleanup: blinking cursor a few px too tall in scratch pad / quick note"
status: backlog
estimate: "2h"
sprint: "s11"
blocked_by: []
blocked_by_gh: []
gh_issue:
  - "2218"
area:
  - "apps/text-editor"
  - "ui/overlays"
tags: []
---

## What

egui's default caret spans the full galley row height (including leading) so
it visually overflows its line in the text editor and quick note. Shorten the
painted caret to the font's visual extent via the themed Visuals seam or a
leading adjustment.

## References

- GitHub issue #2218
- src/app/text_editor_app.rs:476-489
- src/overlays/quick_note.rs
