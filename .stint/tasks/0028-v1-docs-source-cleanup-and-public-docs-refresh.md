---
id: "0028"
title: "v1: docs source cleanup and public docs refresh"
status: backlog
sprint: "s14"
estimate: 8h
blocked_by:
  - 27
gh_issue: []
area: ["infra/docs"]
tags: ["v1", "docs", "cleanup"]
---

Remove stale planning docs, regenerate public docs, and align README, website docs, SDK docs, PGAP reference, and security model with the v1 app-platform contract.

## Why

v1 should not ship with parallel roadmap history or docs that imply unsupported app, marketplace, or sandbox behavior.

## Gotchas

- Delete superseded docs instead of archiving them beside the canonical PRM.
- Keep generated CLI docs in sync with the current binary.
