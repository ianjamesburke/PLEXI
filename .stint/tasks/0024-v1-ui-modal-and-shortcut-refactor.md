---
id: "0024"
title: "v1 UI: modal and shortcut refactor"
status: done
estimate: "16h"
actual: "13m"
started_at: "2026-06-11T17:57:59Z"
completed_at: "2026-06-11T18:10:30Z"
sprint: "s5"
blocked_by:
  - 23
gh_issue: []
area:
  - "ui/overlays"
  - "ui/widgets"
tags:
  - "v1"
  - "ui"
  - "modals"
  - "shortcuts"
---



Move remaining v1 modals and shortcut hint surfaces onto centralized ModalShell, ListRow, TextField, Button, HintBar, and key-chip primitives.

## Why

Users should not see multiple modal grammars or shortcut display styles across the host.

## Gotchas

- Preserve existing keyboard behavior.
- Do not create File Explorer-specific or marketplace-specific clones of shared primitives.

## Variance

Completed as part of the 0023-0027 bundled stabilization pass; remaining visible modals mostly needed caller migration to existing primitives.
