---
name: babysitter
description: "Land a queue of stint tasks as fast as possible by orchestrating agent instances (Claude `c` or Codex `co`) in Plexi panes. You give it stints (or sprints); it spawns every pane it needs: a WORKER pane implements each batch and opens one PR, a separate fresh TESTER pane validates the PR against the real install build, bugs route between them until it passes, then merge and fresh panes for the next batch. The head relays itself to a fresh pane after every merge via RUN_STATE.md. You are the router, never the coder. Checks in every 10 min. Triggered by /babysitter, \"babysit the queue\", \"queue these stints\"."
source: local
date_added: "2026-07-11"
---

# Babysitter — Orchestrate Worker + Tester Panes Through a Stint Queue

You are the **HEAD AGENT — a router and coordinator, not a coder.** You never touch code, git, or the repo yourself. Your sole job is to keep your own context window small and route messages: outsource every piece of real work — implementation, testing, research, reading long reports, any multi-step investigation — to agent panes. If a step would put more than ~20 lines of someone else's output into your context, delegate it. You hold distilled state (PR numbers, verdicts, decisions, timings), never scrollback. Two roles exist per batch:

- **Worker** — a pane that implements the batch and opens the PR.
- **Tester** — a *separate, fresh* pane that installs the PR build, drives the app to verify it, and reports bugs back to you.

**Panes run Claude (`c` alias) or Codex (`co` alias) — both bypass permissions.** Never use bare `claude`/`codex`. The two TUIs drive identically for this loop: same send/enter two-step, same `/model <name>` switch, same `/compact`. The one divergence: fresh-conversation reset is `/clear` in Claude, `/new` in Codex. Examples below use `co`; substitute `c` freely.

### Model tiers — pick per batch by difficulty

| Tier | Claude (`c`) | Codex (`co`) | Use for |
|---|---|---|---|
| Mid | `sonnet` or `opus` | `gpt-5.6-terra` | small, simple, mechanical batches |
| High | `fable` | `gpt-5.6-sol` | hard/ambiguous work, any fix round |

Set via the two-step: `plexi pane send <id> "/model <name>"` then `plexi pane key <id> enter`. Judge difficulty from the stint bodies before briefing. **Every fix round runs on the high tier** — see step 4.

**Panes are single-use — NEVER recycle a worker or a tester across tasks.** A tester validates exactly one thing (one PR, or one re-check of a fix) and is then done. A worker owns exactly one batch, from brief through merge, and is then done. For the next task or validation, **close the old pane and open a brand-new one** — a warm transcript biases the read, bloats context, and silently degrades the agent. (`/compact` mid-batch during a fix round is fine — that's the same task; carrying a pane into a *new* task is not.)

You are the wire between them: Worker → PR → Tester → (bugs) → you → Worker → (fix) → you → Tester → … → pass → merge → fresh panes for the next batch.

## Invocation

```
/babysitter [<PANE_ID>] <STINT_ID> [<STINT_ID> ...]
/babysitter [<PANE_ID>] sprints <SPRINT_ID> [<SPRINT_ID> ...]
/babysitter resume
```

- `<PANE_ID>` — optional. If given, it's a pane already running a **ready** agent chat (idle prompt) to use as your first Worker. **If omitted (the normal case), spawn worker-1 yourself** per "Spawning an agent pane" — the loop creates and closes every pane it needs; the user never has to hand you one. A bare-number first argument is a pane id only if `plexi pane list` confirms a live agent pane with that id; otherwise treat it as a stint id.
- `<STINT_ID...>` — stints to land, in order.
- `sprints <SPRINT_ID...>` — **sprint-queue mode**: the queue is every open task in the named sprints, in the given sprint order. Resolve it with `stint sprint show <id>` at start, and **re-resolve after every merge** — a merge can unblock tasks (`blocked` → `ready`) that were not in the initial snapshot. The run ends when the named sprints have no claimable tasks left, not when the initial snapshot drains. Tasks that stay `blocked` on work outside the named sprints are reported as skipped, never waited on.
- `resume` — **fresh-head takeover** of a run already in flight: read `RUN_STATE.md` (next to this SKILL.md) for the mode, flags, and next batch; re-resolve the queue from stint; append a takeover line to `LOG.md`; continue the loop at the next batch. This is both the head-relay mechanism (see "Head handoff" below) and the crash-recovery path — a user can boot a brand-new babysitter with `/babysitter resume` at any time.

**The head runs Opus by default.** Routing, slot reads, and brief relaying do not need the high tier — the hard reasoning happens in worker panes, which get their own model per batch. If the head itself hits a genuine judgment call (ambiguous pre-existing-bug ruling, a stop-condition decision), note it in `RUN_STATE.md` for the record and escalate yourself for that one decision rather than running the whole loop on the high tier.

First action:
- **No pane handed (default):** spawn worker-1 yourself — `plexi pane new -n "worker-1"`, launch the agent, poll for the booted prompt, set the model tier (see "Spawning an agent pane").
- **Pane handed:** confirm it is real and idle, and label it.
  ```
  plexi pane capture <PANE_ID> --from-cursor 0
  plexi pane name <PANE_ID> "worker-1"
  ```
  If it isn't a live agent prompt, don't stall the run — tell the user, then spawn a fresh worker-1 yourself and proceed.

## Verified command cheatsheet

Exact forms confirmed in live runs. Use them; don't invent variants.

| Need | Command |
|---|---|
| **Is the agent busy or idle?** | No single signal is trustworthy — use the "cheap triangle" below. Never gate a decision on one flag. |
| **Read a pane's progress slot (PRIMARY status channel)** | `plexi pane slot read <name> <pane_id>` — contract names: `status`, `pr`, `verdict`, `last_error`, `issue` |
| **Write a slot (the pane itself does this)** | `plexi pane slot write <name> <content> --replace` — pane defaults to `$PLEXI_PANE_ID`; there is **no positional pane-id argument** (use `--pane-id` to target another pane). `--replace` is required on every write after the first, or it errors `slot already exists`. Silent on success. |
| **Read the last N lines (default read)** | `plexi pane capture <id> --lines 20` |
| Read full pane buffer (verdict parsing only) | `plexi pane capture <id> --from-cursor 0` |
| Read only *new* output (delta) | `plexi pane capture <id> --from-cursor <CURSOR>` |
| Pane UI state as JSON | `plexi pane state <id>` |
| Type a prompt into the pane's TUI | `plexi pane send <id> "<text>"` |
| **Submit** the prompt | `plexi pane key <id> enter` |
| Interrupt the agent | `plexi pane key <id> ctrl+c` |
| Open a new terminal pane | `plexi pane new -n "<label>"` |
| Launch an agent in a shell pane | `plexi pane command <id> "co" --enter` (Codex) or `"c"` (Claude) |
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

### Progress channel — pane slots FIRST, capture is the fallback

**The primary way you read a pane's progress is its typed slots, not its scrollback.** A slot read is ~3 tokens and unambiguous; a full TUI capture is ~700 tokens and semantically fragile (false idle/busy reads, marker-glyph guessing, truncation, locale-sensitive Unicode). Read the declared state; scrape the TUI only when a slot is empty or stale.

**The pane-owned slot contract** (generic names — the slots belong to the pane; the babysitter is one reader). Worker and tester both honor it:

| Slot | Meaning |
|---|---|
| `status` | Current state, `<step-token>:<state>` — state is one of `working \| done \| blocked \| needs-input \| failed`. The step token (e.g. `batch3-impl`, `step4`) makes a stale value impossible to misread as current. |
| `pr` | PR number once opened. |
| `verdict` | Tester's final call: `PASS` or `FAIL`. |
| `last_error` | Short reason string when `status` is `blocked`/`failed`. |
| `issue` | Stint/issue id(s) the pane is working. |

Read the primary status slot on every check:

```
plexi pane slot read status <pane_id> 2>&1 | grep -v "sudo:"
```

**Write syntax — get this exact, or every worker burns its first two tool calls discovering it.** The write verb's argument shape is *not* the read verb's:

```
plexi pane slot write <name> <content> --replace     # what a pane writes about ITSELF
plexi pane slot read  <name> [pane_id]               # what the head reads about a pane
```

- `write` takes **no positional pane id**. The pane defaults to `$PLEXI_PANE_ID`; to target another pane use the `--pane-id <id>` *flag*. Passing the id positionally fails with `unexpected argument`.
- `--replace` is **required on every write after the first** — without it an existing slot errors `slot 'status' already exists — use --append to add to it or --replace to overwrite it`. Since `status` is written on every transition, always pass `--replace`.
- Success is **silent** (no output, exit 0); failure prints a named error and exits non-zero. Trust the exit code; never read the slot back to confirm a write.

**Freshness rule.** A slot value is only trustworthy if it names the current step. Two enforcement mechanisms, and every worker/tester brief must use one:
- Stamp a step/generation token into `status` on write (`step4:blocked`, `batch3-test:working`), OR
- `plexi pane slot delete status <pane_id>` at each step's start, so a stale value reads as *empty* (→ fall back to capture) rather than as current.

**Fallback to capture only when the slot is empty or its step token is stale.** The capture path (`--lines 20`, or full-buffer + `sed '1d' | jq -r '.lines[]'` chrome-strip for verdict parsing) is retained solely for that case and for reading a long verbatim report a slot can't hold.

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

Machine-readable progress lives in the pane's **slots** (`status`, `pr`, `verdict`, `last_error`, `issue`) — read them with `plexi pane slot read <name> <pane_id>` (see the slot-contract section above), not scrollback. The busy/idle screen-signal reads below are the *fallback* for when a slot is empty or its step token is stale.

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

(This is for typing into a pane's *TUI*. Launching the agent at the *shell* level in a fresh pane uses `plexi pane command <id> "co" --enter`.)

## Batch into as few PRs as possible (hard rule)

Before feeding anything, group the queue. **Fewer PRs is always better.** Small or related stints ship together in **one** PR — never one-PR-per-stint when they can combine. Parallelize implementation where stints don't share files, but collapse the result into the smallest number of PRs that makes sense. A **batch** is the unit of work below, not a single stint. Group by shared subsystem/files, small size, logical cohesion. When in doubt, combine.

## Spawning an agent pane

Workers and testers are agent panes launched with `co` (Codex) or `c` (Claude) — both bypass permissions.

```
plexi pane new -n "worker-<N>"          # or tester-<N>
# grab the new pane id from the command output (fall back to `plexi pane list`)
plexi pane command <newid> "co" --enter
sleep 4
plexi pane capture <newid> --from-cursor 0   # confirm the agent booted to its prompt
# then set the model tier before briefing: /model <name> two-step
```

**GOTCHA: BOOT RACE. Never fire a brief back-to-back on a just-launched agent pane.** Both TUIs take a few seconds to boot to their prompt. If a brief lands before the prompt is up, input is lost or mangled. Fix: after `plexi pane command <id> "co" --enter` (or `"c"`), poll `plexi pane capture <id> --from-cursor 0` until you see the booted prompt (the input line with the model footer, e.g. `gpt-...` for Codex) **before** sending the work brief.

## Model selection per pane

Set the model right after boot, per the tier table above: mid tier (`sonnet`/`opus` or `gpt-5.6-terra`) for simple mechanical batches, high tier (`fable` or `gpt-5.6-sol`) for hard ones. Testers usually run mid tier — their job is following a drive script, not design. Escalate a pane to high tier the moment it is fumbling/looping, and always for fix rounds (step 4).

## The loop

For each **batch**, in order:

1. **Worker: implement + open PR.** Send the worker a directive:
   > "Land stints `<ID>[, <ID>...]` **together in a single PR**. Run `stint show <ID>` for each, then claim them, implement in one worktree branched from alpha, and open ONE PR to alpha covering all of them per the repo AGENTS.md ship pipeline. **Stop once the PR is open and checks are green — do NOT invoke `/validate-pr`, and do NOT merge.** A separate tester pane owns validation. **Before pushing, run the full CI-equivalent local gate, not just `cargo test --bin plexi`**: `mypy sdk/python/plexi_sdk/ --ignore-missing-imports --check-untyped-defs --exclude testing.py`, and every `just gen-*-docs` / `just check-*-docs` generator touched by the diff (SDK, capability, config, authoring, CLI docs) — regenerate and commit any that are stale. `cargo test --bin plexi` passing is not sufficient evidence the PR is green; confirm with `gh pr checks <PR#>` after push and fix any red check before reporting done. **HARD GATE: paste the final green summary line of every suite you ran (`just test`, sdk pytest, mypy) in your reply — an unverified push gets bounced back without a tester round.** **Never end your turn while a build or test is still running — wait on it in the foreground and report its result.** **Publish your progress on your pane's typed slots so I can read your state cheaply instead of scraping your screen — this is required, not optional.** On every state transition write your `status` slot with a step token, event-driven (not on a timer): `plexi pane slot write status impl:working --replace` when you start, then `pr:working`, `blocked`, `needs-input`, or `failed` as they happen; write the PR number to `pr` and any blocker reason to `last_error` the same way. Write `impl:done` (with `pr` set) the moment the PR is open and green. **Exact syntax — do not improvise it:** `plexi pane slot write <name> <content> --replace`. There is no positional pane-id argument (the pane defaults to `$PLEXI_PANE_ID`), and `--replace` is mandatory on every write after the first or it errors `slot already exists`. Success is silent with exit 0; never read a slot back to confirm a write. Reply with the PR number when it's open and checks are green."

   These three brief clauses exist because they were the top token sinks in practice: workers running `/validate-pr` (the pipeline auto-chains `implement-stint` → `open-pr` → `validate-pr`, so a worker that follows the skills literally installs the build itself — a ~5 min compile the tester then repeats — and parks on a `[TESTING]` block waiting for a reply that never comes, which you then read as "idle with unfinished work" and burn a nudge on), workers pushing red (each unverified push burns a full tester round), and workers yielding their turn mid-compile (each yield burns a nudge round-trip). State all three up front in every worker brief, including fix-round briefs.

   **Memory ceiling on every suite run — `ulimit -v` DOES NOT WORK ON macOS.** Do not put it in a brief; it is worse than useless because it reads as protection. XNU does not enforce `RLIMIT_AS`, so `( ulimit -v 8388608; cargo test )` constrains nothing. Proven twice: a worker's test binary hit 22 GB (2026-07-21) and, *with the ulimit wrap in its brief*, 28 GB (2026-07-23, PID killed at 30.5/31.7 GB swap). The wrap did not fire either time.

   **Use an external watchdog instead**, and put it in the brief: run the suite with a background poller that kills any `target/debug/deps/plexi-*` process exceeding ~8 GB RSS, sampling `top -l 1 -o mem -n 5 -stats mem,pid,command` every 15s.

   **The head watches too.** Machine memory is a run-level resource, so a runaway is yours to catch, not just the worker's. When a worker has been in a suite for more than ~10 minutes, spend one call on it:
   ```
   top -l 1 -o mem -n 5 -stats mem,pid,command   # any deps/plexi-* in the GB range?
   sysctl -n vm.swapusage
   ```
   `ps -o` / `ps aux` are blocked in the head's sandbox — `top -l 1` and `pgrep -fl <name>` are the forms that work. `pgrep -fl` gives the full path, which is how you prove the process is a given worker's test binary rather than a real host. Kill it (`kill -9`), then tell the worker *immediately* that the suite failure it is about to see came from your kill and not from an assertion — otherwise it spends a fix round chasing a phantom test bug.

   **A multi-GB test balloon is a real bug in the diff, not flake.** Route it as a fix-round finding with the no-quick-fix framing (unbounded queue, wake feedback loop, an accumulating test adapter), never as "re-run it".

   **Tests inside a live pane inherit the host's channel — strip `PLEXI_*` in every worker brief's test command.** Worker panes run inside a running Plexi host, so `PLEXI_CHANNEL` / `PLEXI_CONTEXT_*` / `PLEXI_SOCKET` are in their environment. `cargo test` reads these and resolves workspace/config state to the host's *real* channel profile (e.g. `~/.plexi-beta/`), colliding across tests and producing flaky failures that a worker will burn hours chasing as if they were its own bug. In every worker brief, give the test command with those vars stripped: `env -u PLEXI_CHANNEL -u PLEXI_CONTEXT_ROOT -u PLEXI_CONTEXT_ID -u PLEXI_CONTEXT_NAME -u PLEXI_SOCKET -u PLEXI_RUNNING -u PLEXI_PANE_ID cargo test --bin plexi`. If you see a worker chasing `.plexi-<channel>` / `config_dir` / `temp_dir` test failures, that is this leak, not its diff — tell it so and hand it the stripped command. (Permanent fix tracked in the test-hermeticity stint; once that lands, `just test` strips these itself.)

   **The worker's gate is automated + headless only. NEVER put live-host-drive in a worker brief.** The worker runs `cargo test`/`mypy`/docs-gen and, at most, *headless* renders (`just scene` / `just scene-live` → a PNG). It must never be told to boot the installed GUI, foreground/activate the host window, or drive the running app — that is the tester's exclusive job (step 3), and it's the one thing only the tester pane does. This trap is easy to fall into because a stint's own **`## Done When`** frequently says "run a live channel-scoped host smoke" — do NOT copy that clause into the worker brief. When the stint's Done-When asks for live validation, that requirement is satisfied by the tester round, not by the worker. Observed cost: a worker handed "confirm terminals/Markdown/panes work in a live host" spent 40+ minutes stuck in macOS AppKit `NSRunningApplication.activate` returning `activated=false`, long after its code and Rust suite were green. If you catch a worker probing/foregrounding a live GUI, relieve it immediately: tell it the live smoke is the tester's job, confirm only the automated gate is green, and report the PR.

   **Verify, don't trust the worker's self-report.** After the worker replies with a PR number, you run `gh pr checks <PR#>` yourself before spawning the tester. If anything is red, send it straight back to the worker as a bug (same routing as a tester-found bug) — do not spawn the tester against a build with known-red CI.

   **RUST-ONLY PRs SHOW ONLY `claude` + CodeRabbit IN CI; that is GREEN, not incomplete.** The `typecheck` and `check-*-docs` GitHub jobs are conditional on Python (`sdk/python`) or docs changes. A pure-Rust-host PR (e.g. `src/*.rs` only) will NEVER spawn a `typecheck` job, so `gh pr checks` returning just `claude: skipping` + `CodeRabbit: pass` is a full green, not a half-reported run. Do NOT wait or poll for a typecheck that will never appear; that is a silent stall.

   **But green ≠ reviewed.** `CodeRabbit` currently reports "Review skipped: automatic reviews are disabled" on every PR, and `claude` skips on Rust-only diffs — neither is actually reading the code. The real gate for a Rust PR is the worker's local `cargo test --bin plexi` summary (make the worker paste it), the pre-push Codex review in `/implement-stint` Phase 4, and the tester round. Never tell a user a diff was "reviewed by CI."

   Send → `key enter` **once**. Confirm it registered by polling the **status bar** (`plexi pane capture <id> --lines 3`) until `esc to interrupt` appears — not by re-sending. A long brief often pastes as a collapsed block (`paste again to expand`) and needs exactly one more `enter` to submit; that is the only sanctioned re-send.

2. **Check in every 10 min — cheaply.** `ScheduleWakeup` `delaySeconds: 600`. On each wake, the default check is **one slot read** (~3 tokens):

   ```
   plexi pane slot read status <id> 2>&1 | grep -v "sudo:"   # e.g. batch3-impl:working
   ```

   Branch on the state token: `working` → reschedule, done. `done`/`blocked`/`needs-input`/`failed` → read `pr`/`verdict`/`last_error` for the details and act. **The slot is the source of truth when its step token is current.**

   **Only fall back to capture when the slot is empty or its step token is stale** (the pane hasn't adopted the contract, or crashed before writing). Then the old two-command read applies:

   ```
   plexi pane list 2>&1 | grep -v "sudo:"           # agent.state: working or idle
   plexi pane capture <id> --lines 20 2>&1 | grep -v "sudo:"   # agent's last message
   ```

   The 20-line tail almost always contains the verdict/reply/blocker; act on it directly.

   **Escalate to a sub-agent only when the tail is not enough** — a long tester report whose numbered bug list scrolled past 20 lines, or evidence that must be quoted verbatim from deep in the buffer. Sub-agent brief (pass the pane id):
   > "Inspect Plexi pane `<id>` for status only — do not touch it. Run `plexi pane list` for its `agent.state`, then `plexi pane capture <id> --from-cursor 0` and read the END of the buffer. Report ≤8 lines: (a) task/PR + phase, (b) working / idle / blocked / usage-limited, (c) verdict or bug list verbatim if present, (d) any question verbatim. Never re-send a command to force output. Ignore 'sudo:' noise lines."

   Act: progressing → reschedule 10 min. Idle-at-prompt / question / blocked → answer or nudge (send + enter). Errored / looping → `ctrl+c`, re-orient. Detect "done" by an explicit signal (PR number, "merged"), never by silence. **A worker idle with unfinished work gets nudged immediately, not next cycle.**

   **Waiting on a merge or any single external state change:** don't poll with wakeups or sub-agents — arm one background watch and act when it fires:
   ```
   Bash (run_in_background): until [ "$(gh pr view <PR#> --json state --jq .state)" != "OPEN" ]; do sleep 15; done; gh pr view <PR#> --json state,mergedAt
   ```

3. **Spawn the Tester (always a brand-new pane) once the PR is open.** Open a fresh pane with `co`, labeled `tester-<N>`, boot it (poll for the Codex prompt), then brief it. **Never reuse an existing tester pane** — every validation and every re-check gets its own new pane; the old one is closed once its verdict is read. First gate on the diff — judge from `gh pr diff <PR#> --name-only`:

   - **Docs/scripts/manifests-only, no runtime behavior change** → the tester does a diff review only and skips both the install and the suites. There is nothing to live-drive and the worker already ran the suites pre-push; re-running them here duplicates that.
   - **Anything else** → the install-and-drive brief below.

   > "Validate PR #`<PR#>` for Plexi. Install it with `just pr-install <PR#>` (safe from any cwd — the recipe resolves and builds the PR's actual head itself). **Before trusting any FAIL, confirm the fix is actually live in the installed build via a proof-of-fire signal**: drive the changed feature once and confirm its own `info` log line (or other new observable trace) fires in `~/.plexi-pr-<PR#>/plexi.log`, and check the head sha in the profile's `install.log` matches `gh pr view <PR#> --json headRefOid`. If the signal is absent, the build is stale or wrong-tree — report 'fix not present in installed build', never a behavior FAIL (a false FAIL here burns a full worker fix round; live incident 2026-07-21). Then **actually drive the installed `plexi-pr-<PR#>` build** — open the app, use the specific feature these stints added, and confirm end-to-end that it really works (not just that it compiles). Where the PR adds an assertion/validation/guard, prove it is **falsifiable**: deliberately violate it locally, watch it fail with a clear message, revert. Use the host's own primitives to observe it, never macOS `screencapture`/screen-recording (see below). Do NOT re-run test suites (`cargo test`, `just test`, pytest) — the worker already ran them green pre-push and pasted the summary; your job is exclusively behavior the suites can't see. **Operate fully autonomously: drive the host through the plexi CLI end to end and reach your own verdict — never ask a human to click, look at, or confirm anything.** Report a clear PASS, or a numbered list of concrete bugs/repro steps. Do not touch the code. **Publish your progress on your pane's typed slots (required) so I read your state cheaply instead of scraping your screen.** On every transition, event-driven, write your `status` slot with a step token: `plexi pane slot write status test:working --replace` when you start, then `test:done` on completion; write your final call to `verdict` (`PASS` or `FAIL`) and, on FAIL, a short reason to `last_error`, the same way. **Exact syntax — do not improvise it:** `plexi pane slot write <name> <content> --replace`. There is no positional pane-id argument (the pane defaults to `$PLEXI_PANE_ID`), and `--replace` is mandatory on every write after the first or it errors `slot already exists`. Success is silent with exit 0; never read a slot back to confirm a write."

   **Autonomous verification is the default, always.** In ~99% of cases the tester can prove or disprove the behavior itself by driving a real `plexi-pr-<N>` instance through the CLI (`pane state`, `pane capture`, the channel log, `scene-live` shots). Only when a check is *genuinely* impossible agentically — needs human eyes on physical hardware, audio, an external account — does the loop stop: surface a specific alert to the user naming exactly what needs human verification, and park that batch. Never let a tester convert "this is tedious to drive" into a user alert.

   **The tester validates behavior, not the diff.** It does not re-review code — AI diff review already happened once, pre-push, in `/implement-stint` Phase 4. Live-driving the real build is the thing only this pane can do; that is its entire job.

   **Never let the tester reach for macOS `screencapture`, screen recording, or any OS permission prompt to observe the host.** Plexi ships its own capture primitives that need zero OS permission — use them instead:
   - `plexi-pr-<N> pane state <id>` — normalized semantic tree (assert on `semantic.nodes`, not pixels).
   - `plexi-pr-<N> pane capture <id>` — terminal/output content.
   - `~/.plexi-pr-<N>/plexi.log` — every frame already logs `paint_fps`/`guest_fps`/`avg_host_ms`; this is FPS ground truth, not a screenshot.
   - For an actual headless screenshot (human-review only): `just scene-live <scene.toml> pr-<N>` — its `shot` step captures the whole host framebuffer via `drive-host` capture, no OS permission needed.
   - Full loop reference: `.agents/skills/drive-host/SKILL.md`.
   If a tester pane starts calling `screencapture` or a permission dialog shows up in its transcript, that's a sign it's improvising instead of following this brief — correct it immediately with the primitives above.

4. **Route the verdict.** Check the tester the same 10-min way (direct `--lines 20` read; sub-agent only for a long report).
   - **Near-pass FAIL** ("gate incomplete, not a regression" — the tester's own findings pass but one check couldn't run, was driven against the wrong artifact, or a trace was expected-absent) → do **not** open a fix round. Spawn a fresh **micro-check tester** scoped to exactly the unfinished check. A fix round on a near-pass burns a compact + escalation + full re-validation for a bug that does not exist. (Validated twice in live runs.)
   - **Brief-induced false FAIL — YOUR bug, not the tester's.** When a tester reports FAIL *and* says the substance is correct ("the new prose correctly describes the implementation, **but** it violates the specified criterion"), you wrote a bad criterion. **Never write a mechanical proxy — a grep returning nothing, an exact string absence, a line count — as the pass criterion for a semantic requirement.** Live case: the re-check brief demanded `git grep -F 'runtime = "python"'` return nothing; the correct fix legitimately kept that literal inside a fenced example of the *generated* `PACKAGE.toml` (annotated `# generated, never authored`) while the prose stated it is not a manifest key. Substance right, proxy failed. **Resolution: read the artifact yourself, overrule the FAIL on the record, log it as your error, and merge — do not open a fix round and do not make the worker satisfy a criterion you got wrong.** State the requirement semantically instead ("the header must not claim a nonexistent *authored manifest* key; showing the generated field is correct").
   - **Bugs found** → run the **fix-round protocol** on the worker, in order, before relaying anything:
     1. **Compact:** send `/compact` (two-step, then poll until it finishes) — the worker is about to reason hard and its implement transcript is dead weight.
     2. **Escalate the model:** if the worker isn't already on the high tier, `/model fable` (or `/model gpt-5.6-sol` on Codex).
     3. **Relay the tester report verbatim, wrapped in the no-quick-fix directive,** plus the same hard-gate + no-yield clauses as the initial brief:
     > "Tester found these on PR #`<PR#>`: <bug list>. **Do NOT quick-fix patch this.** A failure that survived your own gate is a high-level symptom of a lower-level problem — investigate the root cause first, and ask whether this reveals a chance to design the system so this *category* of bug is impossible in the future, not just this instance. Never reach for a hacky workaround because it's faster; time spent is not a constraint — optimize for long-term robustness, elegant system design, and current best practices. If the right fix is a real refactor, propose it to me before patching. Then: determine whether each bug is caused by your change or pre-existing on alpha (prove pre-existing with a baseline repro before claiming it). Verify the fix yourself the same way the tester found it, paste green suite summary lines, push, reply with the commit and a one-line root-cause statement."
     If the worker proves a bug pre-existing on alpha with baseline evidence, drop it from this PR's gate and have the worker file a follow-up stint for it — never scope-creep the PR.
     When the worker reports fixed, **close the previous tester pane and open a NEW `co` tester pane** for the re-check (never reuse the warm one), and give it a **targeted re-check, not a full re-run**:
     > "New commits pushed to PR #`<PR#>` (<what changed>). Re-install and re-validate ONLY the changed path thoroughly, plus a one-item smoke of the previously-passed area. Everything else already passed at <prior commit> — do not repeat it. PASS or bugs?"
     Loop worker ↔ (a fresh tester each round) until a tester returns a clean PASS. You are the only channel between them, and you hold the running summary of what already passed so each fresh tester only re-checks the delta.
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

6. **Retire the worker after EVERY task — never recycle a worker into the next one.**

   **A worker's context bloats with every task it carries.** An overloaded worker silently degrades — it re-reads files it already read, misses instructions in a long brief, and reasons from stale earlier tasks. Do not gamble on how much is "too much." The rule is simple and unconditional: **when a worker's batch is done (PR merged, or the batch abandoned), close its pane and spawn a brand-new one for the next batch.** No in-place `/clear`/`/new` reuse — a fresh pane costs seconds and removes the entire class of stale-transcript bugs.

   ```
   plexi pane close <worker-id>
   plexi pane new -n "worker-<N+1>"
   plexi pane command <newid> "co" --enter
   # poll for the booted prompt, set the model tier, then send a self-contained brief
   ```

   The information the next worker needs is **not** the old scrollback — it is a good self-contained brief. You already hold the distilled state (PR numbers, merge results, decisions, gotchas); put that in the brief and throw the transcript away. Never rely on "warm repo knowledge" as a reason to keep a pane alive.

   **Never retire mid-task.** A batch belongs to one worker from brief through merge, including fix rounds (which use `/compact` in place, step 4). Retire only at the clean boundary.

   (Testers likewise — fresh pane per validation, closed after each verdict; see step 3.)

7. **Hand off the head — fresh babysitter after every merge.** The single-use doctrine applies to YOU. See "Head handoff" below: update `RUN_STATE.md`, spawn your successor, verify its takeover, then retire yourself. Your context window carries exactly one batch of routing, ever.

## Head handoff — `RUN_STATE.md` and the fresh-head relay

The head agent rots like any other pane: skill reloads on wake, batch after batch of slot reads and verdicts. The fix is the same as for workers — single-use. **Each head owns exactly one batch (brief → merge), then relays to a fresh successor.** No in-place `/compact` for the head; compaction summaries drift, fresh spawns don't.

**`RUN_STATE.md`** (next to this SKILL.md) is the relay baton — and the reason a handoff brief carries almost nothing. Nothing about the run may live only in a head's context. Overwrite it at every merge boundary (~10 lines):

```
# Babysitter run state — overwritten at each merge boundary
updated: <UTC timestamp>
mode: sprints s9 s8 s4 | stints <remaining ids verbatim>
auto_merge: yes|no
merged: batch1 (0503+0532+0507) -> PR #2470; batch2 (0536+0534) -> PR #2471
next: batch3 = 0535+0457 (small, mid tier); note: 0531 unblocks after s8 four
gotchas: <run-scoped only — e.g. "0530 tester should smoke minimap too">
```

The full sprint queue is never carried in context or in the baton — sprint-queue mode re-resolves it from `stint sprint show` at takeover. Only an explicit stint list is copied verbatim into `mode:`.

**The relay, at each merge boundary (after step 6):**

1. Overwrite `RUN_STATE.md` per the template.
2. Spawn the successor: `plexi pane new -n "babysitter-<N+1>"`, launch `c`, poll for the booted prompt, set `/model opus` (two-step).
3. Send `/babysitter resume` (two-step, once).
4. **Wait for the takeover ack: poll `LOG.md`** (not the successor's screen) for its takeover line — the successor's first duty on resume is appending `HH:MM babysitter-<N+1> took over (after batch <N>, PR #<M>)` to `LOG.md`. File-poll beats screen-scrape: unambiguous, ~0 tokens.
5. Ack seen → stop scheduling wakeups; if you are running in a pane, `plexi pane close <own pane id>` as your final act. If you are the initial head running in a user session (not a pane), just report the successor's pane id and end the loop on your side.
6. **No ack within ~3 minutes** → capture the successor pane once; if the prompt never submitted, one more `enter` (the sanctioned re-send); if the pane is dead or wedged, close it, keep the run yourself, and retry the relay once at the next boundary. **Never close yourself before the ack** — an unconfirmed handoff orphans the run.

**On `/babysitter resume` (you are the successor):** read `RUN_STATE.md`; append your takeover line to `LOG.md` (this is the ack — do it first); re-resolve the queue (`stint sprint show` for sprint mode, the `mode:` list otherwise); prune checked items from `HUMAN_CHECKS.md`; continue the loop at `next:`. Do not re-verify prior merges beyond what `RUN_STATE.md` claims — your predecessor already verified them with `gh`/`stint`; trust the baton, it exists so you start clean.

## Human-check queue — `HUMAN_CHECKS.md` (next to this SKILL.md)

Some stints carry a **`## Human Check`** section in their body: a validation only a human can perform (visual taste, audible sound, an external account). These never block the run and never park a batch. The contract:

- The tester validates everything else as normal; a Human Check item is **out of the tester's scope** and its absence is not a FAIL.
- Any evidence the check needs (before/after PNGs, scene shots) is produced by the worker or tester into a **stable path** — `.stint/evidence/<stint-id>-<name>.png` — never a pane scratch dir or a `pr-<N>` profile that gets reaped.
- **At merge**, append one checkbox entry to `HUMAN_CHECKS.md`:

  ```
  - [ ] PR #<N> (<stint>: <title>): <one-line instruction>. Evidence: <paths>. Judging: <what the human is deciding>.
  ```

- **At run start**, prune entries the user has checked off (`- [x]`), so the file stays a short live to-do list, not an archive.
- In the end-of-run summary, point the user at the file and count its open items.

This file is the durable artifact — never keep the pending-checks list only in your own context.

## Run log — `LOG.md` (next to this SKILL.md)

The loop keeps a self-improvement log at `.agents/skills/babysitter/LOG.md`. It is telemetry about the **workflow**, not the codebase — its whole purpose is to make future babysitter runs cheaper and smoother. Append as you go; don't reconstruct at the end from memory.

**Log an entry (UTC timestamp + a few lines) at each of these moments:**
- Worker briefed: batch stints, agent (`c`/`co`), model tier, pane label, start time.
- PR opened: PR#, elapsed since brief.
- Each tester verdict: PASS/bugs, elapsed, attempt number.
- Merge: total wall-clock brief→merge, number of worker↔tester rounds.
- **Any workflow friction, immediately:** an agent forgot part of its brief, a redundant/wasted tool call, a thrash loop caught, an unclear brief that needed a nudge, a gotcha not yet in this skill. These are the highest-value lines in the file.

**Sprints.** The queue runs in sprint blocks as detailed in the stint tasks. At the end of each sprint, append a **Sprint Recap**: what landed (stints → PRs), per-worker timing and try-count table, what we learned, and concrete suggestions to streamline the babysitter workflow (fewer tool calls, less token churn, brief wording fixes). When a recap suggestion is validated in practice, promote it into this SKILL.md and note the promotion in the log.

Format (keep entries terse):

```
## 2026-07-21 — sprint <name/ids>
- 18:04 worker-1 briefed: stints 0501+0502, co/gpt-5.6-terra
- 18:41 PR #2460 open (37m)
- 19:10 tester-1: 2 bugs (attempt 1)
- 19:12 friction: worker ran /validate-pr despite brief clause — reword?
- 20:02 tester-2: PASS (attempt 2)
- 20:05 merged (2h01m, 2 rounds)
### Sprint recap
...
```

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
- **Label every pane. Every pane is single-use — including the head.** Testers: close after each verdict, fresh pane per validation/re-check. Workers: one batch per pane, close + respawn at the boundary (step 6). The head: one batch per head, relay via `RUN_STATE.md` + `/babysitter resume` at each merge (step 7). Never carry a stale transcript into a new task.
- **The run's state lives on disk, never only in context.** `RUN_STATE.md` (baton), `LOG.md` (telemetry), `HUMAN_CHECKS.md` (pending human validations), `.stint/` (the queue). Any head must be replaceable at any moment by `/babysitter resume`.
- **Log as you go.** Timestamped events into `LOG.md` next to this skill; sprint recap at each sprint boundary (see Run log).
- Verify state with `gh`/`stint`, not any pane's self-report.
- Keep the 10-min cadence via `ScheduleWakeup`; nudge rather than wait passively.
