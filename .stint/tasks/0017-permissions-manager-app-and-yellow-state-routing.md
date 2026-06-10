---
id: "0017"
title: "Permissions: manager app and yellow-state routing"
status: backlog
sprint: "s3"
estimate: 16h
blocked_by: ["0016"]
blocked_by_gh: []
gh_issue: ["867", "868", "412"]
area: ["host/permissions", "sdk/pgap"]
tags: ["permissions", "trust", "capabilities"]
---

Bring permission management and yellow-state routing into the trust foundation so package installs and app runtime decisions are auditable.

## Why

Marketplace installs need a visible place to inspect and revise grants, and app runtime consent should not disappear into one-off prompts.

## Gotchas

- Capability declarations must match actual powers.
- Brokered network belongs behind explicit host capability paths.

## References

- GitHub issues #867, #868, #412
- `docs/prm/app-framework-marketplace.md`
