---
id: "0019"
title: "Marketplace: publisher submission and review flow"
status: in-progress
estimate: "16h"
started_at: "2026-06-13T01:57:12Z"
sprint: "s4"
blocked_by:
  - 18
  - 15
gh_issue: []
area:
  - "infra/server"
  - "cli/commands"
tags:
  - "marketplace"
  - "publisher"
  - "review"
---


Add the publisher path for package validation, submission, automated checks, and human review for native-process apps.

## Why

Marketplace trust depends on a real review lane, especially while Python apps are reviewed native processes rather than sandboxed WASM.

## Gotchas

- Use the local package validator from task `0015`; do not create a parallel hosted validator with different rules.

## References

- `docs/prm/app-framework-marketplace.md`
- `docs/prm/marketplace-hosted.md`
