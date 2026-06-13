---
id: "0009"
title: "App authoring: SDK dev standard and agent drive loop"
status: todo
estimate: "8h"
sprint: "s2"
blocked_by: []
gh_issue:
  - "257"
area:
  - "sdk/python"
  - "infra/build"
  - "cli/commands"
tags:
  - "app-authoring"
  - "sdk-v2"
  - "agent-loop"
---


Standardize app-author commands around a Justfile-backed init, health, test, lint, and agent drive loop.

## Why

Agents need a repeatable render-inspect-act loop to build Plexi apps without reading Rust internals.

## Gotchas

- Preserve local-first development; hosted marketplace services must not be required for local app authoring.

## References

- GitHub issue #257
- `docs/prm/app-framework-marketplace.md`
