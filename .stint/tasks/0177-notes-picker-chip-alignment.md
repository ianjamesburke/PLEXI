---
id: "0177"
title: "v1 cleanup: notes picker right-aligns type chips like the palette"
status: done
estimate: "1h"
actual: "8m"
started_at: "2026-06-13T21:40:48Z"
completed_at: "2026-06-13T21:48:04Z"
sprint: "s11"
blocked_by: []
gh_issue:
  - "2227"
area:
  - "ui/overlays"
tags: []
---




**Variance note:** 8m actual vs 1h estimate — one-line swap; existing UI test covered the render path completely, no new test authoring needed.

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
