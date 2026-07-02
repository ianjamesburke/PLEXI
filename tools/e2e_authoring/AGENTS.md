# tools/e2e_authoring — Agent Contract

**Read before editing:** this file, the root `AGENTS.md`, and
`.agents/skills/drive-host/SKILL.md` (the live-drive loop this composes).

## Scope

The agent-drives-agent E2E runner (stint 0331). A parent process plays a
non-technical user driving a child coding agent through a real Plexi
app-building session, and captures the session in the format under
`benchmarks/app-authoring/`. Composes existing CLI primitives
(`host start/stop/status`, `pane list/new/send/key/capture/state`,
`events subscribe`, the channel `plexi.log`) — it adds orchestration, not new
host behavior.

## Rules

- **Compose, don't reinvent.** Every host interaction goes through the Plexi CLI
  via `plexi_cli.PlexiCli`. Never talk to the host socket directly.
- **Env discipline is centralized.** All drive commands run under
  `env.drive_env()`, which strips the inherited pane identity and pins
  `PLEXI_SOCKET` / `PLEXI_CHANNEL` to the host-under-test. Never build a drive
  env by hand at a call site.
- **The parent stays a user.** Prompts and answers carry no command names, file
  paths, or SDK symbols. That contract lives in `protocol.py`; keep it.
- **Observe ground truth.** Outcome and friction come from host observations
  (`pane_capture` / `pane_state` / `host_log` / `events`), never the child's
  self-report.
- **Required config throws.** Missing fixture fields and empty channel/binary
  raise; no silent defaults.
- **Isolation is per-channel.** `host start` binds to the installed binary's own
  channel (src/cli/host.rs) — it cannot be pointed at an arbitrary profile.
  Concurrent non-interfering sessions need distinct channels; sequential
  sessions on one channel are isolated by `--fresh-profile` + unique session dirs.

## Python

`uv` only, `requires-python >=3.11`. Run tests: `uv run --python 3.12 --extra dev python -m pytest`
from this directory (or `just e2e-test` from the repo root).

## Layout

- `src/plexi_e2e/` — `config` (fixtures), `env` (drive-env discipline),
  `plexi_cli` (CLI wrappers), `capture` (session dir writer), `protocol`
  (user-proxy rules), `runner` (orchestration), `versions` (CLI/SDK/channel
  stamp), `scorecard` (derived per-session score), `index` (INDEX.md generator),
  `__main__` (`plexi-e2e` CLI: `run` / `score` / `index`).
- `fixtures/` — committed prompt library (user-realistic, graded easy→hard, no
  implementation hints). The `test_fixtures` suite guards that contract corpus-wide.
- `tests/` — pytest; command construction, dry-run, scorecard, and index are all
  fully host-free.

`scorecard` and `index` are **projections** — they read only what the runner
already captured and never store a second source of truth. Every session ends
with a `scorecard.json`; the baseline sweep (`just e2e-baseline`) runs the whole
fixture library and regenerates the index.

Sessions are written to `benchmarks/app-authoring/sessions/` — that directory and
its format are the interchange the benchmark suite (stint 0215) accumulates. The
committed sessions there are the dry-run structural baseline; the live sweep is
deferred to a machine with a display + child-agent credentials.
