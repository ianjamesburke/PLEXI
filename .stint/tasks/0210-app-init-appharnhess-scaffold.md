---
id: "0210"
title: "plexi app init scaffolds AppHarness test"
status: done
estimate: "1h"
actual: "45m"
started_at: "2026-06-17T16:26:43Z"
completed_at: "2026-06-17T17:10:48Z"
blocked_by: []
gh_issue: []
area:
  - "cli/commands"
  - "infra/testing"
  - "sdk/python"
tags:
  - "v1"
  - "testing"
  - "tooling"
---



`plexi app init <name>` currently produces a manifest and `app.py` but no test file. An agent starting on a new app has no example of AppHarness and re-derives the pattern from docs each time.

## Scope

- Add `tests/test_app.py` to the `plexi app init` output template — a working AppHarness example that runs one frame, calls `save_snapshot`, and asserts `assert_no_overlap`
- The generated test must be runnable with `uv run pytest tests/` out of the box (no extra deps beyond `plexi_sdk`)
- Update any `plexi app init` snapshot or integration tests to expect the new file

## Non-Scope

- `plexi app test` CLI subcommand (that is task 0211)
- Backfilling tests for existing Core 9 apps (separate pass)

## Why

Co-locating a working test with the scaffold means agents learn the AppHarness pattern from the code they're already reading, not from CLAUDE.md or skills that rot.

## References

- `sdk/python/plexi_sdk/testing.py` — `AppHarness` and `render_draw_commands` implementations
- `.agents/skills/testing/SKILL.md` — Step 3 AppHarness section (the pattern the scaffold should exemplify)
- `src/cli/` — `app init` subcommand where the template lives
