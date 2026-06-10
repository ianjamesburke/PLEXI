---
id: "0015"
title: "Packages: artifact and validator contract"
status: backlog
sprint: "s3"
estimate: 16h
blocked_by: ["0013"]
blocked_by_gh: []
gh_issue: []
area: ["infra/build", "cli/commands", "sdk/pgap"]
tags: ["packages", "validation", "marketplace"]
---

Define package artifacts and validation for manifest, contents, checksums, runtime, capabilities, and obvious bypass patterns.

## Why

Hosted marketplace work should consume the same validator that local package/install already trusts.

## Gotchas

- Fail closed on missing manifest, unknown capabilities, path traversal, symlink escapes, unsupported runtime, and mismatched metadata.
- Keep command names flexible, but preserve validate, inspect, install, run locally.

## References

- `docs/prm/app-framework-marketplace.md`
