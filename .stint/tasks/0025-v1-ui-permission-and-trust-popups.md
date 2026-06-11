---
id: "0025"
title: "v1 UI: permission and trust popups"
status: in-progress
estimate: "16h"
started_at: "2026-06-11T17:57:59Z"
sprint: "s5"
blocked_by:
  - 17
  - 23
gh_issue: []
area:
  - "host/permissions"
  - "ui/overlays"
  - "ui/widgets"
tags:
  - "v1"
  - "ui"
  - "permissions"
  - "trust"
---


Rework permission grant, package trust, install confirmation, and marketplace warning popups on shared Host UI Kit primitives.

## Why

The v1 trust model is only credible if permission and package decisions are clear, consistent, and visually distinct from ordinary confirmation dialogs.

## Gotchas

- Trust labels must stay blunt: reviewed native process, first-party core, and sandboxed WASM only after enforcement exists.
- Dangerous or irreversible choices need clear danger-state styling and logging.
