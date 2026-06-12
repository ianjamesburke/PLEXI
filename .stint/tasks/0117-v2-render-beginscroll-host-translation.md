---
id: "0117"
title: "v2 render: host-managed BeginScroll"
status: backlog
sprint: "s26"
estimate: 8h
blocked_by:
  - 80
gh_issue: ["1148"]
area: ["sdk/python", "sdk/pgap"]
tags: ["v2", "pgap", "rendering"]
---

Replace manual app-side scroll offset math with a host-applied clipped coordinate transform for scroll regions.

## v1 Decision

Not a v1 blocker. v1 app authoring should avoid overlap and prove normal L1 layouts; host-managed scroll translation is a v2 protocol semantics cleanup.
