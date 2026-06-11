---
id: "0014"
title: "Trust: host-mediated Assistant capabilities"
status: in-progress
estimate: "16h"
started_at: "2026-06-11T10:36:59Z"
sprint: "s3"
blocked_by:
  - 13
gh_issue: []
area:
  - "host/permissions"
  - "host/ai"
  - "sdk/pgap"
tags:
  - "trust"
  - "assistant"
  - "capabilities"
---


Replace Assistant CLI subprocess control tools with host-mediated APIs and explicit capability declarations.

## Why

The first-party Assistant should model the same trust contract that marketplace apps will be judged against.

## Gotchas

- Declare every pane, app, terminal, and AI power the Assistant uses.
- Add denial tests for host APIs reachable by apps.

## References

- `docs/prm/app-framework-marketplace.md`
- `apps/assistant/manifest.toml`
