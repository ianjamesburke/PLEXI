---
id: "0008"
title: "App authoring: scaffold and app dev polish"
status: backlog
sprint: "s2"
estimate: 8h
blocked_by: ["0007"]
blocked_by_gh: []
gh_issue: ["1962"]
area: ["sdk/python", "cli/commands"]
tags: ["app-authoring", "scaffold", "sdk-v2"]
---

Make the generated app and `app dev` path visually correct enough that agents start from a good default.

## Why

The marketplace depends on generated apps looking intentional without hand-placed pixels or one-off UI fixes.

## Gotchas

- Keep `view()` as the default normal-app hook.
- Do not improve `apps/dev/` throwaways unless they directly validate the SDK path.

## References

- GitHub issue #1962
- `docs/prm/app-framework-marketplace.md`
