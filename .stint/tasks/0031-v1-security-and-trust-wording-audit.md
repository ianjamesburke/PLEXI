---
id: "0031"
title: "v1: security and trust wording audit"
status: backlog
sprint: "s6"
estimate: 8h
blocked_by: ["0028"]
blocked_by_gh: []
gh_issue: []
area: ["host/permissions", "infra/docs"]
tags: ["v1", "security", "trust"]
---

Audit product, docs, install screens, trust labels, and marketplace wording so v1 accurately describes Python apps as reviewed native processes with capability-gated host APIs, not sandboxed code.

## Why

Trust labeling is only useful if the language is blunt about what is enforced now and what is deferred to the v2 WASM runtime.

## Gotchas

- `Sandboxed WASM` must not appear as an available v1 trust label unless enforcement exists.
- Keep v2 runtime-lane wording clearly separate from v1 guarantees.
