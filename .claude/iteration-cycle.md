# Iteration Cycle — PLEXI

Read at session start alongside DEV_LOG.

## Branch flow (alpha train, batched promotion)
- `alpha` — active dev. Every PR lands here. **User lives on alpha through v3.5.**
- `beta` — batched promotion target. Only updated when user says "promote alpha." Soaks 24–72h.
- `main` — stable. Only updated after beta soak.

Never push directly to `main` or `beta`. Feature branches: `feature/<issue-number>-short-description`. Sub-agent work: `isolation: "worktree"` off `alpha`, PR back to `alpha`.

Hotfixes are the only main-direct work — branch from `main`, PR to `main`, cherry-pick back to `alpha`.

## Build + install for testing
- Alpha (active): `just install-alpha`
- Stable (only when promoting beta → main): `just install`

After every code change, install for the active branch before reporting done. **Orchestrator runs `just install-alpha` after every squash-merge** — the user does not have to.

## Smoke test (required before claiming done)
```
just install-alpha
scripts/smoke-test.sh
```

Smoke test (1) feeds PGAP Init to every installed app and asserts `ready` within 3s, (2) launches the host for 2s and scans the log for panics. Failure = install is broken = task is not done.

## Logs
- Alpha: `~/.plexi-alpha/plexi.log`
- Stable: `~/.plexi/plexi.log`

App logs forward as `app::<app_id>` entries. Check the log first when debugging.

## PR requirements
Every PR description must include:
1. **`Breaks if:`** — concrete observable symptom (visible UI failure, missing log line, specific broken behavior)
2. **Human verification checklist** — 3–5 numbered steps the user runs on alpha to confirm it works
3. **Test added** — file + test name + one-line description

PRs missing any of the three are rejected. The orchestrator (this session) is responsible for writing all three before dispatching the sub-agent — never offload to the agent.

## Test discipline
- Test-first. The failing test is the starting state.
- Any new `HostCommand` or `HostEffect` needs a `HostHarness` test.
- A stub that makes the test pass is an implementation. Don't `todo!()`.
- `todo!()` and `unimplemented!()` are banned outside `#[cfg(test)]` (enforced by clippy deny).

## Sub-agent dispatch (orchestrator workflow)
See `docs/specs/process/release-orchestration.md` for the full spec.

Per release:
1. Decompose each milestone issue into a brief — file paths, defining test, SDK change, smoke invariant, **human-verification steps**.
2. Group by file overlap — same-file issues serialize, others parallelize.
3. Dispatch one sub-agent per issue via `Agent` with `isolation: "worktree"`, `run_in_background: true`.
4. Review diffs (not summaries) when agents report done.
5. Pull worktree, run smoke test, **squash-merge** to `alpha` if clean (`gh pr merge <N> --squash --delete-branch` — never `--merge` or `--rebase`; squash so each PR is one revertible commit).
6. **Orchestrator runs `just install-alpha` immediately after merge**, then pings user with the human-verification checklist. Green → next dispatch. Red → `git revert <squash-sha>` on alpha + new dispatch with diagnosis.
7. After all PRs in milestone merged + verified: run release-level checklist from `docs/specs/releases/v3.x.md`.
8. After alpha contains v3.5 (or whenever user calls it): batch-promote alpha → beta, soak, → main.

## Three-strike rule
If a sub-agent fails on an issue 3 times, orchestrator takes it directly. Repeated failure = bad brief, not bad agent.

## Verification gate
A merged PR without a human-verified result blocks subsequent dispatches in the same theme. Other themes continue in parallel. The user is the QA — never assume green without their confirmation.

## Test credentials
Per global convention: `STAGING_TEST_EMAIL` / `STAGING_TEST_PASSWORD` / `PROD_TEST_EMAIL` / `PROD_TEST_PASSWORD` in `.env`. Never hardcode.
