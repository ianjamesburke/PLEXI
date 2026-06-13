---
id: "0008"
title: "App authoring: scaffold and app dev polish"
status: done
estimate: "8h"
actual: "3m"
started_at: "2026-06-13T07:52:38Z"
completed_at: "2026-06-13T07:55:07Z"
sprint: "s2"
blocked_by: []
gh_issue:
  - "1962"
area:
  - "sdk/python"
  - "cli/commands"
tags:
  - "app-authoring"
  - "scaffold"
  - "sdk-v2"
---




Make the generated app and `app dev` path visually correct enough that agents start from a good default.

## Why

The marketplace depends on generated apps looking intentional without hand-placed pixels or one-off UI fixes.

## Scope

- Fix the `plexi app init` scaffold template (`sdk/python/plexi_sdk/templates/app_init.py`) to produce a visually polished default app.
- Ensure scaffolded `manifest.toml` includes all fields the marketplace validator expects (publisher, description, version) so `plexi marketplace publish` works without manual manifest editing.
- Scaffold template: `src/cli/app.rs:scaffold_python_app()` and `scaffold_agent_python_app()`.

## Gotchas

- Keep `view()` as the default normal-app hook.
- Do not improve `apps/dev/` throwaways unless they directly validate the SDK path.
- The marketplace (src/app/marketplace.rs) expects `[app.marketplace]` with a `publisher` field for submission. The scaffold does not currently emit this section. Add it as a commented-out placeholder so authors know it exists.

## References

- GitHub issue #1962
- `docs/prm/app-framework-marketplace.md`
- `src/cli/app.rs` (scaffold functions)
- `src/app/marketplace.rs` (publisher submission validation)

## Variance

Estimate 8h, actual 3m. The template was already polished and the validator
already existed, so the real work was a 6-line commented placeholder on three
manifests plus a docstring line. Correction: the host validator reads a
top-level `[marketplace]` section, not `[app.marketplace]` as this task's
Gotchas said — verified against `read_marketplace_manifest`. Estimate assumed
visual app-design work that 0166/0010 already cover.
