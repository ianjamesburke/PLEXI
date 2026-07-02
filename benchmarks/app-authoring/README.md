# App-authoring benchmark — session corpus

Real agent app-building sessions, captured by the agent-drives-agent E2E runner
(`tools/e2e_authoring/`, stint 0331). A parent process plays a non-technical user;
a child coding agent tries to build a Plexi app from a vague, user-realistic
prompt. Each run leaves one directory under `sessions/` in the capture format
below. This corpus is both a regression benchmark (as the SDK evolves) and a
case-study library for fixing authoring DX.

The runner and this format are stint 0331. The prompt library, scorecards, and
the committed baseline sweep are stint 0215 — it accumulates sessions here.

## Capture format

Every `sessions/<session-id>/` directory contains:

| File | What it holds |
|---|---|
| `manifest.json` | session id, channel, binary, versions, fixture ref, wall-clock, `dry_run` flag |
| `prompt.toml` | verbatim copy of the fixture the child was given |
| `transcript.md` | the child's terminal transcript, captured over observation rounds |
| `observations.jsonl` | parent ground-truth events (one JSON object per line) |
| `friction.md` | rigorous friction notes — where the child guessed or the scaffold misled it |
| `outcome.json` | structured outcome: `worked` \| `partial` \| `failed`, where it stalled |
| `host.log` | the slice of `plexi.log` covering the session window |
| `scorecard.json` | the derived, comparable score for this session (see below) |
| `plan.json` | (dry-run only) the exact step/argv sequence a live run executes |

`observations.jsonl` records are `{ts, kind, source, data}`. `source` is the
ground-truth channel — `pane_capture`, `pane_state`, `host_log`, `events`, `cli`,
or `protocol` (parent interventions). The parent observes the host, never the
child's self-report.

Every session is version-stamped: `manifest.json` carries a `versions` block
(`cli`, `sdk`, `channel`) so scores stay comparable across time. `cli` is `null`
on a dry run (no installed binary was queried); `sdk` is the Python SDK version
from `sdk/python/pyproject.toml`.

## Scorecard

`scorecard.json` is a **projection** — never a second source of truth. It reads
only the files above and distils them into one flat, comparable record:

| Field | Source |
|---|---|
| `outcome` / `stalled_at` | `outcome.json` (`plan-only` for a dry run) |
| `wall_clock_secs` | manifest |
| `parent_turns` | parent interventions (manifest) |
| `child_turns` | observation rounds that produced visible child output |
| `commands_used` / `errors` | `outcome.json` (ground-truth observations) |
| `lines_of_code` | a `code_metrics` observation the live runner records over the child's workspace |
| `versions` | `cli` / `sdk` / `channel` stamp (manifest) |
| `timings` | `host_ready_secs` (init) → `first_child_output_secs` (first frame) → `first_interactive_secs` (first round-trip) |

Timings are anchored at the earliest observation and derived from observation
timestamps. `first_interactive_secs` stays `null` until a built app emits an
input→state marker into the host log — anything not derivable from the raw
session is `null`, never guessed. Rebuild any scorecard from its raw capture
with `plexi-e2e score <session-dir>`.

## Prompt library

`tools/e2e_authoring/fixtures/` holds a graded suite of user-realistic prompts —
each one says what a *user* would say, with no command names, file paths, or SDK
symbols:

| Fixture | Difficulty | Ask |
|---|---|---|
| `counter` | easy | a number on screen that goes up on keypress, with reset |
| `file-lister` | easy | a list of the files in a folder |
| `form` | medium | a note form (title + body + save) that keeps its notes |
| `log-viewer` | medium | a live tail of a log file, newest at the bottom |
| `assistant` | hard | an ask-a-question box that replies with AI, conversation on screen |

## Running

```bash
# one session, plumbing check anywhere (no host, no display):
just e2e-session tools/e2e_authoring/fixtures/counter.toml e2e --dry-run

# one live session (needs the channel installed + a display + child-agent creds):
just e2e-session tools/e2e_authoring/fixtures/counter.toml e2e --fresh-profile

# the whole suite + regenerated index — the baseline sweep:
just e2e-baseline e2e --dry-run        # structural v0 baseline (committed here)
just e2e-baseline e2e --fresh-profile  # live sweep on a machine with a display

# regenerate INDEX.md over all captured sessions:
just e2e-session ... ; plexi-e2e index --sessions-root benchmarks/app-authoring/sessions
```

The committed sessions here are the **dry-run structural baseline** (marked
`dry-run` / `plan-only` in the index): they prove the capture format, scorecard,
and version stamp end-to-end without a live host. The live baseline is deferred —
it needs a display (host GUI) and child-agent credentials. Run
`just e2e-baseline e2e --fresh-profile` on such a machine to fill in live scores.

See `sessions/INDEX.md` for the session list and `tools/e2e_authoring/AGENTS.md`
for the pipeline contract.
