---
id: "0024"
title: "v1 UI: modal and shortcut refactor"
status: in-progress
estimate: "16h"
started_at: "2026-06-11T17:57:59Z"
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
