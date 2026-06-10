---
id: "0042"
title: "Secrets CLI: plural command alias"
status: backlog
sprint: "s10"
estimate: 1h
blocked_by:
  - 147
gh_issue: ["2084"]
area: ["cli/commands", "host/secrets"]
tags: ["v1", "secrets", "cli", "bundle"]
---

Add `plexi secrets` as an alias for `plexi secret`.

Sequenced after `0147` because validation handoff reliability is the immediate ship-pipeline CLI blocker.

## Why

Users naturally try the plural noun for a collection of secrets.
