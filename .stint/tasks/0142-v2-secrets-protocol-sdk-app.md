---
id: "0142"
title: "v2 secrets: protocol SDK app conversion"
status: backlog
sprint: "s30"
estimate: 8h
blocked_by:
  - 41
  - 42
gh_issue: ["674"]
area: ["host/secrets"]
tags: ["v2", "secrets", "sdk"]
---

Convert SecretsApp into a protocol-backed SDK app after v1 CLI and app trust behavior is stable.

## v1 Decision

Not a v1 blocker. v1 secrets work is the CLI and scope refinement lane; converting the app to a protocol SDK app is a portability cleanup after the trust model is stable.
