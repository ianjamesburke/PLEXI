---
id: "0041"
title: "Secrets CLI: global list and workspace fallback"
status: done
estimate: "3h"
actual: "4m"
started_at: "2026-06-11T08:29:41Z"
completed_at: "2026-06-11T08:33:05Z"
sprint: "s10"
blocked_by:
  - 147
gh_issue:
  - "2085"
area:
  - "cli/commands"
  - "host/secrets"
tags:
  - "v1"
  - "secrets"
  - "cli"
---



Make `plexi secret list --global` work and make `plexi secret list` outside a workspace fall back gracefully to user-scoped secrets.

Sequenced after `0147` because validation handoff reliability is the immediate ship-pipeline CLI blocker.

## Why

Secrets commands should be symmetric and usable from any directory.

Variance: implementation was shorter than estimated because the issue body already mapped the exact CLI parser, dispatch, and list-scope changes needed.
