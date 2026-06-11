---
id: "0027"
title: "v1 UI: regression gallery and coverage"
status: in-progress
estimate: "8h"
started_at: "2026-06-11T17:57:59Z"
sprint: "s5"
blocked_by:
  - 24
  - 25
  - 26
gh_issue: []
area:
  - "ui/overlays"
  - "ui/widgets"
  - "infra/test"
tags:
  - "v1"
  - "ui"
  - "tests"
---


Add gallery states and focused regression coverage for v1 host/app-platform chrome: modals, shortcut hints, permission grants, package trust sheets, install confirmations, disabled states, and danger states.

## Why

The UI kit prevents drift only if important states remain easy to review and hard to regress.

## Gotchas

- Cover small-pane and normal-pane sizes where text fitting can fail.
- Prefer focused HostHarness or snapshot-style tests where available.
