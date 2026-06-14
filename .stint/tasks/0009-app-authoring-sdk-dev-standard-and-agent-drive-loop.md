---
id: "0009"
title: "App authoring: SDK dev standard and agent drive loop"
status: done
estimate: "8h"
actual: "6m"
started_at: "2026-06-13T07:46:23Z"
completed_at: "2026-06-13T07:51:43Z"
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

Agents need a repeatable render-inspect-act loop to build Plexi apps without reading Rust internals. This task unblocks 0011 (Core 9 sweep) and 0012 (verification harness).

## Scope

- Add a Justfile template to `plexi app init` scaffold output with targets: `dev` (watch + open), `health` (validate manifest + check SDK version), `test` (run app-level tests), `lint` (pyright/ruff).
- Add `plexi app dev` CLI command as a convenience wrapper that runs `just dev` in the app directory (or equivalent if no Justfile).
- Define the agent drive loop: render-inspect-act cycle that an LLM agent uses to iterate on an app (open app, capture state, edit code, reload, verify).
- Document the loop in `docs/SDK_QUICKSTART.md`.

## Gotchas

- Preserve local-first development; hosted marketplace services must not be required for local app authoring.
- No Justfile in SDK today (verified: no `justfile` references in `sdk/python/`). This is net-new scaffolding.

## References

- GitHub issue #257
- `docs/prm/app-framework-marketplace.md`
- `docs/SDK_QUICKSTART.md`

## Variance

Estimate 8h, actual 6m wall-clock. The estimate assumed designing the dev loop
from scratch; in practice the scaffold infra (`scaffold_python_app`, `app init`,
`app check` render-inspect) already existed, so this was additive wiring: a
generic `justfile.template`, a thin `plexi app dev` wrapper, and docs. Most
discovery happened in the shared batch analysis pass before `stint start`, so 6m
undercounts true effort. The `app_dev_workflow.toml` scene test is deferred to
0012, which owns scene coverage.
