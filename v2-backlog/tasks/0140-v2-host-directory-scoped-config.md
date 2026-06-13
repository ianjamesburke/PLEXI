---
id: "0140"
title: "v2 host config: directory-scoped overrides"
status: backlog
sprint: "s30"
estimate: 6h
blocked_by:
  - 84
gh_issue: ["1556"]
area: ["host/config"]
tags: ["v2", "config", "context"]
---

Allow project-local config overrides such as directory-specific color schemes without creating ambiguous config precedence.

## v1 Decision

Not a v1 blocker. v1 needs channel/profile config discipline and docs; directory-scoped overrides add a new precedence model and belong after release.
