---
id: "0022"
title: "Marketplace: Plexi AI subscription via ai.query"
status: backlog
sprint: "s4"
estimate: 12h
blocked_by: ["0020"]
blocked_by_gh: []
gh_issue: []
area: ["host/ai", "infra/server"]
tags: ["marketplace", "ai", "subscription"]
---

Define the Plexi AI subscription backend for `ai.query` as separate from app purchase and local app execution.

## Why

Apps should call `ai.query`; the host decides whether the backend is local Ollama, user-owned keys, or a Plexi-managed subscription.

## Gotchas

- The subscription must not be a prerequisite for local apps.
- Request allowance numbers belong in billing spec, not app framework code.

## References

- `docs/prm/app-framework-marketplace.md`
