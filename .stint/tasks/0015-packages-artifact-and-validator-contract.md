---
id: "0015"
title: "Packages: artifact and validator contract"
status: done
estimate: "16h"
actual: "105m"
started_at: "2026-06-11T17:53:36Z"
completed_at: "2026-06-11T19:38:06Z"
sprint: "s3"
blocked_by:
  - 13
gh_issue: []
area:
  - "infra/build"
  - "cli/commands"
  - "sdk/pgap"
tags:
  - "packages"
  - "validation"
  - "marketplace"
---



Define package artifacts and validation for manifest, contents, checksums, runtime, capabilities, and obvious bypass patterns.

## Why

Hosted marketplace work should consume the same validator that local package/install already trusts.

## Gotchas

- Fail closed on missing manifest, unknown capabilities, path traversal, symlink escapes, unsupported runtime, and mismatched metadata.
- Keep command names flexible, but preserve validate, inspect, install, run locally.

## Variance

Actual time was much lower than estimate because the package validator and CLI paths shared existing manifest/runtime plumbing.

## References

- `docs/prm/app-framework-marketplace.md`
