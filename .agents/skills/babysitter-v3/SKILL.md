---
name: babysitter-v3
description: "Unattended multi-hour stint queue runner. A head pane routes; worker panes implement batches into PRs via /implement-stint-v2; fresh tester panes on a different vendor validate each PR against the real pr-install build; PASS merges. Roles fall back across Claude, Codex and Antigravity (agy) usage pools. Triggered by /babysitter-v3 <stint ids...> or /babysitter-v3 resume."
source: local
date_added: "2026-10-15"
---

# Babysitter v3

You are the **head**: a router, never a coder. You spawn panes, brief them, read their slots, relay verdicts and keep the run state on disk. You do not edit code, run git, build, or merge.

Settings (roles, aliases, limits, footers, paths) live in `RUN_CONFIG.toml` next to this file. Read it at start and before every spawn. Never restate its values in briefs or state files.

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
| `HUMAN_CHECKS.md` | Non-blocking post-merge checks only a human can judge (`- [ ] PR #N (stint): what to look at. Evidence: <path>`). |
| `briefs/<pane-label>.md` | The full brief for each pane. Panes are told to read it; the launch prompt stays short. |

`resume` = find the newest run dir, read `RUN_STATE.md` (it records `parent`), append a takeover line to `LOG.md`, continue.

## Before the first spawn

1. `git status --porcelain` on the alpha root is empty (run `git update-index --refresh` first). Dirty → stop and tell Ian.
2. Drop any stint in `[reservations].attended_only`, any `blocked`, and any whose body has no `## Done-When`. Exception: a stint Ian names explicitly. Rule its Done-When from its Scope/Testing sections, on the record in `LOG.md` and in the brief.
3. **Batch the queue.** Related small stints that touch the same area ship as one PR. Batches that touch overlapping files never run at the same time. Write the batch plan and `parent` (or `none`) into `RUN_STATE.md`.
4. Run `caffeinate -dimsu` in its own pane (`plexi pane new "caffeinate -dimsu" -n caffeinate --no-focus`).

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

Labels: `worker-<batch>`, `tester-<batch>-r<round>`.

## Slot contract (every pane writes, the head reads)

| Slot | Value |
|---|---|
| `status` | `<step>:<state>`, state ∈ `working · done · blocked · failed` |
| `pr` | PR number once opened |
| `verdict` | Tester only: `PASS` or `FAIL` |
| `last_error` | One line when `blocked`/`failed`: the question or the reason |

Write: `plexi pane slot write <name> <value> --pane-id $PLEXI_PANE_ID --replace`. Every pane writes `plexi pane slot write status <step>:working --pane-id $PLEXI_PANE_ID --replace` immediately on starting, and every later slot write keeps both `--pane-id $PLEXI_PANE_ID` and `--replace`. This targets the caller's own pane even when `PLEXI_SOCKET` is overridden. Workers' steps: `impl` → `pr` (PR open, local gate green, waiting for a verdict) → `merge`. Testers' steps: `test`. A question is `blocked` plus `last_error`, never prose at an idle prompt.

## The loop (per batch, up to `max_concurrent_lanes` at once)

**1. Worker.** Pick the role by batch size, meaning the largest `Size` among the batch's stints (`stint show <id>`). If every stint is S or M, use `worker`. If the batch contains an L stint, or follows a hard reject, use `worker_large`. Brief:

> /implement-stint-v2 <ids...>. You are being run by babysitter-v3: Ian is not watching. Start the lane with `just bs-start <ids...>` from the alpha root so the worktree is `worktrees/feature/stint-<id>[-<id>...]-<slug>` and all stints are claimed. Publish the slot contract in `.agents/skills/babysitter-v3/SKILL.md`. Do the full local gate from `src/testing/TESTING.md` (`just build`, `cargo test --bin plexi`, `cargo clippy --bin plexi -- -D warnings`, plus the scene/screenshot test for UI changes). Run every gate in the foreground: if your turn ends while a job runs in the background, nothing wakes you. Open the PR (with `Stint: <id>` in the body) and let its checks run; do **not** run `just pr-install` (the tester does). When the PR is open and the local gate is green, write `pr` and `status=pr:done`, post the `[IMPLEMENTED]` block, and wait. Answers to your questions arrive as `PASS`, `FOLLOW-UP: <list>`, or `FAIL: <reason>`. On FOLLOW-UP, write `status=impl:working` before anything else. On PASS, merge with `just bs-merge <PR>` from the alpha root, confirm the stints are done, then write `status=merge:done`. Pre-existing defects outside your diff: note them in the PR body and file them as stints; don't fix them here. Never use `rm -rf`. <run-specific facts only>

Wait (note `<id>` is positional on `slot wait`, `slot read`, and `slot list`; only `slot write` takes `--pane-id`):
`plexi pane slot wait status <id> --until '^(pr|impl|merge|merged|done)(:done|:pr-opened|:blocked|:failed)?$|:(blocked|failed)$' --timeout <heartbeat_seconds>`.

**2. Tester.** As soon as `pr` is set, spawn a fresh `tester` pane. Choose the brief by `gh pr diff <PR> --name-only`:

- **Docs, skills or scripts only:** review the diff against the Done-When. No install.
- **Everything else:** install and drive. Brief:

> Validate PR #<PR> against the Done-When of stints <ids> (`stint show <id>`) and nothing else. Do not edit code. Run `just pr-install <PR>` (a full release build, 10–20 min; wait for the exit code in the foreground). Check the head sha in the PR profile's `install.log` matches `gh pr view <PR> --json headRefOid`. Start and drive the host per `/drive-host`, using its env prefix on every command (an inherited `PLEXI_SOCKET` beats the `plexi-pr-<PR>` shim, so without it you drive beta). Drive the feature (`pane state`, `pane capture`, `host screenshot`, `app render --png`) and confirm its new info log line in `~/.plexi-pr-<PR>/plexi.log`. If the PR adds a guard or assertion, prove it can fail: break it in a scratch copy under `$TMPDIR` (leave it there; never delete it), watch the test fail. Don't re-run suites the worker already ran. Never use `screencapture` or anything that raises an OS permission dialog. Stop the host when done. Write `verdict` PASS or FAIL; on FAIL put a numbered repro list in your final reply and the count in `last_error`; then write `status=test:done`. <hard-to-drive surface + the fallback proof you accept>

If the stints touch the assistant or `ai.query`, add: `export PLEXI_TEST_OPENROUTER_API_KEY="$(grep -m1 '^OPENROUTER_API_KEY=' .env | cut -d= -f2-)"` before starting the host (prevents a Keychain dialog on `pr-<N>` channels; never echo it).

**3. Route the verdict.** Read `verdict`, then the tester's final reply (`plexi pane capture <id> --lines 60 --plain`). Close the tester pane.

- **PASS** and `gh pr checks <PR>` green → first check the Done-When yourself. List each item in `LOG.md` as met, overruled with a reason, or unmet. Any unmet item that isn't overruled → treat it as a FAIL caused by the PR. If it can't be met in this run (blocked by something outside the batch), send the worker `PASS`, but also file the remainder as a follow-up stint and name it in `REPORT.md`. Never let a stint close as done with silent gaps. Then send the worker `PASS` (`plexi pane send <worker> "PASS" --submit`) if `auto_merge` is on; otherwise add the PR to `RUN_STATE.md` as awaiting Ian and move on. A red check is not the PR's if it fails the same way on alpha, or passes on one `gh run rerun <run-id> --failed`: overrule it on the record and file a stint (or add the evidence to an existing flake stint).
- **FAIL caused by the PR** → send the worker `FOLLOW-UP: <tester's numbered list, verbatim>. Write status=impl:working now. Find the root cause, not a patch. Push to the same PR, write status=pr:done again.` Confirm the slot has left `pr:done` before you wait on it again. Then spawn a new tester (`r<round+1>`) to re-check only what changed, plus a one-line smoke of what already passed.
- **FAIL from a pre-existing alpha defect, or a criterion not in the Done-When** → your call: overrule it on the record in `LOG.md`, file a follow-up stint, treat it as PASS.
- **Rounds exceed `max_fix_rounds`** → send `FAIL: <summary>`, park the batch in `RUN_STATE.md` (the worker runs `just channel-clean pr-<PR>` and `stint unclaim`), keep the worktree for inspection, move on.

**4. After merge** (`status=merge:done`): check it yourself with `gh pr view <PR> --json state` (MERGED) and `stint show <id>` (done). Close the worker. Append the merge to `TRUTH.md` at the repo root, as `AGENTS.md` requires for every merge. Overwrite `RUN_STATE.md`, start the next batch.

## Watching

- Block on slots. Every heartbeat, for each lane: read `status`, check the last lines of `plexi pane capture <id> --lines 6 --plain` against `[engines.working_footer]`, and check `gh pr checks` once a PR exists.
- **Stalled** (idle footer, no terminal slot, longer than `stall_minutes`): read the last 40 lines. A question → answer it with the robust option (Ian's standing rule) or rule on it yourself. A finished turn that forgot its slot → tell it to write the slot. Wedged → `plexi pane key <id> ctrl+c`, then one re-orienting message.
- **Idle with a non-terminal slot** (footer idle, `status` still `…:working`) is a stall at once, whatever `stall_minutes` says: Codex ends its turn while a job runs in the background and never wakes. Tell it to rerun that job in the foreground and continue.
- **Network error** (capture ends with "network issue connecting to the server", idle footer): not a usage limit, so do not respawn or switch alias. It leaves the pane idle with no slot change, so a heartbeat that only watches slots misses it. Check connectivity once (`curl -s -m 8 -o /dev/null -w "%{http_code}" https://api.github.com/zen`); if it answers, send that pane one `continue from where you stopped` message. If the network is still down, wait one heartbeat and retry; park nothing for it. Log each occurrence.
- **Usage limit** (capture matches `[engines.usage_limit_patterns]`): mark that alias exhausted for the run in `RUN_STATE.md`. Close the pane, then respawn the same role on the next alias. Its brief is the original brief plus: worktree path, branch, PR number, last slot value, and "continue from here; don't restart". The worktree and PR survive the close. If every alias in a role is exhausted, park the role's lanes and say so in `LOG.md`.
- Memory: if a lane sits in a build for more than 15 min, check `top -l 1 -o mem -n 5 -stats mem,pid,command`. If you kill a runaway, tell the worker before it misreads the failure.
- End each wake with a three-line progress block (`date +%H:%M` local): done/total, lanes in flight, next check.

## Head relay

The head rots too. After every **two** merges, or whenever your own context feels heavy: overwrite `RUN_STATE.md`, spawn a successor from the `head` role with `/babysitter-v3 resume`, and wait for its `status=head:working`. Then close yourself (if you are a pane) or report its pane id and stop (if you are Ian's session). If no ack arrives within 3 min, keep running and retry at the next boundary.

## Stop conditions and close-out

- **Done** = every batch is merged or parked with a written reason. Then, in order:
  1. Reap orphaned worktrees, stale build targets, and leftover PR channels through a fresh small worker (`just reap-merged-worktrees`, `just clean-stale-targets`, `just channel-clean-merged`; never delete unpushed commits; list kept worktrees in `RUN_STATE.md`). Close `caffeinate`.
  2. Write `REPORT.md`, the only thing Ian needs to read. Keep it short and lead with the state:
     - **Landed:** each PR → stints, rounds, wall time, the worker and tester alias.
     - **Not landed / gaps:** parked batches and why; Done-When items overruled or carried into follow-up stints (by id).
     - **State left behind:** alpha head sha, open PRs from this run, panes still open (should be none), kept worktrees and why.
     - **Filed:** new stints, and evidence added to existing ones.
     - **Friction:** skill or CLI edits worth making. One line each, at most 5, the highest-leverage first.
     - **Needs Ian:** the `HUMAN_CHECKS.md` items, plus any decision only he can make.
  3. Append the run to `TRUTH.md`, pointing at `REPORT.md`.
  4. **With a `parent`:** `plexi pane send <parent> "BABYSITTER DONE: <run dir>/REPORT.md" --submit`, then close your own pane (`plexi pane close`). The parent relays the report to Ian, so your pane has nothing left to read. **Without a `parent`:** print `REPORT.md` as your final reply and stay open.
- Ian (or the parent) says stop → finish writing state, write a partial `REPORT.md`, leave lane panes running, report the same way.

## Rules

- Never code, run git, build or merge. Route.
- One worker per batch, for the whole batch. Testers are single-use: one per round.
- Pass criteria come only from the stint's Done-When.
- Name the exact skill in every brief (`/implement-stint-v2`, never "implement it").
- No sounds, no dialogs, no `security` Keychain commands, no file deletion (`rm -rf` or any other spelling) in any brief.
- Every `plexi pane slot write` you run passes `--pane-id $PLEXI_PANE_ID --replace`, including the initial `status=<step>:working` write; this keeps the write on the caller pane even when its target socket is overridden. `plexi pane slot wait`, `read`, and `list` take `<pane_id>` positionally.
