---
id: "0211"
title: "plexi app test CLI subcommand"
status: in-progress
estimate: "1h"
started_at: "2026-06-17T16:36:31Z"
blocked_by:
  - 210
gh_issue: []
area:
  - "cli/commands"
  - "infra/testing"
tags:
  - "v1"
  - "testing"
  - "tooling"
---


Agents and developers have no first-class way to run Python app tests. They must know to call `uv run pytest tests/` manually. A `plexi app test` subcommand makes the test runner discoverable via `plexi app --help` and gives the CLI a consistent entry point for the AppHarness flow.

## Scope

- Add `plexi app test [<app-path>]` subcommand that runs `uv run pytest tests/` inside the app directory
- Streams pytest output to stdout in real time (no buffering)
- Exits nonzero on test failure so CI and ship scripts can gate on it
- `--snapshot` flag: sets `PLEXI_UPDATE_SNAPSHOTS=1` env var (for future snapshot-update support)
- Help text names `AppHarness` and references `tests/test_app.py` as the expected test location

## Non-Scope

- Custom test runner or pytest plugin — this is a thin wrapper, not a framework
- Running Rust/scene tests (those stay under `cargo test` and `just scene`)
- Snapshot diffing UI

## Why

Making the test runner discoverable via the CLI means agents can find it from `plexi app --help` without reading any doc; the help text itself teaches the pattern.

## References

- `.stint/tasks/0210-app-init-appharnhess-scaffold.md` — produces the `tests/test_app.py` this command runs
- `sdk/python/plexi_sdk/testing.py` — `AppHarness` the tests use
- `src/cli/` — where the `app` subcommand lives
