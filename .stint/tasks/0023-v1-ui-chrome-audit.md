---
id: "0023"
title: "v1 UI: host chrome audit"
status: done
estimate: "6h"
actual: "13m"
started_at: "2026-06-11T17:57:52Z"
completed_at: "2026-06-11T20:16:36Z"
sprint: "s5"
blocked_by:
  - 22
gh_issue: []
area:
  - "ui/overlays"
  - "ui/widgets"
tags:
  - "v1"
  - "ui"
  - "host-ui-kit"
---




Audit remaining host chrome for one-off modal shells, raw shortcut labels, custom permission prompts, package/install confirmations, and trust-warning UI that should move onto the Host UI Kit.

## Why

The completed Host UI Kit gives Plexi shared primitives, but v1 needs the visible product surfaces to actually consume them consistently.

## Gotchas

- Start from code, not screenshots.
- Do not refactor every egui call; focus on v1-visible host chrome.

## Variance

Completed as part of the 0023-0027 bundled stabilization pass; existing Host UI Kit primitives covered the visible audit findings.
