---
id: "0017"
title: "Permissions: manager app and yellow-state routing"
status: in-progress
estimate: "16h"
started_at: "2026-06-11T17:54:22Z"
sprint: "s3"
blocked_by:
  - 16
gh_issue: []
area:
  - "host/permissions"
  - "sdk/pgap"
tags:
  - "permissions"
  - "trust"
  - "capabilities"
---


Bring permission management and yellow-state routing into the trust foundation so package installs and app runtime decisions are auditable.

## Why

Marketplace installs need a visible place to inspect and revise grants, and app runtime consent should not disappear into one-off prompts.

## Gotchas

- Capability declarations must match actual powers.
- Brokered network belongs behind explicit host capability paths.

## References

- `docs/prm/app-framework-marketplace.md`
