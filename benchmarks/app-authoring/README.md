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
| `plan.json` | (dry-run only) the exact step/argv sequence a live run executes |

`observations.jsonl` records are `{ts, kind, source, data}`. `source` is the
ground-truth channel — `pane_capture`, `pane_state`, `host_log`, `events`, `cli`,
or `protocol` (parent interventions). The parent observes the host, never the
child's self-report.

## Running a session

```bash
cd tools/e2e_authoring
# validate plumbing anywhere (no host, no display):
uv run --python 3.12 plexi-e2e run fixtures/counter.toml --dry-run \
  --sessions-root ../../benchmarks/app-authoring/sessions

# live pilot (needs the channel installed + a display + child-agent creds):
uv run --python 3.12 plexi-e2e run fixtures/counter.toml --channel e2e --fresh-profile \
  --sessions-root ../../benchmarks/app-authoring/sessions
```

See `sessions/INDEX.md` for the session list and `tools/e2e_authoring/AGENTS.md`
for the pipeline contract.
