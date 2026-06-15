---
id: "0031"
title: "v1: security and trust wording audit"
status: done
estimate: "8h"
actual: "45m"
started_at: "2026-06-15T08:05:48Z"
completed_at: "2026-06-15T17:44:37Z"
blocked_by:
  - 28
gh_issue: []
area:
  - "host/permissions"
  - "infra/docs"
tags:
  - "v1"
  - "security"
  - "trust"
---





Audit product, docs, install screens, trust labels, and marketplace wording so v1 accurately describes Python apps as reviewed native processes with capability-gated host APIs, not sandboxed code.

## Why

Trust labeling is only useful if the language is blunt about what is enforced now and what is deferred to the v2 WASM runtime.

## Gotchas

- `Sandboxed WASM` must not appear as an available v1 trust label unless enforcement exists.
- Keep v2 runtime-lane wording clearly separate from v1 guarantees.

## Variance

Audit found the public security wording already mostly correct. This pass added regression coverage so docs do not claim Python sandboxing later.
