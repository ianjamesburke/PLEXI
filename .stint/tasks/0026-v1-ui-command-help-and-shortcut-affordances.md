---
id: "0026"
title: "v1 UI: command help and shortcut affordances"
status: backlog
sprint: "s5"
estimate: 8h
blocked_by: ["0024"]
blocked_by_gh: []
gh_issue: []
area: ["ui/overlays", "cli/commands"]
tags: ["v1", "ui", "shortcuts", "help"]
---

Normalize keyboard shortcut display, command palette help affordances, hint bars, and CLI-tip-adjacent host surfaces so they use the same visual language and wording.

## Why

The CLI is the product, and the host UI should teach the same commands and shortcuts without conflicting presentation.

## Gotchas

- Use key-chip and HintBar primitives for shortcut rows.
- Avoid adding visible instructional copy where a concise command affordance is enough.
