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

**Known smoke-test issue (track separately):** `scripts/smoke-test.sh` currently points at `~/.plexi-v3/` and `/usr/local/bin/plexi-v3`, not alpha paths. On a machine that only ever installs alpha, the host check silently skips. File a follow-up issue to either name-derive the paths or add an `install-v3`-only branch.

## Cargo test invocation
There is no `lib` target in this repo. Run tests with `cargo test --bin plexi`, **not** `cargo test --lib` or bare `cargo test`. Sub-agents must use the `--bin plexi` form.

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

## Research before implementing
Before writing code, sub-agents must briefly skim the docs of any non-trivial dependency they're about to use (egui, eframe, serde, tokio, CoreAudio/CoreMIDI, AVFoundation, etc.). The goal is one or two minutes of "is this still the right API in this version?" — not deep research. Verify versioned APIs against the actual `Cargo.lock` / `pyproject.toml` versions, not training-data assumptions. Specifically check: deprecated methods, signature changes, recommended idioms vs. older patterns. Sub-agent briefs MUST include "research dependencies first" as an explicit step.

## Pre-dispatch audit (orchestrator-only — non-negotiable)
Before dispatching any sub-agent, the orchestrator runs a stale-state audit on the issue:
1. `gh issue view <N> --json state,closedAt` — confirm `OPEN`.
2. `git log --oneline --all -200 | grep -iE "<issue-number>|<keyword>"` — look for shipped PRs without a `Closes #N` trailer (orphan-open issues).
3. Spot-check the codebase: grep for the names of types/functions the issue would introduce. If they exist, the work likely shipped.
4. If shipped: close issue + tick milestone box + skip dispatch. If not shipped: dispatch.

This audit takes under a minute. Skipping it wastes a sub-agent run on shipped work — happened with #312, #314, #317 during the v3.1 kickoff.

## Sub-agent dispatch (orchestrator workflow)
See `docs/specs/process/release-orchestration.md` for the full spec.

Per release:
1. Decompose each milestone issue into a brief — file paths, defining test, SDK change, smoke invariant, **human-verification steps**.
2. Group by file overlap — same-file issues serialize, others parallelize.
3. Dispatch one sub-agent per issue via `Agent` with `isolation: "worktree"`, `run_in_background: true`.
4. Review diffs (not summaries) when agents report done.
5. Pull worktree, run smoke test, **squash-merge** to `alpha` if clean. Use `gh pr merge <N> --squash --delete-branch --body "$(gh pr view <N> --json body -q .body)"` so the squash commit body keeps the full PR description. Never `--merge` or `--rebase`. **Issue auto-close does NOT fire on alpha merges** because `main` is the default branch — every alpha PR's linked issue must be closed manually with `gh issue close <N> --comment "..."`. This stays true until v3.x ships and alpha gets promoted to main.
6. **Orchestrator runs `just install-alpha` immediately after merge**, then pings user with the human-verification checklist. Green → next dispatch. Red → `git revert <squash-sha>` on alpha + new dispatch with diagnosis.
7. After all PRs in milestone merged + verified: run release-level checklist from `docs/specs/releases/v3.x.md`.
8. After alpha contains v3.5 (or whenever user calls it): batch-promote alpha → beta, soak, → main.

## Three-strike rule
If a sub-agent fails on an issue 3 times, orchestrator takes it directly. Repeated failure = bad brief, not bad agent.

## Verification gate
A merged PR without a human-verified result blocks subsequent dispatches in the same theme. Other themes continue in parallel. The user is the QA — never assume green without their confirmation.

## Test credentials
Per global convention: `STAGING_TEST_EMAIL` / `STAGING_TEST_PASSWORD` / `PROD_TEST_EMAIL` / `PROD_TEST_PASSWORD` in `.env`. Never hardcode.
