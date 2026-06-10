---
id: "0032"
title: "v1 UI: QuickNote atomic page transitions"
status: backlog
sprint: "s5"
estimate: 6h
blocked_by: ["0024"]
blocked_by_gh: []
gh_issue: ["2133"]
area: ["ui/overlays", "ui/widgets"]
tags: ["v1", "ui", "quick-note", "host-ui-kit"]
---

Make QuickNote compose, destination, and submenu transitions feel atomic by keeping shell, content, and size changes under one stable overlay owner.

## Why

Visible one-frame flashes make the modal system feel unstable right where users capture notes quickly.

## Gotchas

- Preserve same-frame input responsiveness.
- Do not expand scope into command substitution, persistence, or quick-note config schema.
