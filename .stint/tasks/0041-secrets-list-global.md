---
id: "0041"
title: "Secrets CLI: global list and workspace fallback"
status: backlog
sprint: "s10"
estimate: 3h
blocked_by:
  - 147
gh_issue: ["2085"]
area: ["cli/commands", "host/secrets"]
tags: ["v1", "secrets", "cli"]
---

Make `plexi secret list --global` work and make `plexi secret list` outside a workspace fall back gracefully to user-scoped secrets.

Sequenced after `0147` because validation handoff reliability is the immediate ship-pipeline CLI blocker.

## Why

Secrets commands should be symmetric and usable from any directory.
