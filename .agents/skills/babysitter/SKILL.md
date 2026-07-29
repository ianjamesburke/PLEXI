---
name: babysitter
description: "Land a queue of stint tasks as fast as possible by orchestrating agent instances (Claude or Codex, launched via size-tier aliases) in Plexi panes. You give it stints (or sprints); it spawns every pane it needs: a WORKER pane implements each batch and opens one PR, a separate fresh TESTER pane validates the PR against the real install build, bugs route between them until it passes, then merge and fresh panes for the next batch. The head relays itself to a fresh pane after every merge via RUN_STATE.md. You are the router, never the coder. Checks in every 10 min. Triggered by /babysitter, \"babysit the queue\", \"queue these stints\"."
source: local
date_added: "2026-07-11"
---

# Babysitter — Orchestrate Worker + Tester Panes Through a Stint Queue

You are the **HEAD AGENT — a router and coordinator, not a coder.** You never touch code, git, or the repo yourself. Your sole job is to keep your own context window small and route messages: outsource every piece of real work — implementation, testing, research, reading long reports, any multi-step investigation — to agent panes. If a step would put more than ~20 lines of someone else's output into your context, delegate it. You hold distilled state (PR numbers, verdicts, decisions, timings), never scrollback. Two roles exist per batch:

- **Worker** — a pane that implements the batch and opens the PR.
- **Tester** — a *separate, fresh* pane that installs the PR build, drives the app to verify it, and reports bugs back to you.

**Panes launch Claude or Codex through the size-tier aliases — `cs`/`cm`/`cl` (Claude small/medium/large) and `cos`/`com`/`col` (Codex equivalents) — all bypass permissions.** Never use bare `claude`/`codex`. The tier→model mapping lives only in the user's zshrc — pick a tier by alias, never by model id (ids go stale). The two TUIs use the same `pane send --submit` path and `/compact`. The one divergence: fresh-conversation reset is `/clear` in Claude, `/new` in Codex. Examples below use Codex aliases; substitute the Claude ones freely.

### Model tiers — medium (`cm`) is the worker default

| Tier | Claude | Codex | Use for |
|---|---|---|---|
| Small | `cs` | `cos` | genuinely trivial, mechanical batches only |
| Medium | `cm` | `com` | most batches — the standard worker tier |
| Large | `cl` | `col` | hard/ambiguous batches, any pane that is fumbling or looping, every fix round |

The tier is chosen at launch: pass the alias as the shell command in `plexi pane command` — there is no post-boot `/model` step. Judge difficulty from the stint bodies before spawning; when unsure, use medium, not small. `/model` exists solely for mid-run escalation on a warm pane. **Every fix round runs on the large tier** — see step 4.

### Engine policy — which family a fresh worker/tester gets

Read `ENGINE_POLICY` (one line, next to this skill) **at every worker/tester spawn**, not once per run:

- `codex-first` (the standing default): spawn Codex aliases (`cos`/`com`/`col`). If the pane boots into a usage/rate-limit error instead of a live prompt, close it, respawn the same brief on the Claude alias of the same tier, and log the fallback in `LOG.md` — that is expected degradation, not an incident. Once one Codex spawn has hit the limit, go straight to Claude for the rest of the run (re-probe Codex only on a new day).
- `claude-first`: the mirror image — Claude aliases first, Codex as the fallback.

The head itself stays on `cm` per "Model tiers" regardless of policy — continuity of the loop matters more than engine parity. A missing `ENGINE_POLICY` file means `codex-first`. Ian flips the policy by editing the file's one word; never hardcode an engine choice in a brief.

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

**The head runs the medium tier (`cm`) by default.** Routing, slot reads, and brief relaying do not need the large tier — the hard reasoning happens in worker panes, which get their own tier per batch. If the head itself hits a genuine judgment call (ambiguous pre-existing-bug ruling, a stop-condition decision), note it in `RUN_STATE.md` for the record and escalate yourself for that one decision rather than running the whole loop on the large tier.

First action:
- **No pane handed (default):** spawn worker-1 yourself with `plexi pane new -n "worker-1" --agent com` (see "Spawning an agent pane").
- **Pane handed:** confirm it is real and idle, and label it.
  ```
  plexi pane capture <PANE_ID> --from-cursor 0 --plain
  plexi pane name <PANE_ID> "worker-1"
  ```
  If it isn't a live agent prompt, don't stall the run — tell the user, then spawn a fresh worker-1 yourself and proceed.

## Verified command cheatsheet

Exact forms confirmed in live runs. Use them; don't invent variants.

| Need | Command |
|---|---|
| **Is the agent busy or idle?** | `plexi pane status <id>` — use its host-derived verdict and confidence; `unknown`/`low` means escalate, not guess. |
| **Read a pane's progress slot (PRIMARY status channel)** | `plexi pane slot read <name> <pane_id>` — contract names: `status`, `pr`, `verdict`, `last_error`, `issue` |
| **Write a slot (the pane itself does this, never the head)** | Pane-slot write contract in `.agents/skills/implement-stint/SKILL.md` (Worker Mode) — the single home of the write syntax |
| **Read the last N lines (default read)** | `plexi pane capture <id> --lines 20` |
| Read full pane buffer (verdict parsing only) | `plexi pane capture <id> --from-cursor 0 --plain` |
| Read only *new* output (delta) | `plexi pane capture <id> --from-cursor <CURSOR> --plain` |
| Pane UI state as JSON | `plexi pane state <id>` |
| Send and submit a prompt into the pane's TUI | `plexi pane send <id> "<text>" --submit` |
| Interrupt the agent | `plexi pane key <id> ctrl+c` |
| Open a new terminal pane | `plexi pane new -n "<label>"` |
| Launch a ready agent pane | `plexi pane new -n "<label>" --agent com` — the tier alias (`cs`/`cm`/`cl`, `cos`/`com`/`col`) picks the model |
| Escalate a warm pane's model mid-run (only use of `/model`) | `plexi pane send <id> "/model <name>" --submit` |
| Rename / label a pane | `plexi pane name <id> "<label>"` |
| List panes (alive? find by name) | `plexi pane list` |
| Close a pane | `plexi pane close <id>` |

**Label every pane** (`worker-N`, `tester-N`) so you can find them in `plexi pane list` by name instead of tracking bare ids.

**Backtick trap:** never put backticks in the text you pass to `plexi pane send` from a double-quoted shell string — they trigger command substitution and the send fails or mangles. Write commands and paths bare in briefs.

### AI broker key on `pr-<N>` channels — export `PLEXI_TEST_OPENROUTER_API_KEY`

Every `plexi-pr-<N>` binary is signed differently, so its first Keychain read
raises a macOS access dialog only a human can click — which silently stalls an
unattended tester mid-run. On a `pr-<N>` channel only, the broker takes the key
from `PLEXI_TEST_OPENROUTER_API_KEY` and skips the Keychain entirely
(`test_channel_api_key` in `src/plexi_ai/broker.rs`). Export it in the pane that
starts the PR host whenever the stints under test touch the assistant or
`ai.query`:

```bash
export PLEXI_TEST_OPENROUTER_API_KEY="$(grep -m1 '^OPENROUTER_API_KEY=' .env | cut -d= -f2-)"
```

Unset, the Keychain path runs unchanged, so the dialog is back. `alpha`, `beta`,
and `main` never read this var: the broker requires both the runtime `pr-<N>`
name and the matching compile-time marker embedded by `scripts/install.sh`.
Renaming a real-channel binary is insufficient. Do not try to use the var to
configure a real build. Never echo or paste the value.

### Capture forms — `--lines 20` is the default; full-buffer reads are the exception

Since stint 0383 (PR #2390), `capture --lines N` returns the last N real content lines on full-screen TUIs. Cost order, cheapest first:

- `plexi pane capture <id> --lines 20` → bounded tail. **Default for every status check** — it answers "what is the agent's last message" in ~20 lines.
- `plexi pane capture <id> --from-cursor <CURSOR> --plain` → raw delta since a known cursor; stdout is only captured text and stderr carries the next `cursor=<N>`.
- `plexi pane capture <id> --from-cursor 0 --plain` → raw full buffer. Only when parsing a long verdict/report whose start you'd otherwise miss. Prefer narrowing it with `grep`/`sed` over dumping the whole thing; delegate to a sub-agent only when the report is genuinely too long to narrow.

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

Block on the primary status slot instead of polling it:

```
plexi pane slot wait status <pane_id> --until ':(done|blocked|needs-input|failed)$' --timeout 600
```

**Panes write; the head only reads.** The exact write syntax lives in one place — the pane-slot write contract in `.agents/skills/implement-stint/SKILL.md` (Worker Mode); point any pane there rather than restating it. The head's read verb is `plexi pane slot read <name> [pane_id]` — note its argument shape is *not* the write verb's.

**Freshness rule.** A slot value is only trustworthy if it names the current step. Two enforcement mechanisms — workers get theirs from the Worker Mode contract; every tester brief must pick one:
- Stamp a step/generation token into `status` on write (`step4:blocked`, `batch3-test:working`), OR
- `plexi pane slot delete status <pane_id>` at each step's start, so a stale value reads as *empty* (→ fall back to capture) rather than as current.

**Fallback to capture only when the slot is empty or its step token is stale.** The capture path (`--lines 20`, or full-buffer `--plain` for verdict parsing) is retained solely for that case and for reading a long verbatim report a slot can't hold.

### Reading whether a pane is busy

Use `plexi pane status <id>`. It returns the host's `working`, `idle`, `blocked`, or `unknown` verdict, confidence, and raw evidence in one response. Act on `working` or a high-confidence `idle`; treat `blocked`, `unknown`, or low confidence as an escalation signal. Machine-readable progress still lives in the pane's slots (`status`, `pr`, `verdict`, `last_error`, `issue`); use status when that state is missing or stale.

### ANTI-THRASH — never re-fire a command to "confirm" it landed

The failure mode this skill exists to prevent: submit a prompt → capture comes back empty → assume it did not land → submit again → loop forever. **Banned.**

- Send a prompt once with `plexi pane send <id> "<prompt>" --submit`. Its exit code is the submission confirmation; never re-send it to check.
- If a capture is empty or a cursor is unchanged, that tells you nothing. Do **not** act on it and do **not** repeat the command. Fall back to `pane status` / slot state.
- If you catch yourself about to run the exact same command a **second** time in a row, STOP. Something is wrong with your read path, not the pane. Re-orient via `pane list`.

### Prompt submission

```
plexi pane send <id> "<prompt text>" --submit
```

The host settles the input, presses Enter, confirms submission, and self-heals one collapsed paste. Exit 0 is confirmed; a non-zero result is typed-but-unconfirmed with the observed input line on stderr. This is for a pane's *TUI*; use `pane command ... --enter` only for ordinary shell commands.

## Batch into as few PRs as possible (hard rule)

Before feeding anything, group the queue. **Fewer PRs is always better.** Small or related stints ship together in **one** PR — never one-PR-per-stint when they can combine. Parallelize implementation where stints don't share files, but collapse the result into the smallest number of PRs that makes sense. A **batch** is the unit of work below, not a single stint. Group by shared subsystem/files, small size, logical cohesion. When in doubt, combine.

## Spawning an agent pane

Workers and testers are agent panes launched with a size-tier alias (tier table above) — the alias sets the model, so there is no post-boot `/model` step.

```
NEWID=$(plexi pane new -n "worker-<N>" --agent com)  # or tester-<N>; tier aliases: cs/cm/cl or cos/com/col
```

The command blocks until the agent reports a booted idle prompt and prints its pane id only then. Exit 2 means boot timeout; the created pane id is on stderr so close it before retrying. Send the brief only after exit 0.

## Model selection per pane

Pick the tier at launch via the alias: workers default to medium (`cm`/`com`); drop to small (`cs`/`cos`) only for genuinely trivial mechanical batches, and start on large (`cl`/`col`) for hard or ambiguous ones. Testers launch medium (`cm`/`com`) — driving a real build and judging PASS/FAIL is not trivial mechanical work. Mid-run escalation is the only use of `/model`: escalate a warm pane to the large-tier model the moment it is fumbling/looping, and always for fix rounds (step 4).

## The loop

For each **batch**, in order:

1. **Worker: implement + open PR.** The brief is always the skill invocation plus a freeform overlay — never a restatement of mechanics:
   > /implement-stint worker <ID> [<ID> ...]
   >
   > <overlay — run-specific facts only: scope notes, decisions not yet in the task bodies, env quirks, cross-batch gotchas, fix-round relays>

   The Worker Mode contract in `.agents/skills/implement-stint/SKILL.md` owns every worker mechanic — one worktree and one PR per batch, stop at PR open + green checks (no `/validate-pr`, no merge), the headless-only gate, env-stripped suites, the memory watchdog, pasted suite evidence, no mid-build yields, and slot publishing. **Overlays add run-specific facts; they never restate or contradict the skill's mechanics.** In particular, never write a memory ceiling — `ulimit -v` or any variant — into an overlay: it constrains nothing on macOS (Worker Mode owns the why and the real watchdog), so it reads as protection while providing none.

   **Never copy a stint's Done-When live-validation clause into an overlay.** The worker's gate is headless-only (Worker Mode owns that rule); live-driving the installed build is the tester's exclusive job (step 3), and the tester round satisfies the Done-When. If you catch a worker probing or foregrounding a live GUI, relieve it immediately: tell it the live smoke is the tester's job, confirm the automated gate is green, and take the PR.

   **The head watches memory too.** Machine memory is a run-level resource, so a runaway suite is yours to catch as well as the worker's (its own watchdog is Worker Mode's). When a worker has been in a suite for more than ~10 minutes, spend one call on it:
   ```
   top -l 1 -o mem -n 5 -stats mem,pid,command   # any deps/plexi-* in the GB range?
   sysctl -n vm.swapusage
   ```
   `ps -o` / `ps aux` are blocked in the head's sandbox — `top -l 1` and `pgrep -fl <name>` are the forms that work. `pgrep -fl` gives the full path, which is how you prove the process is a given worker's test binary rather than a real host. Kill it (`kill -9`), then tell the worker *immediately* that the suite failure it is about to see came from your kill and not from an assertion — otherwise it spends a fix round chasing a phantom test bug. Route a memory balloon as a fix-round finding (Worker Mode: it is a real bug in the diff, not flake), never as "re-run it".

   **If a worker is chasing `.plexi-<channel>` / `config_dir` / `temp_dir` test failures**, that is the host-env leak Worker Mode's env-stripped test command exists to prevent — tell it so and point it back at that command. (`just test` already strips these vars itself; the leak bites direct `cargo test` invocations.)

   **Verify, don't trust the worker's self-report.** After the worker replies with a PR number, you run `gh pr checks <PR#>` yourself before spawning the tester. If anything is red, send it straight back to the worker as a bug (same routing as a tester-found bug) — do not spawn the tester against a build with known-red CI.

   **RUST-ONLY PRs SHOW ONLY `claude` + CodeRabbit IN CI; that is GREEN, not incomplete.** The `typecheck` and `check-*-docs` GitHub jobs are conditional on Python (`sdk/python`) or docs changes. A pure-Rust-host PR (e.g. `src/*.rs` only) will NEVER spawn a `typecheck` job, so `gh pr checks` returning just `claude: skipping` + `CodeRabbit: pass` is a full green, not a half-reported run. Do NOT wait or poll for a typecheck that will never appear; that is a silent stall.

   **But green ≠ reviewed.** `CodeRabbit` currently reports "Review skipped: automatic reviews are disabled" on every PR, and `claude` skips on Rust-only diffs — neither is actually reading the code. The real gate for a Rust PR is the worker's local `cargo test --bin plexi` summary (Worker Mode makes it paste one), the pre-push Codex review in `/implement-stint` Phase 4, and the tester round. Never tell a user a diff was "reviewed by CI."

   Send the brief once with `plexi pane send <id> "<brief>" --submit`. Its exit code confirms submission; do not re-send to infer delivery.

2. **Wait for progress — cheaply.** Arm one host-side wait for the current worker state:

   ```
   plexi pane slot wait status <id> --until ':(done|blocked|needs-input|failed)$' --timeout 600
   ```

   Exit 0 prints the matched value; exit 2 is a timeout, so arm another wait; exit 1 is usage/plumbing. On a terminal state, read `pr`/`verdict`/`last_error` for details and act. **The slot is the source of truth when its step token is current.**

   **PROGRESS BLOCK — end every wake-up's final message with it.** The user glances at this instead of reconstructing state from tool calls. Three lines, always the same shape (`date +%H:%M` for the LOCAL time — never UTC; counts from your own distilled state, verified against `stint show` at merge boundaries):

   ```
   ⏱ 12:52 — 23/25 stints done
   in flight: 0518 impl (worker-b11b) | queued: 0519 (waits 0518)
   state: nominal — no fix rounds, next check ~13:02
   ```

   Line 1: current local time + done/total for THIS run's queue. Line 2: what is mid-flight (batch, phase, who) and what is queued next. Line 3: one plain-English clause on health — `nominal`, or the active incident (`pane spawn outage — sub-agent workaround`), plus when the next check fires. Emit it even on a nothing-changed wake; skip it only on turns that are pure verdict-routing between wakes.

   **EXCEPTION — a pane blocked on a permission prompt cannot update its own slot.** It is frozen mid-tool-call, so its last-written value (`…:working`) stays fresh-looking. After a timeout, query `pane status`; a `blocked` verdict with a Bash detail names a permission prompt. Approve once only after confirming the path is a self-created `/tmp` scratch dir. Codex aliases bypass most permissions but can still prompt on `rm -rf`.

   **Only fall back to capture when the slot is empty or its step token is stale** (the pane hasn't adopted the contract, or crashed before writing). Use the host verdict and bounded tail:

   ```
   plexi pane status <id>
   plexi pane capture <id> --lines 20   # agent's last message
   ```

   The 20-line tail almost always contains the verdict/reply/blocker; act on it directly.

   **Escalate to a sub-agent only when the tail is not enough** — a long tester report whose numbered bug list scrolled past 20 lines, or evidence that must be quoted verbatim from deep in the buffer. Sub-agent brief (pass the pane id):
   > "Inspect Plexi pane `<id>` for status only — do not touch it. Run `plexi pane status <id>`, then `plexi pane capture <id> --from-cursor 0 --plain` and read the end of the buffer. Report ≤8 lines: (a) task/PR + phase, (b) working / idle / blocked / usage-limited, (c) verdict or bug list verbatim if present, (d) any question verbatim. Never re-send a command to force output."

   Act: progressing → arm another wait. Idle-at-prompt / question / blocked → answer or nudge with `pane send --submit`. Errored / looping → `ctrl+c`, re-orient. Detect "done" by an explicit signal (PR number, "merged"), never by silence. **A worker idle with unfinished work gets nudged immediately, not next cycle.**

   **Waiting on a merge or any single external state change:** don't poll with wakeups or sub-agents — arm one background watch and act when it fires:
   ```
   Bash (run_in_background): until [ "$(gh pr view <PR#> --json state --jq .state)" != "OPEN" ]; do sleep 15; done; gh pr view <PR#> --json state,mergedAt
   ```

3. **Spawn the Tester (always a brand-new pane) once the PR is open.** Open a fresh pane with the tester's tier alias (medium — `cm`/`com`) using `pane new -n "tester-<N>" --agent <alias>`, then brief it after it returns the ready pane id. **Never reuse an existing tester pane** — every validation and every re-check gets its own new pane; the old one is closed once its verdict is read. First gate on the diff — judge from `gh pr diff <PR#> --name-only`:

   - **Docs/scripts/manifests-only, no runtime behavior change** → the tester does a diff review only and skips both the install and the suites. There is nothing to live-drive and the worker already ran the suites pre-push; re-running them here duplicates that.
   - **Pure library/model crate with no host wiring yet** (a new `crates/*` edit model, a `cfg(test)`-only core, anything the host does not call yet) → **explicitly tell the tester NOT to `pr-install` and NOT to boot a host**, and brief *adversarial API + invariant validation* instead. There is no installed-build surface, so a live-drive brief manufactures a false FAIL for "no live surface" (proven live, more than once). What to demand instead: **prove the gate is not vacuous** by deliberately breaking an invariant in a scratch copy and confirming the fuzzer/gate catches it (an invariant suite that passes on a knowingly-broken model is the most likely defect in this shape); undo/redo round-trips byte-identical, including a destructive command and interleaved undo-then-new to check redo invalidation; and **the command enum is the only mutation path** — any `pub` field or setter that bypasses the command log silently breaks undo and every downstream consumer.
   - **Anything else** → the install-and-drive brief below.

   > "Validate PR #`<PR#>` for Plexi. Install it with `just pr-install <PR#>` (safe from any cwd — the recipe resolves and builds the PR's actual head itself); installation does not boot the host, so then run `plexi-pr-<PR#> host start --background`. **Before trusting any FAIL, confirm the fix is actually live in the installed build via a proof-of-fire signal**: drive the changed feature once and confirm its own `info` log line (or other new observable trace) fires in `~/.plexi-pr-<PR#>/plexi.log`, and check the head sha in the profile's `install.log` matches `gh pr view <PR#> --json headRefOid`. If the signal is absent, the build is stale or wrong-tree — report 'fix not present in installed build', never a behavior FAIL (a false FAIL here burns a full worker fix round — proven live). Then **actually drive the installed `plexi-pr-<PR#>` build** — open the app, use the specific feature these stints added, and confirm end-to-end that it really works (not just that it compiles). Where the PR adds an assertion/validation/guard, prove it is **falsifiable**: deliberately violate it locally, watch it fail with a clear message, revert. Use the host's own primitives to observe it, never macOS `screencapture`/screen-recording (see below). Do NOT re-run test suites (`cargo test`, `just test`, pytest) — the worker already ran them green pre-push and pasted the summary; your job is exclusively behavior the suites can't see. **Operate fully autonomously: drive the host through the plexi CLI end to end and reach your own verdict — never ask a human to click, look at, or confirm anything.** Report a clear PASS, or a numbered list of concrete bugs/repro steps. Do not touch the code. **Publish your progress on your pane's typed slots (required) so I read your state cheaply instead of scraping your screen.** On every transition, event-driven, write your `status` slot with a step token (`test:working` at start, `test:done` on completion); write your final call to `verdict` (`PASS` or `FAIL`) and, on FAIL, a short reason to `last_error`, the same way. The exact write syntax is the pane-slot write contract in .agents/skills/implement-stint/SKILL.md (Worker Mode section) — follow it, do not improvise it."

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
   - **Near-pass FAIL** ("gate incomplete, not a regression" — the tester's own findings pass but one check couldn't run, was driven against the wrong artifact, or a trace was expected-absent) → do **not** open a fix round. Spawn a fresh **micro-check tester** scoped to exactly the unfinished check. A fix round on a near-pass burns a compact + escalation + full re-validation for a bug that does not exist (proven live).
   - **Brief-induced false FAIL — YOUR bug, not the tester's.** When a tester reports FAIL *and* says the substance is correct ("the new prose correctly describes the implementation, **but** it violates the specified criterion"), you wrote a bad criterion. **Never write a mechanical proxy — a grep returning nothing, an exact string absence, a line count — as the pass criterion for a semantic requirement.** (Proven live: a grep-returns-nothing criterion failed a correct fix that legitimately kept the string inside a fenced example.) **Resolution: read the artifact yourself, overrule the FAIL on the record, log it as your error, and merge — do not open a fix round and do not make the worker satisfy a criterion you got wrong.** State the requirement semantically instead ("the header must not claim a nonexistent *authored manifest* key; showing the generated field is correct").
   - **Bugs found** → run the **fix-round protocol** on the worker, in order, before relaying anything:
     1. **Compact:** send `/compact` with `pane send --submit`, then wait for its terminal slot state — the worker is about to reason hard and its implement transcript is dead weight.
     2. **Escalate the model:** if the worker isn't already on the large tier, `/model` it to the large-tier model — the one the `cl`/`col` alias launches. Resolve the name when you need it: `grep -E "alias (cl|col)=" ~/dotfiles/zshrc` shows the `--model`/`-m` value.
     3. **Relay the tester report verbatim, wrapped in the no-quick-fix directive.** The Worker Mode contract still governs the fix round (headless gate, evidence, slots, no yields) — reference it, never restate it:
     > "Tester found these on PR #`<PR#>`: <bug list>. **Do NOT quick-fix patch this.** A failure that survived your own gate is a high-level symptom of a lower-level problem — investigate the root cause first, and ask whether this reveals a chance to design the system so this *category* of bug is impossible in the future, not just this instance. Never reach for a hacky workaround because it's faster; time spent is not a constraint — optimize for long-term robustness, elegant system design, and current best practices. If the right fix is a real refactor, propose it to me before patching. Then: determine whether each bug is caused by your change or pre-existing on alpha (prove pre-existing with a baseline repro before claiming it). Verify the fix with the closest automated/headless repro of the tester's finding (the live re-check stays the tester's), satisfy the Worker Mode gate again, push, reply with the commit and a one-line root-cause statement."
     If the worker proves a bug pre-existing on alpha with baseline evidence, drop it from this PR's gate and have the worker file a follow-up stint for it — never scope-creep the PR.
     When the worker reports fixed, **close the previous tester pane and open a NEW tester pane (its tier alias)** for the re-check (never reuse the warm one), and give it a **targeted re-check, not a full re-run**:
     > "New commits pushed to PR #`<PR#>` (<what changed>). Re-install and re-validate ONLY the changed path thoroughly, plus a one-item smoke of the previously-passed area. Everything else already passed at <prior commit> — do not repeat it. PASS or bugs?"
     Loop worker ↔ (a fresh tester each round) until a tester returns a clean PASS. You are the only channel between them, and you hold the running summary of what already passed so each fresh tester only re-checks the delta.
   - **PASS** → proceed to merge.

5. **Merge — after tester PASS *and* the human gate.** A PASS is not permission to merge.

   **Default (always, unless the user opted out): surface the passing PR and wait.** Report the verdict and ask for explicit merge approval. Do not proceed on your own judgment that the evidence looks strong — that is exactly the reasoning that merges something the user wanted to see first.

   **Only skip the wait when the user has explicitly opted into auto-merge for this queue** (see Rules). If they have, merge on PASS and roll straight to the next batch.

   **COMMIT YOUR OWN SKILL EDITS BEFORE THE MERGE.** `just merge-pr` has a **dirty-tree gate**, and the head writes repo-tracked babysitter files all run long, so this trips on *every* batch. Worse, the sync step overwrites the working tree — an uncommitted `SKILL.md` edit is **silently destroyed** by the merge (proven live). `LOG.md` is gitignored and safe; `SKILL.md` is tracked and is not. So at each merge boundary, *before* handing the merge to the worker, have it commit your `SKILL.md` changes as a small `chore(babysitter):` commit. **Commit, never `git checkout --`** — a commit preserves, a checkout discards, and by then there is no copy anywhere else.

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
   plexi pane new -n "worker-<N+1>" --agent com   # tier alias picks the model; stdout is a ready pane id
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
next: batch3 = 0535+0457 (small, default tier); note: 0531 unblocks after s8 four
gotchas: <run-scoped only — e.g. "0530 tester should smoke minimap too">
```

The full sprint queue is never carried in context or in the baton — sprint-queue mode re-resolves it from `stint sprint show` at takeover. Only an explicit stint list is copied verbatim into `mode:`.

**The relay, at each merge boundary (after step 6):**

1. Overwrite `RUN_STATE.md` per the template.
2. Spawn the successor: `plexi pane new -n "babysitter-<N+1>" --agent cm` (the head's default tier).
3. Send `/babysitter resume` once with `plexi pane send <id> "/babysitter resume" --submit`.
4. **Wait for the takeover ack: poll `LOG.md`** (not the successor's screen) for its takeover line — the successor's first duty on resume is appending `HH:MM babysitter-<N+1> took over (after batch <N>, PR #<M>)` to `LOG.md`. File-poll beats screen-scrape: unambiguous, ~0 tokens.
5. Ack seen → stop scheduling wakeups; if you are running in a pane, `plexi pane close <own pane id>` as your final act. If you are the initial head running in a user session (not a pane), just report the successor's pane id and end the loop on your side.
6. **No ack within ~3 minutes** → inspect the successor once with `pane status`; if it is dead or wedged, close it, keep the run yourself, and retry the relay once at the next boundary. **Never close yourself before the ack** — an unconfirmed handoff orphans the run.

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
- Worker briefed: batch stints, launch alias (tier), pane label, start time.
- PR opened: PR#, elapsed since brief.
- Each tester verdict: PASS/bugs, elapsed, attempt number.
- Merge: total wall-clock brief→merge, number of worker↔tester rounds.
- **Any workflow friction, immediately:** an agent forgot part of its brief, a redundant/wasted tool call, a thrash loop caught, an unclear brief that needed a nudge, a gotcha not yet in this skill. These are the highest-value lines in the file.

**Sprints.** The queue runs in sprint blocks as detailed in the stint tasks. At the end of each sprint, append a **Sprint Recap**: what landed (stints → PRs), per-worker timing and try-count table, what we learned, and concrete suggestions to streamline the babysitter workflow (fewer tool calls, less token churn, brief wording fixes). When a recap suggestion is validated in practice, promote it into this SKILL.md and note the promotion in the log.

Format (keep entries terse):

```
## 2026-07-21 — sprint <name/ids>
- 18:04 worker-1 briefed: stints 0501+0502, com (medium)
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

## Compatibility — pre-build fallback

If `plexi pane send --help` does not list `--submit`, you are on a pre-build; use these fallbacks only for that build. Keep the new verbs above as the normal path.

- **Submit:** `plexi pane send <id> "<text>"`, then `plexi pane key <id> enter` once. Do not use this sequence to confirm delivery or repeatedly press Enter.
- **Spawn agent:** `plexi pane new -n "<label>"`, then `plexi pane command <id> "<tier alias>" --enter`; wait for the booted prompt before sending the brief.
- **Wait for a slot:** poll `plexi pane slot read <name> <pane_id>` on the existing wake cadence; a terminal state needs an explicit `read`, not a host-side wait.
- **Capture raw output:** capture JSON and extract `.lines[]` after its envelope; do not treat an empty `--from-cursor` delta as proof that a command failed.
- **Determine pane status:** use the corroborated `pane list` agent state, untruncated status bar, and trailing buffer activity; never infer idle from one absent screen marker.
- **Updater stderr:** `sudo:` lines can be background-updater noise when the command exits 0; filter them with `2>&1 | grep -v "sudo:"` only when that noise obstructs the read, and never retry solely because of it.

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
