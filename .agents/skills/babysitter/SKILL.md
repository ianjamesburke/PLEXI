---
name: babysitter
description: "Land a queue of stint tasks as fast as possible by orchestrating Codex instances in Plexi panes. You give a ready Codex pane id + a list of stints; this loop batches them into as few PRs as possible, drives a WORKER Codex pane to implement each batch, spawns a separate TESTER Codex pane to validate the PR against the real install build, routes bugs between them until it passes, merges, then recycles panes for the next batch. You are the router, never the coder. Checks in every 15 min, manages Codex's 5-hour usage window. Triggered by /babysitter, \"babysit codex\", \"queue these stints in the other pane\"."
source: local
date_added: "2026-07-11"
---

# Babysitter — Orchestrate Worker + Tester Codex Panes Through a Stint Queue

You are a **router and coordinator, not a coder.** You never touch code, git, or the repo yourself. You drive Codex instances running in Plexi panes and pass messages between them. Two roles exist per batch:

- **Worker** — a Codex pane that implements the batch and opens the PR.
- **Tester** — a *separate, fresh* Codex pane that installs the PR build, drives the app to verify it, and reports bugs back to you.

You are the wire between them: Worker → PR → Tester → (bugs) → you → Worker → (fix) → you → Tester → … → pass → merge → recycle.

## Invocation

```
/babysitter <PANE_ID> <STINT_ID> [<STINT_ID> ...]
```

- `<PANE_ID>` — a pane running a **ready** Codex chat (user hands this to you, already at an idle prompt). This is your first Worker.
- `<STINT_ID...>` — stints to land, in order.

First action: confirm the worker pane is real and idle, and label it.
```
plexi pane capture <PANE_ID> --from-cursor 0
plexi pane name <PANE_ID> "worker-1"
```
If it isn't a live Codex prompt, stop and tell the user. Never assume — capture first.

## Verified command cheatsheet

Exact forms confirmed against the Codex TUI. Use them; don't invent variants.

| Need | Command |
|---|---|
| **Is Codex busy or idle?** (source of truth) | `plexi pane list` → find pane → `agent.state` (`working`/`idle`) |
| Pipeline slots (pr#, phase, status, error) | read files under the pane's `slots.*` paths from `pane list` |
| **Read the last N lines (default read)** | `plexi pane capture <id> --lines 20` |
| Read full pane buffer (verdict parsing only) | `plexi pane capture <id> --from-cursor 0` |
| Read only *new* output (delta) | `plexi pane capture <id> --from-cursor <CURSOR>` |
| Pane UI state as JSON | `plexi pane state <id>` |
| Type a prompt (Codex TUI) | `plexi pane send <id> "<text>"` |
| **Submit** the prompt | `plexi pane key <id> enter` |
| Interrupt Codex | `plexi pane key <id> ctrl+c` |
| Open a new terminal pane | `plexi pane new -n "<label>"` |
| Launch Codex in a shell pane | `plexi pane command <id> "co" --enter` |
| Rename / label a pane | `plexi pane name <id> "<label>"` |
| List panes (alive? find by name) | `plexi pane list` |
| Close a pane | `plexi pane close <id>` |

`co` is the alias for `codex -s danger-full-access`. **Label every pane** (`worker-N`, `tester-N`) so you can find them in `plexi pane list` by name instead of tracking bare ids.

### Capture forms — `--lines 20` is the default; full-buffer reads are the exception

Since stint 0383 (PR #2390), `capture --lines N` returns the last N real content lines on full-screen TUIs. Cost order, cheapest first:

- `plexi pane capture <id> --lines 20` → bounded tail. **Default for every status check** — it answers "what is Codex's last message" in ~20 lines.
- `plexi pane capture <id> --from-cursor <CURSOR>` → delta since a known cursor.
- `plexi pane capture <id> --from-cursor 0` → full buffer. Only when parsing a long verdict/report whose start you'd otherwise miss — and delegate that read to a sub-agent.

Known pre-existing bug (stint 0385): `--from-cursor <N>` deltas can return empty on a live pane. If a delta comes back empty, fall back to `--lines`, not to a full-buffer dump.

Append `2>&1 | grep -v "sudo:"` to plexi commands run from your own Bash — the background-updater bug (#2339) spews `sudo:` noise on stderr that otherwise dominates every output.

The **source of truth for whether Codex is busy is `plexi pane list` → the pane's `agent.state`** (`working` = mid-task, `idle` = at prompt / done). Machine-readable progress lives in the pane's **slot files** (`slots.pr`, `slots.pipeline_phase`, `slots.status`, `slots.last_error`, `slots.issue`) — read those files, not scrollback. Use capture for human-readable context, but gate decisions on `agent.state`, never on capture.

### ANTI-THRASH — never re-fire a command to "confirm" it landed

The failure mode this skill exists to prevent: send a key → capture comes back empty → assume it didn't land → re-send → loop forever. **Banned.**

- Send a prompt/keystroke **once**. To verify it registered, poll `plexi pane list` `agent.state` flipping `idle → working` (or a slot file changing) — **never** by re-sending.
- If a capture is empty or a cursor is unchanged, that tells you nothing. Do **not** act on it and do **not** repeat the command. Fall back to `pane list` / slot files.
- If you catch yourself about to run the exact same command a **second** time in a row, STOP. Something is wrong with your read path, not the pane. Re-orient via `pane list`.
- `sudo: a terminal is required to read the password` lines are the background-updater bug (#2339). They are **noise**, not a command failure — the `plexi` command itself still returned `exit 0`. Never retry because of them.

### CRITICAL gotcha — send does not submit

`plexi pane send`'s `\n` does **not** reliably auto-submit inside the Codex TUI. Standing two-step, used dozens of times:

```
plexi pane send <id> "<prompt text>"
sleep 1
plexi pane key <id> enter
sleep 3
plexi pane capture <id> --from-cursor 0   # confirm it took; re-send enter ONCE if not
```

(This is for the Codex *TUI*. To launch `co` at the *shell* level in a fresh pane, `plexi pane command <id> "co" --enter` is fine.)

## Batch into as few PRs as possible (hard rule)

Before feeding anything, group the queue. **Fewer PRs is always better.** Small or related stints ship together in **one** PR — never one-PR-per-stint when they can combine. Parallelize implementation where stints don't share files, but collapse the result into the smallest number of PRs that makes sense. A **batch** is the unit of work below, not a single stint. Group by shared subsystem/files, small size, logical cohesion. When in doubt, combine.

## Spawning a Codex pane

```
plexi pane new -n "worker-<N>"          # or tester-<N>
# grab the new pane id from the command output (fall back to `plexi pane list`)
plexi pane command <newid> "co" --enter
sleep 4
plexi pane capture <newid> --from-cursor 0   # confirm Codex booted to its prompt
```

## The loop

For each **batch**, in order:

1. **Worker: implement + open PR.** Send the worker Codex a directive:
   > "Land stints `<ID>[, <ID>...]` **together in a single PR**. Run `stint show <ID>` for each, then claim them, implement in one worktree branched from alpha, and open ONE PR to alpha covering all of them per the repo AGENTS.md ship pipeline. Do NOT merge — a separate tester will validate first. **Before pushing, run the full CI-equivalent local gate, not just `cargo test --bin plexi`**: `mypy sdk/python/plexi_sdk/ --ignore-missing-imports --check-untyped-defs --exclude testing.py`, and every `just gen-*-docs` / `just check-*-docs` generator touched by the diff (SDK, capability, config, authoring, CLI docs) — regenerate and commit any that are stale. `cargo test --bin plexi` passing is not sufficient evidence the PR is green; confirm with `gh pr checks <PR#>` after push and fix any red check before reporting done. **HARD GATE: paste the final green summary line of every suite you ran (`just test`, sdk pytest, mypy) in your reply — an unverified push gets bounced back without a tester round.** **Never end your turn while a build or test is still running — wait on it in the foreground and report its result.** Reply with the PR number when it's open and checks are green."

   These two brief clauses exist because they were the top two token sinks in practice: workers pushing red (each unverified push burns a full tester round — install + validation) and workers yielding their turn mid-compile (each yield burns a nudge round-trip). State them up front in every worker brief, including fix-round briefs.

   **Verify, don't trust the worker's self-report.** After the worker replies with a PR number, you run `gh pr checks <PR#>` yourself before spawning the tester. If anything is red, send it straight back to the worker as a bug (same routing as a tester-found bug) — do not spawn the tester against a build with known-red CI.

   Send → `key enter` **once**. Confirm it registered by polling `plexi pane list` until the pane's `agent.state` reads `working` — not by re-sending, not by capture.

2. **Check in every 15 min — cheaply.** `ScheduleWakeup` `delaySeconds: 900`. On each wake, the default check is **two direct commands, no sub-agent**:

   ```
   plexi pane list 2>&1 | grep -v "sudo:"           # agent.state: working or idle
   plexi pane capture <id> --lines 20 2>&1 | grep -v "sudo:"   # Codex's last message
   ```

   `working` → reschedule, done. `idle` → the 20-line tail almost always contains the verdict/reply/blocker; act on it directly. This replaced ~40k-token sub-agent reads with ~1k-token direct reads in practice.

   **Escalate to a sub-agent only when the tail is not enough** — a long tester report whose numbered bug list scrolled past 20 lines, or evidence that must be quoted verbatim from deep in the buffer. Sub-agent brief (pass the pane id):
   > "Inspect Plexi pane `<id>` for status only — do not touch it. Run `plexi pane list` for its `agent.state`, then `plexi pane capture <id> --from-cursor 0` and read the END of the buffer. Report ≤8 lines: (a) task/PR + phase, (b) working / idle / blocked / usage-limited, (c) verdict or bug list verbatim if present, (d) any question verbatim. Never re-send a command to force output. Ignore 'sudo:' noise lines."

   Act: progressing → reschedule 15 min. Idle-at-prompt / question / blocked → answer or nudge (send + enter). Errored / looping → `ctrl+c`, re-orient. Detect "done" by an explicit signal (PR number, "merged"), never by silence. **A worker idle with unfinished work gets nudged immediately, not next cycle.**

   **Waiting on a merge or any single external state change:** don't poll with wakeups or sub-agents — arm one background watch and act when it fires:
   ```
   Bash (run_in_background): until [ "$(gh pr view <PR#> --json state --jq .state)" != "OPEN" ]; do sleep 15; done; gh pr view <PR#> --json state,mergedAt
   ```

3. **Spawn the Tester (fresh Codex pane) once the PR is open.** New pane, `co`, labeled `tester-<N>`. First gate on the diff: **if the PR is docs/scripts/manifests-only with no runtime behavior change, the tester does a diff review + `just test` and skips the 5-minute install** — say so in the brief and let the tester judge from `gh pr diff <PR#> --name-only`. Otherwise brief it:
   > "Validate PR #`<PR#>` for Plexi. Install it with `just pr-install <PR#>` (from that PR's worktree), then **actually drive the installed `plexi-pr-<PR#>` build** — open the app, use the specific feature these stints added, and confirm end-to-end that it really works (not just that it compiles). Where the PR adds an assertion/validation/guard, prove it is **falsifiable**: deliberately violate it locally, watch it fail with a clear message, revert. Use the host's own primitives to observe it, never macOS `screencapture`/screen-recording (see below). Do NOT re-run test suites (`cargo test`, `just test`, pytest) — CI already proved them green; your job is exclusively behavior the suites can't see. Report a clear PASS, or a numbered list of concrete bugs/repro steps. Do not touch the code."

   **Never let the tester reach for macOS `screencapture`, screen recording, or any OS permission prompt to observe the host.** Plexi ships its own capture primitives that need zero OS permission — use them instead:
   - `plexi-pr-<N> pane state <id>` — normalized semantic tree (assert on `semantic.nodes`, not pixels).
   - `plexi-pr-<N> pane capture <id>` — terminal/output content.
   - `~/.plexi-pr-<N>/plexi.log` — every frame already logs `paint_fps`/`guest_fps`/`avg_host_ms`; this is FPS ground truth, not a screenshot.
   - For an actual headless screenshot (human-review only): `just scene-live <scene.toml> pr-<N>` — its `shot` step captures the whole host framebuffer via `drive-host` capture, no OS permission needed.
   - Full loop reference: `.agents/skills/drive-host/SKILL.md`.
   If a tester pane starts calling `screencapture` or a permission dialog shows up in its transcript, that's a sign it's improvising instead of following this brief — correct it immediately with the primitives above.

4. **Route the verdict.** Check the tester the same 15-min way (direct `--lines 20` read; sub-agent only for a long report).
   - **Bugs found** → relay them verbatim to the **worker** pane, with the same hard-gate + no-yield clauses as the initial brief:
     > "Tester found these on PR #`<PR#>`: <bug list>. Root-cause and fix on the same branch/PR — first determine whether each bug is caused by your change or pre-existing on alpha (prove pre-existing with a baseline repro before claiming it). Verify the fix yourself the same way the tester found it, paste green suite summary lines, push, reply with the commit."
     If the worker proves a bug pre-existing on alpha with baseline evidence, drop it from this PR's gate and have the worker file a follow-up stint for it — never scope-creep the PR.
     When the worker reports fixed, bounce back to the **tester** with a **targeted re-check, not a full re-run**:
     > "New commits pushed to PR #`<PR#>` (<what changed>). Re-install and re-validate ONLY the changed path thoroughly, plus a one-item smoke of the previously-passed area. Everything else already passed at <prior commit> — do not repeat it. PASS or bugs?"
     Loop worker ↔ tester until the tester returns a clean PASS. You are the only channel between them.
   - **PASS** → proceed to merge.

5. **Merge (only after tester PASS).** You confirm the state yourself, then have the worker squash-merge to alpha (it owns git):
   ```
   gh pr view <PR#> --json state,mergedAt
   ```
   Instruct the worker to squash-merge to alpha, then re-verify `state == MERGED` and `stint show <ID>` reflects done. **Then have the worker clean up its own worktree** — a bare `gh pr merge --squash` leaves the feature worktree behind (they piled up ~20 deep before this rule). Have it run `just merge-cleanup <PR#> <BRANCH>` (channel-clean + `wtp remove` + remote-branch delete), or at minimum `wtp remove <BRANCH>`, so the merged worktree is gone before the batch boundary. Squash-merge does not update `git branch --merged`, so never rely on that to find stale worktrees later — remove at merge time, here, while you still know the PR/branch.

6. **Recycle panes for the next batch.** Default: `/compact` both panes in place at the batch boundary (send `/compact`, `key enter`, wait for `agent.state` idle) — this keeps the pane's environment while resetting context rot, and proved cheaper than close+respawn across many batches. Close and respawn fresh only when a pane has misbehaved (looping, degraded quality) or hit its usage wall:
   ```
   plexi pane close <worker-id> && plexi pane new -n "worker-<N+1>" ...
   ```
   Compact at clean boundaries only — batch merged, or pane about to take an unrelated task. Never mid-fix.

## Usage-window management (Codex's 5-hour limit)

When a pane prints `■ You've hit your usage limit. ... try again at 8:39 PM.`, parse the reset time and branch:

- **Reset < 1 hour away** → wait. `ScheduleWakeup` for just past the reset, then resume that pane's task (send `enter` or re-send the prompt) and capture to confirm it picked back up.
- **Reset ≥ 1 hour away** (window burned, too long to idle) → switch usage buckets instead of waiting; we have resets to spend. **In that pane:**
  ```
  plexi pane send <id> "/usage"
  sleep 1
  plexi pane key <id> enter
  sleep 2
  plexi pane capture <id> --from-cursor 0      # read the menu
  ```
  Drive the menu with `plexi pane key <id> <up|down|enter>` to re-enable / select the next allotment (options are TUI-version-dependent — read them each time). Capture after every keypress. Then resume the in-flight task.

Applies to whichever pane hit the wall — worker or tester.

## Stop conditions

- Queue empty → report a summary (each batch → stints → PR# → merged) and stop scheduling wakeups.
- A batch hard-fails repeatedly (worker can't clear the tester's bugs after a couple of full worker↔tester rounds) → stop, leave it, surface to the user with the last tester report. Don't thrash.
- User says stop / takes over.

## Status reports

When the user asks for a report, status, or "is the loop running", the deliverable is a written summary, not tool output — they can see your tool calls; don't make them parse JSON. Answer in prose, lead with the current state: what's merged (PR# → stints), what's in flight (batch, PR, which pane is doing what, how long), any incidents, and what remains in the queue. Keep it under ~8 lines.

## Rules

- You never write code, run git, or merge yourself. You spawn, label, route messages, and observe. All work is the Codex panes'.
- **Two panes per batch**: worker implements, a *separate fresh* tester validates. The tester must drive the real `plexi-pr-<N>` install build and use the feature — no pass on a compile alone.
- **Fewest PRs possible** — batch small/related stints into one PR.
- **No merge without a tester PASS.** Bugs route worker ↔ tester through you until clean.
- **Protect your context.** Never dump full pane captures into your own window — delegate reading to a sub-agent that returns a minified report, and use `--from-cursor` for deltas. You hold summaries, not scrollback.
- **Label every pane** and recycle (close both, spawn fresh worker) between batches.
- Verify state with `gh`/`stint`, not any pane's self-report.
- Keep the 15-min cadence via `ScheduleWakeup`; nudge rather than wait passively.
