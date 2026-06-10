---
id: "0011"
title: "App authoring: Core 9 reference app sweep"
status: backlog
sprint: "s2"
estimate: 16h
blocked_by: ["0009", "0010"]
blocked_by_gh: []
gh_issue: []
area: ["apps/examples", "sdk/python", "host/permissions"]
tags: ["app-authoring", "core-9", "references"]
---

Make Core 9 apps clean references for common app patterns: list, form, table, network fetch, AI/chat, state persistence, and canvas fallback.

## Why

The fastest path for agents to generate good marketplace apps is copying first-party patterns that already look and behave correctly.

## Gotchas

- Core 9 only. Do not spend this sprint maintaining unrelated `apps/dev/` examples.
- Keep permission-gated flows explicit in examples that touch network or secrets.

## References

- `docs/prm/app-framework-marketplace.md`
