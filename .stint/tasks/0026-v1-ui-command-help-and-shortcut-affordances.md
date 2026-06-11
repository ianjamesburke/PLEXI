---
id: "0026"
title: "v1 UI: command help and shortcut affordances"
status: done
estimate: "8h"
actual: "13m"
started_at: "2026-06-11T17:57:59Z"
completed_at: "2026-06-11T18:10:30Z"
sprint: "s5"
blocked_by:
  - 24
gh_issue: []
area:
  - "ui/overlays"
  - "cli/commands"
tags:
  - "v1"
  - "ui"
  - "shortcuts"
  - "help"
---



Normalize keyboard shortcut display, command palette help affordances, hint bars, and CLI-tip-adjacent host surfaces so they use the same visual language and wording.

## Why

The CLI is the product, and the host UI should teach the same commands and shortcuts without conflicting presentation.

## Gotchas

- Use key-chip and HintBar primitives for shortcut rows.
- Avoid adding visible instructional copy where a concise command affordance is enough.

## Variance

Completed as part of the 0023-0027 bundled stabilization pass; the work removed duplicated shortcut footer layout rather than creating new command surfaces.
