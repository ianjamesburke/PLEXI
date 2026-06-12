---
id: "0029"
title: "v1: GitHub issue hygiene and stint alignment"
status: done
estimate: "8h"
completed_at: "2026-06-12T06:25:49Z"
sprint: "s14"
blocked_by:
  - 27
gh_issue: []
area:
  - "infra/triage"
tags:
  - "v1"
  - "issues"
  - "stint"
---


Walk open GitHub issues area by area, close stale tickets, move future work behind v1/v2 labels, and update stint task links where an issue remains the implementation ticket.

## Why

Until stint fully replaces issue planning, both systems need to agree about what is current, parked, blocked, or obsolete.

## Gotchas

- Do issue cleanup one area at a time.
- Do not preserve old issue links in stint tasks just because they are related.
