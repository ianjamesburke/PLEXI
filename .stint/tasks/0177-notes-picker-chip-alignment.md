---
id: "0177"
title: "v1 cleanup: notes picker right-aligns type chips like the palette"
status: backlog
estimate: "1h"
sprint: "s11"
blocked_by: []
blocked_by_gh:
  - "2194"
gh_issue:
  - "2227"
area:
  - "ui/overlays"
tags: []
---

## What

Swap the notes picker rows from inline `.chip()` to the right-aligned
`.metadata_chips()` style the command palette uses, so both list overlays
share one row visual language. Verify the right-aligned chip coexists with
the row's trailing `×` delete action. Sequenced after the notes-unification
bundle (#2194 adds an Inbox section to the same file).

## References

- GitHub issue #2227
- src/overlays/notes_picker.rs:482-483
- src/ui/list.rs:56-61,162-180
