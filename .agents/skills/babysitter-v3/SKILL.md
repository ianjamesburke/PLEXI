---
name: babysitter-v3
description: "Unattended multi-hour stint queue runner. A head pane routes; worker panes implement batches into PRs via /implement-stint-v2; fresh tester panes on a different vendor validate each PR against the real pr-install build; PASS merges. Roles fall back across Claude, Codex and Antigravity (agy) usage pools. Triggered by /babysitter-v3 <stint ids...> or /babysitter-v3 resume."
source: local
date_added: "2026-10-15"
---

# Babysitter v3

You are the **head**: a router, never a coder. You spawn panes, brief them, read their slots, relay verdicts and keep the run state on disk. You do not edit code, run git, build, or merge.

Settings (roles, aliases, limits, footers, paths) live in `RUN_CONFIG.toml` next to this file. Read it at start and before every spawn. Never restate its values in briefs or state files.

Reserved child queues may explicitly select an alias from the size role's `_explicit` allowlist using `route --worker-alias`. Explicit lists never participate in automatic selection or exhaustion fallback. Keep the chosen alias in the batch record so the same route and independent-vendor gates apply.

## Hosted Rust verification

`RUN_CONFIG.toml` `[ci].rust_host = "after_alpha"` defers the Rust host workflow
until nightly verification after Alpha landing. Workers publish `pr:done` after
their required local gates; neither worker nor head waits or polls for hosted
Rust tests, Clippy or Linux. Independent testers still validate the exact PR head.
The phase-aware gates and evidence schema live in [TESTING.md](../../../../src/testing/TESTING.md#the-gate). Batch records use relevant scoped tests plus release build, Clippy and independent installed affected-behavior verification. Full-suite evidence is required only for the final combined candidate. Never label deferred checks as passed.

The policy reads workflow identity as well as check name and excludes only
`Rust host` checks under this explicit setting. It also excludes check runs
identified by GitHub as the `coderabbitai` app; ambiguous identities remain gated.
Missing hosted checks are expected; other PR checks still must pass. A stale head, missing local evidence, install
mismatch or failed independent verdict still blocks integration. The setting is
policy, not a self-attested waiver. Earlier in-flight Rust checks may finish but
do not hold the queue. Do not disable unrelated workflows or trigger a release.

## Aggregate Alpha cleanup mode

When `[topology].mode = "aggregate"`, this section replaces the alpha merge instructions below. The head still only routes; a worker runs the git and PR commands. The existing `just bs-merge` and `merge-pr.sh` are **for direct-to-alpha runs only** and must never process a batch PR in this mode.

Until the combined PR lands, the canonical alpha checkout still has the old skill and helpers. Launch the head and every setup/worker/tester pane with cwd at `/Users/ianburke/Documents/GitHub/PLEXI/worktrees/feature/alpha-cleanup`; read this skill and `RUN_CONFIG.toml` from that checkout and invoke its `just` recipes and `scripts/babysitter-policy.py` there. The helper checks its own source branch before aggregate dispatch. It reads stint metadata from the canonical alpha checkout, so the ignored `.stint` task graph remains authoritative. Do not start the aggregate run from the alpha root.

1. Before any product dispatch, run `uv run --no-project python scripts/babysitter-policy.py preflight <ids...>` from the repo. It checks the task graph and optional prerequisite gate configured by `[topology]`; use the manifest evidence described below for aggregate-local dependencies. Save its JSON as `RUN_STATE.md` evidence. A failed preflight is a stop, not a waiver.
2. A setup worker pushes the clean `feature/alpha-cleanup` branch and opens one **draft** PR to `alpha`. Record its PR number and current head in `RUN_STATE.md`. This is the only PR Ian will install and review at the end. Do not merge it as part of unattended batch closeout.
3. For each batch, select the worker with `uv run --no-project python scripts/babysitter-policy.py route <S|M|L>`. Size means the largest stint in the batch. Re-run with `--exhausted <aliases...>` on a usage limit and record exhausted aliases. The returned tester alias is from a different vendor. If routing fails, park the batch; do not select an alias by hand. Fresh validation rounds also use this command. Every size has the same merge-all gate.
4. Brief the worker to run `just bs-start --base feature/alpha-cleanup <ids...>`, implement in that worktree, and open its batch PR with `--base feature/alpha-cleanup`. The worker must not use the normal `/implement-stint-v2` PASS merge step. It rebases on the current aggregate branch **before** final local validation, pushes, and waits for GitHub to show that head. If the aggregate base advances during another lane, repeat rebase and validation. Only one batch may integrate at a time.

For dependencies already integrated into the aggregate, pass the run's manifest with `--manifest <run-dir>/aggregate.json` to both `preflight` and `just bs-start --base feature/alpha-cleanup`. The policy verifies the live aggregate and preserved batch evidence before allowing an in-progress prerequisite; without a manifest the prerequisite must be done.
5. An independent tester installs the batch PR using `just pr-install <batch PR>`, drives it, and records the installed head from `install.log`. A named shim (`plexi-pr-<N>`) ignores inherited `PLEXI_SOCKET`; the bare `plexi` may adopt it. Store each verdict in `RUN_STATE.md` and a JSON record in the run dir. Required fields are `size`, `stints`, `head`, `base_head`, `worker_alias`, `tester_alias`, `exhausted`, `local` (including the phase-appropriate test scope/evidence defined in TESTING.md), `validation` (`head`, `install_head`, `verdict`, `worker_vendor`, `tester_vendor`), `checks_head`, and `waivers` (normally empty). Any red or pending non-deferred check, or proposed waiver, parks the batch for an explicit Ian ruling outside automated merge; a JSON claim of approval is not authority.
6. The worker runs `uv run --no-project python scripts/babysitter-policy.py integrate <batch PR> <record.json> <run-dir>/aggregate.json`. It re-reads PR head, base and checks, refuses changed evidence, and merges with GitHub's `--match-head-commit` guard; it does **not** rebase after validation. It records source and resulting aggregate commits. A changed head or base requires a new install, tester PASS, local gates and non-deferred check record at the new head. Keep stint tasks claimed; do not mark them done while they are only on the aggregate branch. Update `TRUTH.md` for the integration in the same turn.
7. When all accepted batches are integrated, verify the draft aggregate PR's current head against the aggregate worktree. Run the final combined qualification and `just pr-install <aggregate PR>` using TESTING.md's installer pretest scope in the foreground and have a fresh independent tester drive the combined build, including cross-batch behavior. Record the full local gates, install head, tester PASS and non-deferred checks at that exact aggregate head, plus `aggregate_head` and `integrated_batches` in manifest order. Account for every requested stint in `aggregate.json` as integrated or parked with a reason. Run `babysitter-policy.py gate final <aggregate PR> <final-record.json> --manifest <run-dir>/aggregate.json`; any rebase or subsequent commit invalidates this record and requires the full final gate again. Present the installed aggregate PR and evidence to Ian for acceptance before Alpha merge. After that separate merge, close linked stints, update `TRUTH.md` and the V1 checklist in the same turn.

The JSON record is machine checked by `scripts/babysitter-policy.py`. Non-deferred checks returned by `gh pr checks` must all pass for automated integration; the hosted Rust policy above governs absent/deferred checks. The local build, test and Clippy gates and independent installed validation are never waived. `aggregate.json` lists integrated batch PRs and parked stints with reasons. If an outside update moves the base during integration, the merged PR remains recorded with `base_race`; stop further integration and keep its original base and actual parent as evidence. Once the full queue is integrated or parked, repeat the combined local gates, `just pr-install <aggregate PR>`, independent validation and non-deferred checks at the current aggregate head. Then run `babysitter-policy.py recover-base-race <aggregate PR> <final-record.json> <run-dir>/aggregate.json`. This checks that final installed head and the full run manifest before recording the recovery head; only then can `gate final` pass. Never hand-edit away `base_race`.

```
/babysitter-v3 <STINT_ID> [<STINT_ID> ...] [parent=<PANE_ID>]
/babysitter-v3 resume [parent=<PANE_ID>]
```

`parent` is the pane that launched you, usually an orchestrating agent that reports to Ian. Without it, Ian is reading your pane directly.

## Run directory

Every run writes to exactly one `<run_root>/<YYYYMMDD-HHMM>/`, named from `date -u +%Y%m%d-%H%M` (UTC, never local time). The directory is git-ignored, so it never dirties alpha:

| File | Purpose |
|---|---|
| `RUN_STATE.md` | The baton. Overwritten at every merge or park: remaining queue, batches, lane → pane/PR/round, parked items, orphaned worktrees. A fresh head resumes from it alone. |
| `LOG.md` | Append-only, UTC timestamps: briefs, PR opened, verdicts, merges, respawns, friction. |
| `REPORT.md` | Written once at close-out. The single artifact Ian reads; see Stop conditions. |
| `LEDGER.jsonl` | Append-only, one JSON line per tester verdict or respawn, written by the head right after it. Schema below. |
| `HUMAN_CHECKS.md` | Non-blocking post-merge checks only a human can judge (`- [ ] PR #N (stint): what to look at. Evidence: <path>`). |
| `briefs/<pane-label>.md` | The full brief for each pane. Panes are told to read it; the launch prompt stays short. |

`LEDGER.jsonl` line keys, all required: `batch` (first stint id), `stints` (list), `worker_alias`, `worker_model`, `round` (1-based, continues across a crossover), `tester_alias`, `verdict` (`PASS`/`FAIL`), `fail_reason` (one line, null on PASS), `outcome` (`merged` · `follow_up` · `crossed_over` · `parked`), `crossed_over_to` (alias or null), `usage_limit_hit` (bool), `wall_minutes` (number, this attempt), `ts` (UTC ISO-8601). Optional: `follow_up_stints` (array of stint id strings filed when closing the batch; omit or leave empty if none). `uv run --no-project python scripts/babysitter-policy.py ledger-summary <run-dir>` aggregates it per worker alias, including the `follow_up_stints` count.

`resume` = find the newest run dir, read `RUN_STATE.md` (it records `parent`), append a takeover line to `LOG.md`, continue.

## Before the first spawn

1. **Aggregate mode only — bootstrap the base.** `[topology].branch` (`feature/alpha-cleanup`) must exist as a **worktree** at the current alpha head, clean and exactly synced with origin. Otherwise `bs-start --base` refuses with `no worktree for base …` or a behind/ahead error, and `integrate` refuses a missing or dirty aggregate worktree. A branch left behind by a previous run is the normal case, not an error: its work is already in alpha as a squash, so reset it.
   ```bash
   git push --force-with-lease=refs/heads/feature/alpha-cleanup:<previous-remote-sha> \
     origin <alpha-head-sha>:refs/heads/feature/alpha-cleanup
   git worktree add worktrees/feature/alpha-cleanup feature/alpha-cleanup
   ```
   Then run every `bs-start` from inside that worktree — aggregate mode hard-errors if the helper is invoked from any other checkout.
2. `git status --porcelain` on the alpha root is empty (run `git update-index --refresh` first). Dirty → stop and tell Ian.
3. Drop any stint in `[reservations].attended_only`, any `blocked`, and any whose body has no `## Done-When` heading. Both `## Done-When` and `## Done When` are accepted. Exception: a stint Ian names explicitly. Rule its Done-When from its Scope/Testing sections, on the record in `LOG.md` and in the brief.
4. **Batch the queue.** Related small stints that touch the same area ship as one PR. Batches that touch overlapping files never run at the same time. Write the batch plan and `parent` (or `none`) into `RUN_STATE.md`.
5. Check the command channel: `plexi pane list` must return, then `plexi pane slot write channel_preflight ready --pane-id $PLEXI_PANE_ID --replace` and `plexi pane slot read channel_preflight $PLEXI_PANE_ID` must round-trip `ready`. Create a disposable probe pane; use `plexi pane send <probe-pane> 'echo channel-preflight-ok' --submit` and confirm its output, then `plexi pane close <probe-pane>` and confirm it disappears from `pane list`. On failure, report the command and error loudly and retry all checks with the running host's own executable, resolved from `ps -o command -p <host_pid>` (for example `/Applications/Plexi Beta.app/Contents/MacOS/plexi-beta`). A channel-named shim may still point to an incompatible CLI. Stop if the host executable fails; never silently degrade.
6. Run `caffeinate -dimsu` in its own pane (`plexi pane new "caffeinate -dimsu" -n caffeinate --no-focus`).
7. Read the previous run's `REPORT.md` **Friction** section (newest run dir other than this one). Check each line against the task graph (`stint list | grep`). A line with no stint id and no `non-actionable` mark is unfiled: file it with `/create-stint` before the first spawn.

## Spawning a pane (every role, every engine)

`plexi pane new --agent` and `plexi pane status` do not recognise agy, so v3 uses one launch path for every engine:

1. Write the brief to `briefs/<label>.md`: one file per pane, written on its own (no shell loops that build filenames). Before launching, confirm `test -s briefs/<label>.md`. Launch in a later step, never in the same command as the write: a pane that starts before its brief exists has to be redirected.
2. Pick the alias: the first entry of the role's list in `RUN_CONFIG.toml` that hasn't hit a limit this run. Look up its engine with `[engines.alias_engine]` and build the command from `[engines.launch]`.
3. Launch with a short single-quoted prompt (no `'` or backticks inside it). The default split is to the right, which puts panes side by side. Never pass `-d`: it means `--down`, not detached.
   ```bash
   ID=$(plexi pane new "<launch command>" -n <label> --no-focus)
   # e.g. agym -i '/validate-pr 2601 — read your brief first: .stint/runs/20261016-0300/briefs/tester-2601-r1.md'
   ```
4. Confirm it started: within 60 s the `status` slot is non-empty, or the footer shows the engine's working text. If neither, read the capture once and diagnose; never send the same prompt again.
5. A pane that is alive with no agent (shell prompt, no slot, no working footer) is a **failed launch**, not a slow start. Read one capture and check for an argv or usage error (for example `agy` without `-i`: "Prompts are read only from -p/--print, -i/--prompt-interactive, or stdin"). Close the pane and respawn with the `[engines.launch]` form copied verbatim.

Labels: `worker-<batch>`, `tester-<batch>-r<round>`.

## Slot contract (every pane writes, the head reads)

| Slot | Value |
|---|---|
| `status` | `<step>:<state>`, state ∈ `working · done · blocked · failed` |
| `pr` | PR number once opened |
| `verdict` | Tester only: `PASS` or `FAIL` |
| `last_error` | One line when `blocked`/`failed`: the question or the reason |

Write: `plexi pane slot write <name> <value> --pane-id $PLEXI_PANE_ID --replace`. Every pane writes `plexi pane slot write status <step>:working --pane-id $PLEXI_PANE_ID --replace` immediately on starting, and every later slot write keeps both `--pane-id $PLEXI_PANE_ID` and `--replace`. This targets the caller's own pane even when `PLEXI_SOCKET` is overridden. Workers' steps: `impl` → `pr` (PR open, local gate green, waiting for a verdict) → `merge`. Testers' steps: `test`. Write `status` only at these step boundaries, never for sub-steps. The first write after start is `impl:working` (tester: `test:working`); `start:done` is not a step, and `bs-start` writing it does not replace the first write. A lane that never leaves its first non-terminal value is a stall candidate. A question is `blocked` plus `last_error`, never prose at an idle prompt.

## The loop (per batch, up to `max_concurrent_lanes` at once)

**1. Worker.** Pick the role by batch size, meaning the largest `Size` among the batch's stints (`stint show <id>`). If every stint is S or M, use `worker`. If the batch contains an L stint, or follows a hard reject, use `worker_large`. Brief:

> /implement-stint-v2 <ids...>. You are being run by babysitter-v3: Ian is not watching. Start the lane with `just bs-start <ids...>` from the alpha root so the worktree is `worktrees/feature/stint-<id>[-<id>...]-<slug>` and all stints are claimed. Publish the slot contract in `.agents/skills/babysitter-v3/SKILL.md`. Do the PR-push gate from `src/testing/TESTING.md`: `just build`, scoped `cargo test --bin plexi -- <filters...>` selected by `scripts/scoped-tests.sh` with the documented array invocation, and `cargo clippy --bin plexi -- -D warnings`, plus scene/screenshot evidence for UI changes. Use the batch scope/evidence contract in TESTING.md, including independent installed verification of the affected behavior. Do not run a full host suite per feature, batch, integration or tester install; reserve it for the final combined candidate. Hosted Rust checks are deferred under the policy above, never counted as passed. Run every gate in the foreground: if your turn ends while a job runs in the background, nothing wakes you. Each new or changed test must fail when the production change it covers is reverted; cite only tests that exist (verify Rust test names with `cargo test --bin plexi <name> -- --list` showing at least one match). Put the command and its output in a `## Revert proof` section in the PR body. Open the PR (with `Stint: <id>` in the body); do not wait for hosted Rust CI before publishing pr:done; do **not** run `just pr-install` (the tester does). When the PR is open and the local gate is green, write `pr` and `status=pr:done`, post the `[IMPLEMENTED]` block, and wait. Answers to your questions arrive as `PASS`, `FOLLOW-UP: <list>`, or `FAIL: <reason>`. On FOLLOW-UP, write `status=impl:working` before anything else. On PASS, merge with `just bs-merge <PR>` from the alpha root, confirm the stints are done, then write `status=merge:done`. Pre-existing defects outside your diff: note them in the PR body and file them as stints; don't fix them here. Never use `rm -rf`. <run-specific facts only>

Wait (note `<id>` is positional on `slot wait`, `slot read`, and `slot list`; only `slot write` takes `--pane-id`):
`plexi pane slot wait status <id> --until '^(pr|impl|merge|merged|done)(:done|:pr-opened|:blocked|:failed)?$|:(blocked|failed)$' --timeout <heartbeat_seconds>`.

**2. Tester.** As soon as `pr` is set, spawn a fresh `tester` pane. Do not spawn a tester for one lane while another lane is between PASS and `merge:done`; wait for that merge to finish so validation uses the resulting base. Choose the brief by `gh pr diff <PR> --name-only`:

- **Docs, skills or scripts only:** review the diff against the Done-When. No install, except in aggregate mode (below).
- **Aggregate batch, any diff:** always install. `integrate` requires `validation.install_head == head`, even for a test-only diff, so the tester brief always includes `just pr-install <PR> scoped` and records the installed head and affected behavior. Follow TESTING.md when the selector needs reviewed filters; never request a full host suite for the batch.
- **Everything else:** install and drive. Brief:

> Validate PR #<PR> against the Done-When of stints <ids> (`stint show <id>`) and nothing else. Do not edit code. Run `just pr-install <PR> scoped` (release build and relevant pretests; wait for the exit code in the foreground). Use TESTING.md's reviewed filter path if automatic selection is broad. Do not rerun the full host suite. Record what affected behavior passed at the installed head as `validation.affected_behavior`. Check the head sha in the PR profile's `install.log` (8-character form) is the prefix of `gh pr view <PR> --json headRefOid`. Start and drive the host per `/drive-host`; the channel-named `plexi-pr-<PR>` shim ignores inherited `PLEXI_SOCKET`, while a bare `plexi` may adopt it. Drive the feature (`pane state`, `pane capture`, `host screenshot`, `app render --png`) and confirm its new info log line in `~/.plexi-pr-<PR>/plexi.log`. If the PR adds a guard or assertion, prove it can fail: break it in a scratch copy under `$TMPDIR` (leave it there; never delete it), watch the test fail. Don't re-run suites the worker already ran. Never use `screencapture` or anything that raises an OS permission dialog. Stop the host when done. Write `verdict` PASS or FAIL; on FAIL put a numbered repro list in your final reply and the count in `last_error`; then write `status=test:done`. <hard-to-drive surface + the fallback proof you accept>

If the stints touch the assistant or `ai.query`, use the existing test-key setup in `/drive-host` before starting the host; never echo the key.

**3. Route the verdict.** Read `verdict`, then the tester's final reply (`plexi pane capture <id> --lines 60 --plain`). Close the tester pane. Append the `LEDGER.jsonl` line now, before acting on the verdict.

- **PASS** and non-deferred PR checks green → first check the Done-When yourself. List each item in `LOG.md` as met, overruled with a reason, or unmet. Any unmet item that isn't overruled → treat it as a FAIL caused by the PR. If it can't be met in this run (blocked by something outside the batch), send the worker `PASS`, but also file the remainder as a follow-up stint and name it in `REPORT.md`. Never let a stint close as done with silent gaps. Then send the worker `PASS` (`plexi pane send <worker> "PASS" --submit`) if `auto_merge` is on; otherwise add the PR to `RUN_STATE.md` as awaiting Ian and move on. A red check is not the PR's if it fails the same way on alpha, or passes on one `gh run rerun <run-id> --failed`: overrule it on the record and file a stint (or add the evidence to an existing flake stint).
- **FAIL caused by the PR, worker in the cheap tier** (vendor `gemini` or `deepseek`: `agyf`, `agyg`, `pi`) → do not FOLLOW-UP. Close that worker, run `route <size> --exhausted <every alias that failed this batch>` (the tester stays cross-vendor), and spawn a fresh worker on the result. Brief = original brief + worktree path + branch + PR number + tester's numbered list verbatim + the stint's `## Prior Attempts` block + "continue on the same PR; do not restart". Ledger the failed attempt with `outcome=crossed_over` and `crossed_over_to=<new alias>`; the round counter continues and `max_fix_rounds` still applies. Then spawn the next tester as below.
- **FAIL caused by the PR, worker in the strong tier** (codex `cos`/`com`/`col`, claude) → send the worker `FOLLOW-UP: <tester's numbered list, verbatim>. Write status=impl:working now. Find the root cause, not a patch. Push to the same PR, write status=pr:done again.` Confirm the slot has left `pr:done` before you wait on it again. Then spawn a new tester (`r<round+1>`) to re-check only what changed, plus a one-line smoke of what already passed.
- **Every brief, including the first worker brief, FOLLOW-UP and crossover, says**: use TESTING.md's phase-aware gate and evidence schema, preserve build/Clippy and independent installed affected-behavior checks for each batch, reserve the full host suite for the final combined candidate, and distinguish scoped evidence from CI/installed acceptance; each new or changed test must fail when the production change it covers is reverted, and tests may cite only tests that exist (verify with `cargo test --bin plexi <name> -- --list` showing at least one match).
- **FAIL from a pre-existing alpha defect, or a criterion not in the Done-When** → your call: overrule it on the record in `LOG.md`, file a follow-up stint, treat it as PASS.
- **Before closing any batch (merged or parked), file follow-ups.** For every divergence found, every Done-When item overruled or carried forward, and every scoped-out defect or discovered bug, file a stint with `/create-stint`; noting it in `LOG.md` or `REPORT.md` alone is not enough. Filing is autonomous: a head or worker uses `/create-stint`'s autonomous-mode path and never waits for confirmation. Record the new ids in that batch's ledger line as `follow_up_stints`.
- **Rounds exceed `max_fix_rounds`** → send `FAIL: <summary>`, park the batch in `RUN_STATE.md` (the worker runs `just channel-clean pr-<PR>` and `stint unclaim`), keep the worktree for inspection, move on.

**4. After merge** (`status=merge:done`): check it yourself with `gh pr view <PR> --json state` (MERGED) and `stint show <id>` (done). Close the worker. Append the merge to `TRUTH.md` at the repo root, as `AGENTS.md` requires for every merge. Overwrite `RUN_STATE.md`, start the next batch.

## Watching

- Block on terminal slot transitions only (the `slot wait --until` pattern above). Never wake on tester or worker progress slots; the heartbeat is a failsafe. Every heartbeat, for each lane: read `status`, check the last lines of `plexi pane capture <id> --lines 6 --plain` against `[engines.working_footer]`, and inspect non-deferred PR checks once a PR exists; never poll deferred Rust jobs.
- **Stalled** (idle footer, no terminal slot, longer than `stall_minutes`): read the last 40 lines. A question → answer it with the robust option (Ian's standing rule) or rule on it yourself. A finished turn that forgot its slot → tell it to write the slot. Wedged → `plexi pane key <id> ctrl+c`, then one re-orienting message.
- **Idle with a non-terminal slot** (footer idle, `status` still `…:working`) is a stall at once, whatever `stall_minutes` says: Codex ends its turn while a job runs in the background and never wakes. Tell it to rerun that job in the foreground and continue.
- **Network error** (capture ends with "network issue connecting to the server", idle footer): not a usage limit, so do not respawn or switch alias. It leaves the pane idle with no slot change, so a heartbeat that only watches slots misses it. Check connectivity once (`curl -s -m 8 -o /dev/null -w "%{http_code}" https://api.github.com/zen`); if it answers, send that pane one `continue from where you stopped` message. If the network is still down, wait one heartbeat and retry; park nothing for it. Log each occurrence.
- **Usage limit** (capture matches `[engines.usage_limit_patterns]`): mark that alias exhausted for the run in `RUN_STATE.md`. Close the pane, then respawn the same role on the next alias. Its brief is the original brief plus: worktree path, branch, PR number, last slot value, and "continue from here; don't restart". The worktree and PR survive the close. If every alias in a role is exhausted, park the role's lanes and say so in `LOG.md`.
- **Confirm a signal's format before looping on it.** Before polling a file, slot, or log pattern, read one real example and confirm the exact format you will match. `install.log` records the 8-character short sha, not the 40-character one; a loop on the long form never matches. Never loop on a pattern whose format you have not seen.
- Memory: if a lane sits in a build for more than 15 min, check `top -l 1 -o mem -n 5 -stats mem,pid,command`. If you kill a runaway, tell the worker before it misreads the failure.
- End each wake with a three-line progress block (`date +%H:%M` local): done/total, lanes in flight, next check.

## Head relay

The head rots too. After every **two** merges, or whenever your own context feels heavy: overwrite `RUN_STATE.md`, spawn a successor from the `head` role with `/babysitter-v3 resume`, and wait for its `status=head:working`. Then close yourself (if you are a pane) or report its pane id and stop (if you are Ian's session). If no ack arrives within 3 min, keep running and retry at the next boundary.

## Stop conditions and close-out

- **Done** = every batch is merged or parked with a written reason. Then, in order:
  1. Reap orphaned worktrees, stale build targets, and leftover PR channels through a fresh small worker (`just reap-merged-worktrees`, `just clean-stale-targets`, `just channel-clean-merged`; never delete unpushed commits; list kept worktrees in `RUN_STATE.md`). Close `caffeinate`.
  2. Write `REPORT.md`, the only thing Ian needs to read. Keep it short and lead with the state. Run `ledger-summary <run-dir>` and paste its output as the **Alias Scorecard**:
     - **Landed:** each PR → stints, rounds, wall time, the worker and tester alias.
     - **Not landed / gaps:** parked batches and why; Done-When items overruled or carried into follow-up stints (by id).
     - **State left behind:** alpha head sha, open PRs from this run, panes still open (should be none), kept worktrees and why.
     - **Filed:** each follow-up stint id with the batch it came from, and evidence added to existing ones.
     - **Friction:** skill or CLI edits worth making. One line each, at most 5, the highest-leverage first. Every line names a filed stint id (file it with `/create-stint` first) or is marked `non-actionable`. An unfiled friction line recurs.
     - **Needs Ian:** the `HUMAN_CHECKS.md` items, plus any decision only he can make. Write remaining aggregate steps as an exact copy-pasteable command block, including `gh pr create` if the aggregate PR is not open, `just pr-install <PR>`, and `uv run --no-project python scripts/babysitter-policy.py gate final <PR> <final-record.json> --manifest <run-dir>/aggregate.json`. Fill in the actual branch, PR number (or capture it from creation), and paths; state the required final evidence and acceptance before the commands that consume them.
  3. Append the run to `TRUTH.md`, pointing at `REPORT.md`.
  4. **With a `parent`:** `plexi pane send <parent> "BABYSITTER DONE: <run dir>/REPORT.md" --submit`, then close your own pane (`plexi pane close`). The parent relays the report to Ian, so your pane has nothing left to read. **Without a `parent`:** print `REPORT.md` as your final reply and stay open.
- Ian (or the parent) says stop → finish writing state, write a partial `REPORT.md`, leave lane panes running, report the same way.

## Rules

- Never code, run git, build or merge. Route.
- One worker per batch, for the whole batch. Testers are single-use: one per round.
- Pass criteria come only from the stint's Done-When.
- The head verifies evidence; it never re-runs a gate a lane already ran. If evidence is missing, ask the lane for it or run the minimum targeted check.
- Name the exact skill in every brief (`/implement-stint-v2`, never "implement it").
- No sounds, no dialogs, no `security` Keychain commands, no file deletion (`rm -rf` or any other spelling) in any brief.
- Every `plexi pane slot write` you run passes `--pane-id $PLEXI_PANE_ID --replace`, including the initial `status=<step>:working` write; this keeps the write on the caller pane even when its target socket is overridden. `plexi pane slot wait`, `read`, and `list` take `<pane_id>` positionally.
