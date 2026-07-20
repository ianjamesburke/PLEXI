---
name: babysitter
description: "Land a queue of stint tasks as fast as possible by orchestrating agent instances in Plexi panes. You give a ready agent pane id + a list of stints; this loop batches them into as few PRs as possible, drives a WORKER pane to implement each batch, spawns a separate TESTER pane to validate the PR against the real install build, routes bugs between them until it passes, merges, then recycles panes for the next batch. You are the router, never the coder. Checks in every 15 min. Triggered by /babysitter, \"babysit the queue\", \"queue these stints in the other pane\"."
source: local
date_added: "2026-07-11"
---

# Babysitter — Orchestrate Worker + Tester Panes Through a Stint Queue

You are a **router and coordinator, not a coder.** You never touch code, git, or the repo yourself. You drive agent instances running in Plexi panes and pass messages between them. Two roles exist per batch:

- **Worker** — a pane that implements the batch and opens the PR.
- **Tester** — a *separate, fresh* pane that installs the PR build, drives the app to verify it, and reports bugs back to you.

**Panes run Claude Code, launched with the `c` alias** (bypasses permissions). Set the model with `/model`. Everything below assumes that; there is no Codex in this loop.

You are the wire between them: Worker → PR → Tester → (bugs) → you → Worker → (fix) → you → Tester → … → pass → merge → recycle.

## Invocation

```
/babysitter <PANE_ID> <STINT_ID> [<STINT_ID> ...]
```

- `<PANE_ID>` — a pane running a **ready** agent chat (user hands this to you, already at an idle prompt). This is your first Worker.
- `<STINT_ID...>` — stints to land, in order.

First action: confirm the worker pane is real and idle, and label it.
```
plexi pane capture <PANE_ID> --from-cursor 0
plexi pane name <PANE_ID> "worker-1"
```
If it isn't a live agent prompt, stop and tell the user. Never assume — capture first.

## Verified command cheatsheet

Exact forms confirmed in live runs. Use them; don't invent variants.

| Need | Command |
|---|---|
| **Is the agent busy or idle?** | No single signal is trustworthy — use the "cheap triangle" below. Never gate a decision on one flag. |
| Pipeline slots (pr#, phase, status, error) | read files under the pane's `slots.*` paths from `pane list` |
| **Read the last N lines (default read)** | `plexi pane capture <id> --lines 20` |
| Read full pane buffer (verdict parsing only) | `plexi pane capture <id> --from-cursor 0` |
| Read only *new* output (delta) | `plexi pane capture <id> --from-cursor <CURSOR>` |
| Pane UI state as JSON | `plexi pane state <id>` |
| Type a prompt into the pane's TUI | `plexi pane send <id> "<text>"` |
| **Submit** the prompt | `plexi pane key <id> enter` |
| Interrupt the agent | `plexi pane key <id> ctrl+c` |
| Open a new terminal pane | `plexi pane new -n "<label>"` |
| Launch an agent in a shell pane | `plexi pane command <id> "c" --enter` |
| Set the pane's model | `plexi pane send <id> "/model <name>"` then `key enter` |
| Rename / label a pane | `plexi pane name <id> "<label>"` |
| List panes (alive? find by name) | `plexi pane list` |
| Close a pane | `plexi pane close <id>` |

**Label every pane** (`worker-N`, `tester-N`) so you can find them in `plexi pane list` by name instead of tracking bare ids.

**Backtick trap:** never put backticks in the text you pass to `plexi pane send` from a double-quoted shell string — they trigger command substitution and the send fails or mangles. Write commands and paths bare in briefs.

### Capture forms — `--lines 20` is the default; full-buffer reads are the exception

Since stint 0383 (PR #2390), `capture --lines N` returns the last N real content lines on full-screen TUIs. Cost order, cheapest first:

- `plexi pane capture <id> --lines 20` → bounded tail. **Default for every status check** — it answers "what is the agent's last message" in ~20 lines.
- `plexi pane capture <id> --from-cursor <CURSOR>` → delta since a known cursor.
- `plexi pane capture <id> --from-cursor 0` → full buffer. Only when parsing a long verdict/report whose start you'd otherwise miss. Prefer narrowing it with `grep`/`sed` over dumping the whole thing; delegate to a sub-agent only when the report is genuinely too long to narrow.

Known pre-existing bug (stint 0385): `--from-cursor <N>` deltas can return empty on a live pane. If a delta comes back empty, fall back to `--lines`, not to a full-buffer dump.

Append `2>&1 | grep -v "sudo:"` to plexi commands run from your own Bash — the background-updater bug (#2339) spews `sudo:` noise on stderr that otherwise dominates every output.

### Reading whether a pane is busy

Screen signals, and what each is actually worth:

| Screen signal | Meaning |
|---|---|
| status bar contains `esc to interrupt` | **busy** — a task is running |
| status bar, un-truncated, with no `esc to interrupt` | **idle** — at prompt / done |
| status bar ending in `…` | **unknown.** The line is elided; `esc to interrupt` may be present but cut off. Read another signal. |
| `❯ Press up to edit queued messages` | your prompt is **queued** behind something in-flight; send `escape` ONCE to clear, never `enter` again |
| `paste again to expand` | a long brief pasted as a collapsed block and was **not submitted**; send `enter` ONCE |
| a bare `❯` | **nothing.** This is just the empty input line. It is not an idle signal — a bare `❯` appears while the agent is busy too. Do not read it. |

**Neither signal is reliable alone. Both have observed failure modes — always corroborate with a second one.**

| Signal | How it fails |
|---|---|
| `agent.state` | Reads `idle` on a genuinely busy pane — observed for minutes with a brief queued behind a stuck `/compact`. Acting on that `idle` is the thrash trap: you conclude ready, re-send, loop. |
| status bar | **Truncates on narrow panes.** A skinny pane elides it to `⏵⏵ bypass permissions on (shift+tab to · …` — `esc to interrupt` is present but cut off, so grepping for it returns 0 and a busy pane reads as idle. |

**The robust check is the cheap triangle** — run it when a pane's state actually gates a decision:

```
plexi pane list          # agent.state
plexi pane capture <id> --lines 16   # status bar AND recent buffer activity
```

Then judge: a trailing `⏺ Bash(...)` / tool call in the buffer means it is working regardless of what either flag says. If `agent.state` says `working`, believe it — its false-negative mode is claiming *idle*, not claiming busy. If `agent.state` says `idle` **and** the status bar is un-truncated **and** shows no `esc to interrupt` **and** the last buffer line is a completed reply, only then is the pane genuinely idle.

Never grep the status bar for `esc to interrupt` and treat absence as proof of idle without checking whether the line ends in `…`.

Machine-readable progress lives in the pane's **slot files** (`slots.pr`, `slots.pipeline_phase`, `slots.status`, `slots.last_error`, `slots.issue`) — read those files, not scrollback.

### ANTI-THRASH — never re-fire a command to "confirm" it landed

The failure mode this skill exists to prevent: send a key → capture comes back empty → assume it didn't land → re-send → loop forever. **Banned.**

- Send a prompt/keystroke **once**. To verify it registered, use the cheap triangle above (`agent.state` + status bar + trailing buffer activity) — **never** by re-sending. Corroborate two signals before concluding a pane is idle; each one alone has a known false reading.
- **One exception, and only one:** if the status bar shows the pane is idle *and* the input line shows `Press up to edit queued messages` or `paste again to expand`, your prompt was typed but never submitted. Send `enter` **once** more. If that still does not take, send `escape` to clear the queue — do not send a third `enter`.
- If a capture is empty or a cursor is unchanged, that tells you nothing. Do **not** act on it and do **not** repeat the command. Fall back to `pane list` / slot files.
- If you catch yourself about to run the exact same command a **second** time in a row, STOP. Something is wrong with your read path, not the pane. Re-orient via `pane list`.
- `sudo: a terminal is required to read the password` lines are the background-updater bug (#2339). They are **noise**, not a command failure — the `plexi` command itself still returned `exit 0`. Never retry because of them.

### CRITICAL gotcha — send does not submit

`plexi pane send`'s `\n` does **not** reliably auto-submit inside the agent TUI. Standing two-step, used dozens of times:

```
plexi pane send <id> "<prompt text>"
sleep 1
plexi pane key <id> enter
sleep 3
plexi pane capture <id> --from-cursor 0   # confirm it took; re-send enter ONCE if not
```

(This is for typing into a pane's *TUI*. Launching the agent at the *shell* level in a fresh pane uses `plexi pane command <id> "c" --enter`.)

## Batch into as few PRs as possible (hard rule)

Before feeding anything, group the queue. **Fewer PRs is always better.** Small or related stints ship together in **one** PR — never one-PR-per-stint when they can combine. Parallelize implementation where stints don't share files, but collapse the result into the smallest number of PRs that makes sense. A **batch** is the unit of work below, not a single stint. Group by shared subsystem/files, small size, logical cohesion. When in doubt, combine.

## Spawning an agent pane

Workers and testers are Claude Code panes launched with the `c` alias (bypasses permissions); `/model` sets the model.

```
plexi pane new -n "worker-<N>"          # or tester-<N>
# grab the new pane id from the command output (fall back to `plexi pane list`)
plexi pane command <newid> "c" --enter
sleep 4
plexi pane capture <newid> --from-cursor 0   # confirm the agent booted to its prompt
```

**GOTCHA: BOOT RACE. Never fire `/model` (or any brief) back-to-back on a just-launched `c` pane.** Claude Code takes a few seconds to boot to its prompt. If `/model <name>` or a brief lands before the prompt is up, the inputs concatenate and the slash command swallows the following text as its argument → API 400 `model: String should have at most 256 characters`, and the whole task brief is lost. Fix: after `plexi pane command <id> "c" --enter`, poll `plexi pane capture <id> --from-cursor 0` until you see the booted prompt (the `Claude Code v...` banner / idle input line) **before** sending `/model`. Then confirm `Set model to <X>` in the buffer **before** sending the work brief.

## Model selection per pane

Workers/testers here are Claude Code panes (launched with `c`). Set the model with the `/model <name>` slash command sent into the pane. Same two-step as any prompt; `send` does not auto-submit:

```
plexi pane send <id> "/model <name>"
sleep 1
plexi pane key <id> enter
```

Set the model at each batch boundary, based on the batch's size, **before** sending the work brief.

| Batch size | Model |
|---|---|
| **S / M** | `/model sonnet` — the default |
| **L** | `/model opus` |
| **Hard task, or an agent fumbling/looping** | `/model fable` — escalate to a stronger model |

## The loop

For each **batch**, in order:

1. **Worker: implement + open PR.** Send the worker a directive:
   > "Land stints `<ID>[, <ID>...]` **together in a single PR**. Run `stint show <ID>` for each, then claim them, implement in one worktree branched from alpha, and open ONE PR to alpha covering all of them per the repo AGENTS.md ship pipeline. **Stop once the PR is open and checks are green — do NOT invoke `/validate-pr`, and do NOT merge.** A separate tester pane owns validation. **Before pushing, run the full CI-equivalent local gate, not just `cargo test --bin plexi`**: `mypy sdk/python/plexi_sdk/ --ignore-missing-imports --check-untyped-defs --exclude testing.py`, and every `just gen-*-docs` / `just check-*-docs` generator touched by the diff (SDK, capability, config, authoring, CLI docs) — regenerate and commit any that are stale. `cargo test --bin plexi` passing is not sufficient evidence the PR is green; confirm with `gh pr checks <PR#>` after push and fix any red check before reporting done. **HARD GATE: paste the final green summary line of every suite you ran (`just test`, sdk pytest, mypy) in your reply — an unverified push gets bounced back without a tester round.** **Never end your turn while a build or test is still running — wait on it in the foreground and report its result.** Reply with the PR number when it's open and checks are green."

   These three brief clauses exist because they were the top token sinks in practice: workers running `/validate-pr` (the pipeline auto-chains `implement-stint` → `open-pr` → `validate-pr`, so a worker that follows the skills literally installs the build itself — a ~5 min compile the tester then repeats — and parks on a `[TESTING]` block waiting for a reply that never comes, which you then read as "idle with unfinished work" and burn a nudge on), workers pushing red (each unverified push burns a full tester round), and workers yielding their turn mid-compile (each yield burns a nudge round-trip). State all three up front in every worker brief, including fix-round briefs.

   **Verify, don't trust the worker's self-report.** After the worker replies with a PR number, you run `gh pr checks <PR#>` yourself before spawning the tester. If anything is red, send it straight back to the worker as a bug (same routing as a tester-found bug) — do not spawn the tester against a build with known-red CI.

   **RUST-ONLY PRs SHOW ONLY `claude` + CodeRabbit IN CI; that is GREEN, not incomplete.** The `typecheck` and `check-*-docs` GitHub jobs are conditional on Python (`sdk/python`) or docs changes. A pure-Rust-host PR (e.g. `src/*.rs` only) will NEVER spawn a `typecheck` job, so `gh pr checks` returning just `claude: skipping` + `CodeRabbit: pass` is a full green, not a half-reported run. Do NOT wait or poll for a typecheck that will never appear; that is a silent stall.

   **But green ≠ reviewed.** `CodeRabbit` currently reports "Review skipped: automatic reviews are disabled" on every PR, and `claude` skips on Rust-only diffs — neither is actually reading the code. The real gate for a Rust PR is the worker's local `cargo test --bin plexi` summary (make the worker paste it), the pre-push Codex review in `/implement-stint` Phase 4, and the tester round. Never tell a user a diff was "reviewed by CI."

   Send → `key enter` **once**. Confirm it registered by polling the **status bar** (`plexi pane capture <id> --lines 3`) until `esc to interrupt` appears — not by re-sending. A long brief often pastes as a collapsed block (`paste again to expand`) and needs exactly one more `enter` to submit; that is the only sanctioned re-send.

2. **Check in every 15 min — cheaply.** `ScheduleWakeup` `delaySeconds: 900`. On each wake, the default check is **two direct commands, no sub-agent**:

   ```
   plexi pane list 2>&1 | grep -v "sudo:"           # agent.state: working or idle
   plexi pane capture <id> --lines 20 2>&1 | grep -v "sudo:"   # agent's last message
   ```

   `working` → reschedule, done. `idle` → the 20-line tail almost always contains the verdict/reply/blocker; act on it directly. This replaced ~40k-token sub-agent reads with ~1k-token direct reads in practice.

   **Escalate to a sub-agent only when the tail is not enough** — a long tester report whose numbered bug list scrolled past 20 lines, or evidence that must be quoted verbatim from deep in the buffer. Sub-agent brief (pass the pane id):
   > "Inspect Plexi pane `<id>` for status only — do not touch it. Run `plexi pane list` for its `agent.state`, then `plexi pane capture <id> --from-cursor 0` and read the END of the buffer. Report ≤8 lines: (a) task/PR + phase, (b) working / idle / blocked / usage-limited, (c) verdict or bug list verbatim if present, (d) any question verbatim. Never re-send a command to force output. Ignore 'sudo:' noise lines."

   Act: progressing → reschedule 15 min. Idle-at-prompt / question / blocked → answer or nudge (send + enter). Errored / looping → `ctrl+c`, re-orient. Detect "done" by an explicit signal (PR number, "merged"), never by silence. **A worker idle with unfinished work gets nudged immediately, not next cycle.**

   **Waiting on a merge or any single external state change:** don't poll with wakeups or sub-agents — arm one background watch and act when it fires:
   ```
   Bash (run_in_background): until [ "$(gh pr view <PR#> --json state --jq .state)" != "OPEN" ]; do sleep 15; done; gh pr view <PR#> --json state,mergedAt
   ```

3. **Spawn the Tester (fresh pane) once the PR is open.** New pane launched with `c`, labeled `tester-<N>`; set its model with `/model` before briefing. First gate on the diff — judge from `gh pr diff <PR#> --name-only`:

   - **Docs/scripts/manifests-only, no runtime behavior change** → the tester does a diff review only and skips both the install and the suites. There is nothing to live-drive and the worker already ran the suites pre-push; re-running them here duplicates that.
   - **Anything else** → the install-and-drive brief below.

   > "Validate PR #`<PR#>` for Plexi. Install it with `just pr-install <PR#>` (from that PR's worktree), then **actually drive the installed `plexi-pr-<PR#>` build** — open the app, use the specific feature these stints added, and confirm end-to-end that it really works (not just that it compiles). Where the PR adds an assertion/validation/guard, prove it is **falsifiable**: deliberately violate it locally, watch it fail with a clear message, revert. Use the host's own primitives to observe it, never macOS `screencapture`/screen-recording (see below). Do NOT re-run test suites (`cargo test`, `just test`, pytest) — the worker already ran them green pre-push and pasted the summary; your job is exclusively behavior the suites can't see. Report a clear PASS, or a numbered list of concrete bugs/repro steps. Do not touch the code."

   **The tester validates behavior, not the diff.** It does not re-review code — AI diff review already happened once, pre-push, in `/implement-stint` Phase 4. Live-driving the real build is the thing only this pane can do; that is its entire job.

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

5. **Merge — after tester PASS *and* the human gate.** A PASS is not permission to merge.

   **Default (always, unless the user opted out): surface the passing PR and wait.** Report the verdict and ask for explicit merge approval. Do not proceed on your own judgment that the evidence looks strong — that is exactly the reasoning that merges something the user wanted to see first.

   **Only skip the wait when the user has explicitly opted into auto-merge for this queue** (see Rules). If they have, merge on PASS and roll straight to the next batch.

   Once approved, have the worker run **one command from the alpha root** (it owns git):
   ```
   just merge-pr <PR#>
   ```

   That recipe owns the whole sequence: rebase → squash → sync local alpha → clean up channel/worktree/branches → close the issue *or* the stint tasks. It resolves stint ids from the branch name and PR body automatically, so a `feature/stint-<id>-<slug>` branch needs no extra arguments. For a standalone PR with nothing to close, `just merge-pr <PR#> no-issue`.

   **Do not hand-roll the steps.** An earlier version of this skill taught a four-step sequence (`gh pr merge --squash` → `just merge-cleanup` → `close-stints` → verify) because the recipe used to abort mid-flow: `git worktree remove --force` routinely fails with `Directory not empty`, and under `set -e` that killed the run *before* the stint close, leaving the PR merged and the task stuck `in-progress`. The recipe now self-heals that case. Calling the sub-steps by hand reintroduces the leak it was split to work around.

   Then verify yourself — never on the worker's say-so:
   ```
   gh pr view <PR#> --json state,mergedAt      # expect state == MERGED
   stint show <ID>                             # expect done, for every id
   ```

   Sub-steps (`merge-rebase`, `merge-squash`, `merge-sync`, `merge-cleanup`, `merge-close-stints`) exist only for resuming a genuinely failed run — e.g. a rebase conflict. Reach for them after a failure, not as the normal path.

6. **Recycle panes for the next batch — gate on context budget, not vibes.**

   **A worker's context is a budget you are spending.** Every batch it carries (task N, then N+1, then a `/create-stint` research flow…) compounds. An overloaded worker does not announce itself; it silently degrades — it re-reads files it already read, misses instructions buried in a long brief, and reasons from stale earlier tasks. By the time you notice, you have already paid for a bad PR and a wasted tester round.

   **At every batch boundary, check the budget before assigning the next task:**
   ```
   plexi pane send <id> "/context"
   sleep 1
   plexi pane key <id> enter
   sleep 3
   plexi pane capture <id> --lines 12
   ```

   **`/context` output on a Claude Code pane can exceed 60KB; never full-dump it, grep for the one token line.** A full `--from-cursor 0` of a `/context` render overflows your own window. Instead grep the buffer for the single line matching the token summary, e.g. `NNNk/967k tokens (NN%)`, and take the **LAST** match; earlier `/context` runs stay in scrollback, so a naive `grep | head` returns a stale percentage. Use `grep -oE '[0-9]+k/967k tokens \([0-9]+%\)' | tail -1` (or capture more `--lines` and grep the tail) to read the freshest utilization.

   Read the utilization percentage, then branch:

   | Context | Action |
   |---|---|
   | **< 50%** | Assign the next batch in place. Cheapest path — the pane keeps its environment and warm repo knowledge. |
   | **50–70%** | `/compact`, then assign. |
   | **> 70%**, or the next task is unrelated to the last | **Close and respawn.** Do not compact. |

   **Prefer close+respawn over `/compact` whenever the next task is unrelated.** A fresh pane costs ~20s to boot and starts at a clean slate. Compact costs a multi-minute stall, sometimes hangs (see below), and still leaves a lossy summary of work that has nothing to do with the next task. The information the new worker actually needs is *not* the old transcript — it is a good brief. You already hold the distilled state (PR numbers, merge results, decisions, gotchas); put that in the new prompt and throw the scrollback away.

   ```
   plexi pane close <worker-id>
   plexi pane new -n "worker-<N+1>"
   plexi pane command <newid> "c" --enter
   # then send a self-contained brief carrying forward only what matters
   ```

   **`/compact` can hang and swallow your next message.** Observed: a `/compact` on an idle worker never released; the follow-up brief sat queued behind it for minutes showing `Press up to edit queued messages`, while `agent.state` still read `idle`. A single `escape` cleared the queue and the brief submitted normally. Never re-send `enter` at a stuck queue — that is the thrash loop. If you compact, confirm it actually finished (status bar clear, no queued-message hint) *before* sending the next brief.

   Recycle at clean boundaries only — batch merged, or pane about to take an unrelated task. Never mid-fix.

## Usage-limit handling

When a pane reports it has hit a usage limit, parse the reset time and branch:

- **Reset < 1 hour away** → wait. `ScheduleWakeup` for just past the reset, then resume that pane's task (send `enter` or re-send the prompt) and capture to confirm it picked back up.
- **Reset ≥ 1 hour away** → don't idle the queue. Report the wall to the user with the reset time and ask how to proceed; they may want to switch accounts, drop to a cheaper model via `/model`, or pause the run.

Applies to whichever pane hit the wall — worker or tester.

## Stop conditions

- Queue empty → report a summary (each batch → stints → PR# → merged) and stop scheduling wakeups.
- A batch hard-fails repeatedly (worker can't clear the tester's bugs after a couple of full worker↔tester rounds) → stop, leave it, surface to the user with the last tester report. Don't thrash.
- User says stop / takes over.

## Status reports

When the user asks for a report, status, or "is the loop running", the deliverable is a written summary, not tool output — they can see your tool calls; don't make them parse JSON. Answer in prose, lead with the current state: what's merged (PR# → stints), what's in flight (batch, PR, which pane is doing what, how long), any incidents, and what remains in the queue. Keep it under ~8 lines.

## Rules

- You never write code, run git, or merge yourself. You spawn, label, route messages, and observe. All work is the agent panes'.
- **Two panes per batch**: worker implements, a *separate fresh* tester validates. The tester must drive the real `plexi-pr-<N>` install build and use the feature — no pass on a compile alone.
- **Fewest PRs possible** — batch small/related stints into one PR.
- **No merge without a tester PASS.** Bugs route worker ↔ tester through you until clean.
- **Opt-in auto-merge (user triggers it, never the default).** Default holds: surface each passing PR to the user and wait. But when the user explicitly says the queue is good to merge and they will test holistically at the end (e.g. a final e2e gate), drop the human-hold gate: on tester PASS, have the worker squash-merge to alpha immediately and roll to the next batch with no surfacing-and-waiting. Keep the tester round; it protects alpha's trunk for downstream stints. Only the human-hold is removed.
- **Protect your context.** Default to `--lines 20` reads and narrow full-buffer reads with `grep`/`sed`. Sub-agent delegation is the exception for genuinely long reports, not the default — see the capture-forms section. You hold summaries, not scrollback.
- **Label every pane** and recycle (close both, spawn fresh worker) between batches.
- Verify state with `gh`/`stint`, not any pane's self-report.
- Keep the 15-min cadence via `ScheduleWakeup`; nudge rather than wait passively.
