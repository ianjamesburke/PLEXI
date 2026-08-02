# Babysitter promotion ledger

Rules earn their place by firing. This file is the accounting: what got promoted into `SKILL.md`, why, and **the condition under which it gets deleted**. Without a deletion condition a rule is permanent by default, and the skill only accretes.

`LOG.md` records what happened in a run. This file records what we changed *because* of it, and what would prove that change wrong.

## How to use it

**At every run end** (same moment you write the sprint recap), do both halves:

1. **WRITE** — every recap suggestion you promote into `SKILL.md` gets an entry here in the same edit. A promotion with no ledger entry is not a promotion; revert it.
2. **READ** — walk the ledger and check each entry's falsification trigger against this run. Then act:
   - Trigger met → **delete the rule from `SKILL.md`**, set the entry to `RETIRED` with the date and what triggered it. Do not soften the rule instead of deleting it.
   - Rule fired → append the date to `fired:`. This is what earns its place.
   - Neither → increment `runs:` and move on.

A head that writes candidates but never runs the READ half has done half the job. The ledger is not an archive.

## Classes

- **RULE** — promoted into `SKILL.md`. Carries a falsification trigger.
- **HOST** — repeated instruction failed to enforce it, so it is not a wording problem. Becomes a Plexi feature request (a stint), and the ledger entry names it. **Promote to HOST on the second run where prose demonstrably failed** — do not spend a third run rewording.
- **CANDIDATE** — observed once, not yet promoted. Needs a second sighting or it expires.

## No A/B measurement — deliberate

We do not measure rules against control runs. Runs differ too much and n is too small; any "benefit" we computed would be noise, and cementing noise is worse than the accretion we are fixing. Falsification triggers are the whole mechanism: a rule stays because it demonstrably fired, and dies on a stated condition. (Ian + head ruling, 2026-08-01.)

## Entry format

```
### L### — <short title>
- class: RULE | HOST | CANDIDATE | RETIRED
- promoted: YYYY-MM-DD   (run/queue that caused it)
- incident: what actually happened, concretely
- rule: the instruction now in SKILL.md — or the stint id, for HOST
- falsification: the condition under which this gets DELETED
- fired: dates it demonstrably caught something
- runs: count of completed runs since promotion
```

---

# Ledger

### L001 — Cargo serialization cannot be enforced by instruction
- class: HOST
- promoted: 2026-08-01 (queue 0677 0678 0679 0674)
- incident: Six real token violations across three workers in one night, every one *after* an explicit written prohibition — including a brand-new pane whose brief led with the rule, its reason, and its predecessor's retirement as precedent. Worker Mode's gate instinct beat every head instruction. Two further apparent violations turned out to be a tester's legitimate `pr-install` (see L004), which is itself evidence the prose guard was unfalsifiable in practice.
- rule: **Not a wording fix.** A resource lock the host enforces, not a sentence workers are asked to honour. Filed as a Plexi feature request — stint **0690** (host-arbitrated cargo/build lock). Until it lands, do not run a multi-lane session that depends on voluntary compliance; run one build lane.
- falsification: delete if a host-side lock ships and a full multi-lane run completes with zero violations, or if two consecutive multi-lane runs pass with prose alone (which would prove the instruction sufficient after all).
- fired: 2026-08-01 (6x)
- runs: 1

### L002 — Retire workers at PR-open, not at merge
- class: RULE
- promoted: 2026-08-01 (queue 0677 0678 0679 0674)
- incident: An idle worker left alive after its PR opened has nothing to do, so any nudge sent it back to its build gate. Directly caused violations 1, 7 and 8. A fresh pane for a fix round cost seconds, and the worktree survived the pane close intact.
- rule: Close a worker pane once its PR is open and green. Spawn a fresh pane for a fix round. Supersedes "one batch per pane through merge".
- falsification: delete if a fix round ever costs materially more than a fresh spawn (lost worktree, lost context that a re-brief could not restore) twice in a row.
- fired: 2026-08-01
- runs: 1

### L003 — A send is not delivered until `pane status` says `working`
- class: RULE
- promoted: 2026-08-01 (queue 0677 0678 0679 0674)
- incident: Collapsed paste six times in one run. `pane send --submit` returns exit 0 while the text sits unsubmitted, and it happens to short single-line sends on freshly-booted panes too. Once it idled a held cargo token for ~8 minutes.
- rule: After any control message, confirm `pane status` reaches `working`; press enter once if it has not. Never treat exit 0 as proof of delivery, and never re-send on exit code alone.
- falsification: delete if a host fix makes `--submit` reliable and two consecutive runs show zero collapsed pastes.
- fired: 2026-08-01 (6x)
- runs: 1

### L004 — Attribute a build by its owner, never by its directory
- class: RULE
- promoted: 2026-08-01 (queue 0677 0678 0679 0674)
- incident: `just pr-install <N>` builds **inside the PR's own feature worktree**, so a tester's legitimate install is indistinguishable by path from a lane violation. The head killed a tester's install twice and retired a worker for violations that were the tester's build. The worker's pane read `idle` at the time — an idle pane cannot be compiling.
- rule: Confirm the OWNER of a cargo process before calling it a violation (`pgrep -fl` gives the full path; cross-check the pane's state). An idle pane is not compiling. A tester's token lane is the PR's feature worktree, not a `pr-<N>` string.
- falsification: delete if `pr-install` moves to a dedicated build tree, making path attribution unambiguous again.
- fired: 2026-08-01 (2x, as the error it now prevents)
- runs: 1

### L005 — A slot value alone is never ground truth
- class: RULE
- promoted: 2026-08-01 (queue 0677 0678 0679 0674)
- incident: Stale slots three times in one run. Workers write on transition but never invalidate on new work. One falsely read `blocked` while its PR was already open, idling the token with two lanes queued; two falsely read `done`.
- rule: Pair the slot with `verdict` and a ground-truth check (`gh pr list --head`) before acting on it. The step token tells you whether the value is current; when it is stale, verify externally.
- falsification: delete if slot invalidation becomes automatic host-side and two consecutive runs show no stale reads.
- fired: 2026-08-01 (3x)
- runs: 1

### L006 — The vacuous-gate question
- class: RULE
- promoted: 2026-08-01 (queue 0677 0678 0679 0674)
- incident: Asking "would this test still pass after the bug is fixed?" caught two audit tests that were green in CI and proved nothing. The replacements assert behaviour that changes post-fix. Highest-value check of the run.
- rule: For every new test a worker adds, ask whether it would still pass once the bug is fixed. If yes, it is vacuous — send it back.
- falsification: delete if it goes 10 runs without catching a vacuous test.
- fired: 2026-08-01 (3x)
- runs: 1

### L007 — Carry known pre-existing defects forward into tester briefs
- class: RULE
- promoted: 2026-08-01 (queue 0677 0678 0679 0674)
- incident: Feeding an audit's result forward as a KNOWN PRE-EXISTING DEFECT clause stopped two later testers from failing their PRs over a host bug neither caused nor was allowed to fix. Without it, two false fix rounds.
- rule: When a run establishes a pre-existing defect, every subsequent tester brief names it and excludes it from that PR's gate. Pairs with the worker-side decision ladder: a Done-When criterion failing on a pre-existing defect outside the diff is the worker's own call.
- falsification: delete if the decision ladder alone prevents the false round twice, making the brief clause redundant.
- fired: 2026-08-01 (2x)
- runs: 1

### L008 — Ending a turn bare freezes the run
- class: RULE
- promoted: 2026-08-01 (stint 0674 freeze, 02:01)
- incident: A worker wrote `impl:needs-input` and went idle; the head relayed the question upward and went idle at 02:08. The only watchdog lived inside the `me` session and died when that session restarted at 02:53. The run was dead for hours.
- rule: `needs-input` abolished; slot grammar is `working | done | blocked | failed`. Every turn ends with the status slot matching reality. Backed by the decision ladder (PR #2548) and, outside the repo, a launchd freeze watchdog that nudges an idle head with no terminal status.
- falsification: delete if two consecutive runs end with zero bare turns AND the watchdog logs no nudges — at which point the behaviour is habitual and the wording is dead weight. Do **not** delete merely because no freeze occurred; the watchdog firing is what proves the rule is still load-bearing.
- fired: 2026-08-01
- runs: 1

### L009 — One head instruction cannot serialize two consumer classes
- class: CANDIDATE
- promoted: 2026-08-01 (queue 0677 0678 0679 0674)
- incident: An install-tester and a worker are both cargo consumers, but `RUN_CONFIG`'s `max_concurrent_cargo_builds` says nothing about testers. Tonight's token counted them only because the head wrote it that way; a naive reading would let a tester install run beside a worker gate.
- rule: (not yet promoted) Make the cargo budget explicitly cover testers as well as workers.
- falsification: expires if L001's host lock lands and makes the config key moot, or if it is not seen again within 3 runs.
- fired: —
- runs: 1

### L010 — A dirty alpha blocks the run at spawn, not at merge
- class: RULE
- promoted: 2026-08-01 (stint 0630)
- incident: The head's own uncommitted rewrite of SKILL.md and RUN_CONFIG.toml sat in the alpha root. `just bs-start 0630` correctly refused to branch a worktree from a dirty tree, so worker-1 blocked before it could claim — the very first action of the run. SKILL.md warned about this hazard only at the merge boundary, where `merge-pr`'s sync would destroy the edits; nobody had noticed it also fails closed at the start.
- rule: The head verifies alpha is clean before spawning worker-1, and gets any of its own skill edits committed as `chore(babysitter):` at run start rather than at merge time. Commit, never restore.
- falsification: delete if `bs-start` grows a dirty-tree bypass, or if two consecutive runs start clean without the check being what made them clean.
- fired: 2026-08-01
- runs: 0

### L011 — Name the un-drivable surface in the brief and pre-authorize the fallback
- class: RULE
- promoted: 2026-08-01 (stint 0630)
- incident: 0630's surface was a HELD modifier (Ctrl) with no pane primitive behind it — not injectable through the CLI. The brief said so up front, told the tester not to grind on it, and pre-authorized falsifying the PR's own new guard instead. The tester hit exactly that wall, took the fallback immediately, and returned PASS in 9m50s on the first attempt. Contrast the previous run, where a tester discovered an un-drivable surface on its own and lost most of a session to it.
- rule: Before writing a tester brief, ask what about this surface may not be drivable (held modifier, hover, drag, real LLM turn). Name it, forbid grinding, and give the substitute that still proves the change — usually falsifying the new guard in a scratch copy.
- falsification: delete if two consecutive runs show testers correctly self-selecting the fallback without being told, making the clause redundant.
- fired: 2026-08-01
- runs: 0

### L012 — A watch that matches a bare `done` fires on the wrong step
- class: RULE
- promoted: 2026-08-01 (stint 0630)
- incident: The head's watch loop broke on any status value ending in `done`. The worker wrote `start:done` on finishing worktree setup; the loop fired and reported a terminal state before implementation had begun. Cost one wasted cycle and a re-armed watch.
- rule: `status` is `<step>:<state>`. Wait on the specific step, or on the `pr` slot becoming non-empty — never on a bare `done` suffix. Pair with the `pane status` verdict; a slot alone is never liveness (see L005).
- falsification: delete if the slot grammar gains a single unambiguous terminal value that cannot collide with an intermediate step.
- fired: 2026-08-01
- runs: 0

### L013 — A wake condition gated on high confidence never fires
- class: RULE
- promoted: 2026-08-01 (queue 0701 0702 0703 0591)
- incident: The head's watch loop broke only on `verdict == idle && confidence == high`. Pane 712 at
  rest reports `idle` / `low` / `unknown` — the normal resting shape of a Codex pane. The condition
  was unsatisfiable, so the worker answered a head review at ~17:27 and the head would not have
  learned until its ~17:53 heartbeat cap. Ian walked in on the dead air and asked why everything had
  stopped. Nothing was broken; the head simply could not see that anything had happened.
- rule: Break on ANY idle, then disambiguate finished-vs-wedged by reading the buffer. Never let one
  field be both the trigger and the interpretation. In SKILL.md step 2.
- falsification: delete if `pane status` gains a trustworthy terminal signal for Codex panes (see
  L014), at which point the confidence field stops being load-bearing at all.
- fired: 2026-08-01
- runs: 0

### L014 — The head has no trustworthy liveness primitive for a Codex pane
- class: HOST
- promoted: 2026-08-01 (queue 0701 0702 0703 0591)
- incident: `idle/low/unknown` is what "finished and waiting" looks like AND what "wedged" looks like.
  Codex panes have no scheduler of their own (SKILL.md "Head liveness"), so an unattended run's entire
  clock is the head's polling loop. One wrong comparison in that loop stalls the run silently, with
  every individual component behaving correctly. That is not a wording problem.
- rule: Filed as a Plexi feature request — a pane liveness primitive the head can trust, same shape as
  stint 0628 (host-owned job supervisor). Until it lands, every head watch loop is single-point-of-
  failure infrastructure and gets reviewed as such.
- falsification: delete when a host-side terminal/liveness signal ships and two consecutive unattended
  runs complete with no head-side stall.
- fired: 2026-08-01
- runs: 0

### L015 — Watch the pane and the PR in the same loop
- class: RULE
- promoted: 2026-08-01 (queue 0701 0702 0703 0591)
- incident: #2555's `test` job FAILED (exit 127, the new roadmap-evidence step) while the head's watch
  polled only pane slots. The red sat unobserved for ~10 minutes because the pane was legitimately
  idle the whole time. A lane has two failure surfaces; the head was instrumented on one.
- rule: Once a `pr` slot exists, every wait iteration also checks `gh pr checks <PR#>`, and a red check
  wakes the head exactly like a terminal slot does. In SKILL.md step 2.
- falsification: delete if CI red is ever surfaced to the head by another channel that two consecutive
  runs prove sufficient.
- fired: 2026-08-01
- runs: 0

### L016 — Enumeration-first: a task's "measure the surface" step is a gated deliverable
- class: RULE
- promoted: 2026-08-01 (queue 0701 0702 0703 0591)
- incident: 0591 step 1 ("investigate first — measure, don't assume", and confirm rather than trust the
  task's own notes) and 0703 Done-When #1 (enumerate stable audio/MIDI/video/decoder entry points) were
  both passed through as prose inside a four-stint batch brief. The worker implemented against an
  assumed surface, gated four app-open paths, and opened the PR. Only when the head asked for the
  enumeration did the audit happen — 58m57s — and it invalidated part of the implementation: raw .wasm
  path-open derives the app id from the FILE STEM and bypasses the manifest-id DAW gate entirely, and
  the protocol-level media ops (audio capture/playback/device listing, MIDI listing/input/output, video
  decoder open/state/close) are capability-gated, not release-tier-gated. Both Done-Whens unmet, an hour
  of rework, bought at full price. The task authors had anticipated this exact failure in the task body.
- rule: When a stint's own first step is an investigation, require the enumeration posted BEFORE any
  implementation code, as its own blocked-for-review checkpoint, and rule on it before work starts.
  Result goes in the PR body verbatim. In SKILL.md "Batch into as few PRs as possible".
- falsification: delete if two consecutive runs show workers self-selecting enumeration-before-
  implementation without the clause, making it redundant.
- fired: 2026-08-01
- runs: 0

### L002 — RECONCILED 2026-08-01
Promoted into SKILL.md step 6 (retire at PR-open) on 2026-08-01, two runs after it entered this ledger.
It sat here unreconciled while three consecutive heads read the note and did not act, which is the
standing proof that this ledger is written at run end and read by nobody at run start. A rule that
lives only here is inert. The ledger is the receipt for a rule; SKILL.md is the rule.

### L015a — amendment, same day
The first implementation of L015 fired on the very first iteration, every iteration: the red that
CAUSED the fix round is still on the wire for the whole round, so "wake on any red" is
indistinguishable from "wake immediately." A watch that always fires carries exactly as much
information as one that never fires (L013). Corrected: record the head sha when arming the watch and
ignore CI until the sha changes. Generalisation worth keeping — a wake condition must be defined
against state the head has NOT yet ruled on, never against raw current state.

### L017 — Never offer a worker a choice where you owed it a diagnosis
- class: RULE
- promoted: 2026-08-01 (queue 0701 0702 0703 0591)
- incident: CI failed with `just: command not found`. The head's fix-round brief said "call the
  scripts directly, OR install just if you can justify why that is the better long-term shape."
  The worker took the shallower option, replaced the workflow's two `just` calls with direct
  `python3` invocations, and pushed. But `scripts/roadmap-evidence.py` ITSELF shells out to
  `just scene ...`, so the outer call was patched and the inner one was left. CI failed again
  16m24s later on the identical root cause, one layer down. Cost: a full CI round trip plus a
  worker spawn. The brief's phrasing invited a surface patch by making "shape" a matter of taste
  rather than requiring the dependency be enumerated first.
- rule: When the head already knows the fix, rule — do not present options. When the head does NOT
  know, require the diagnosis as the deliverable before any edit, and rule on it. An either/or in a
  brief is an instruction to pick the cheaper branch. Pairs with L016: enumerate before asserting a
  dependency is satisfied, at every layer, not just the one the error message named.
- falsification: delete if two consecutive runs show a worker escalating an under-specified either/or
  back to the head instead of silently picking, making the clause redundant.
- fired: 2026-08-01
- runs: 0

### L018 — A CI step that exits 0 is not evidence it did anything
- class: RULE
- promoted: 2026-08-01 (queue 0701 0702 0703 0591)
- incident: Third consecutive `test` failure on PR #2555, all exit 127, all `just: command not found`.
  Round 3 added `- name: Install just / uses: taiki-e/install-action@v2.85.6` in the right job, in the
  right position, with a verified-existing version tag — and supplied **no `with:` block**. That
  action's `action.yml` declares `tool` as `required: true`, but GitHub does not enforce `required`
  for composite-action inputs: the step installed nothing, exited 0, and rendered green in the log.
  The head only caught it by fetching the action's own `action.yml` at the pinned tag. Cost: a third
  CI round trip on an already head-reviewed diff.
- rule: Verifying an action's version tag exists is necessary and not sufficient — also read its
  `action.yml` for required inputs, because `required: true` is advisory for composite actions and a
  missing input is a silent no-op, not an error. Generally: when a step is added to make a later step
  work, the proof is the later step passing, never the added step's own exit code.
- falsification: delete if GitHub starts enforcing `required` inputs for composite actions, at which
  point the silent-no-op class disappears.
- fired: 2026-08-01
- runs: 0

### L019 — A worker can silently escalate its own model tier
- class: RULE
- promoted: 2026-08-01 (queue 0701 0702 0703 0591)
- incident: worker-3 (pane 723) was launched `com` = `gpt-5.6-terra` (medium, per RUN_CONFIG
  `worker_tier`). Midway through the CI fix round its footer read `gpt-5.6-sol high` — `col`, the
  LARGE tier. Nobody authorized it; the head only noticed because the model string happened to be
  visible in a routine buffer tail. RUN_CONFIG's tier is set at launch and the head never re-checks
  it, so a self-escalation is invisible and unbounded for the life of the pane. Ian's standing rule
  is that large requires his say-so or a hard-reject after the configured tier has failed. A
  `/model` sent to switch back was accepted as a command but the footer did not change; the head
  declined to grind on the TUI mid-run.
- rule: Read the tier off the pane footer whenever you capture it, and treat a tier that does not
  match RUN_CONFIG as an incident: state it, and do not let it carry into further substantive work.
  Launch tier is an assumption with a shelf life, not a fact.
- falsification: delete if the host makes a pane's tier immutable after launch, or exposes it as a
  slot the head can assert on — at which point this becomes a HOST fix rather than a watch habit.
- fired: 2026-08-01
- runs: 0

### L020 — "Nothing pending" reads identically to "nothing started"
- class: RULE
- promoted: 2026-08-01 (queue 0701 0702 0703 0591)
- incident: The head's CI watch broke when no check was `pending`. Immediately after a new push, the
  run's jobs are not registered yet: `gh pr checks` returns only CodeRabbit (always instantly
  `pass`), nothing is pending, and the watch declared CI terminal 180s in while `clippy` and `test`
  had not started. Third variant of the same bug in one run — a condition that cannot distinguish
  two opposite states (L013: idle-vs-wedged; L015a: red-I-ruled-on vs red-that-is-new).
- rule: Require the real jobs to be PRESENT before trusting terminality — count non-CodeRabbit rows
  and demand at least one. Generally: before shipping any wake condition, name the state it would
  ALSO match, and if that state is the opposite of what you want, the condition is wrong.
- falsification: delete if `gh pr checks` gains an explicit "checks not yet created" state that the
  head can test directly.
- fired: 2026-08-01
- runs: 0

### L021 — A gate that has never run green is not a gate
- class: RULE
- promoted: 2026-08-01 (queue 0701 0702 0703 0591)
- incident: Ian moved the roadmap evidence gate off the PR path to `schedule:` + `workflow_dispatch`
  (sound: it re-ran the workspace suite plus every scene serially, ~25 min on every PR to answer a
  release question). But GitHub only runs `schedule:` and `workflow_dispatch` from the DEFAULT
  branch's copy of a workflow file, and `roadmap-evidence.yml` exists only on the PR branch. So the
  gate cannot execute until the PR merges — meaning 0701 would ship a release-authority gate whose
  first-ever execution is an unattended 09:00 UTC nightly, after every prior attempt had died on a
  missing `just` or a scene timeout.
- rule: When a PR introduces a gate that does not run on the PR path, its first successful execution
  is part of that PR's Done-When, not a later surprise. Merge, then immediately `gh workflow run` it
  on the default branch and watch it to a verdict attended. Never let the first run be the nightly.
- falsification: delete if a gate is ever introduced off-PR-path and its first scheduled run passes
  clean twice in a row without an attended dispatch, proving the attended step redundant.
- fired: 2026-08-01
- runs: 0

### L022 — A new check can appear on a sha you already ruled green
- class: RULE
- promoted: 2026-08-01 (queue 0701 0702 0703 0591)
- incident: The head ruled `8d4eb0d9` green (`clippy` pass, `test` pass) and armed its watch against
  that sha, so CI was excluded from waking it. Minutes later a separate workflow run added an
  `update-docs` check to the SAME sha, and it failed. The sha-based guard (L015a) that fixed the
  fire-instantly bug had introduced a blind spot in the other direction: green is a verdict on the
  checks that existed when you looked, not on the sha.
- rule: "CI green" is never final while a PR is open. Re-read `gh pr checks` immediately before
  merging, not only when the watch wakes; a check set can grow after a green reading.
- falsification: delete if all repo workflows are consolidated so a PR's check set is fixed at push
  time.
- fired: 2026-08-01
- runs: 0

### L023 — Rule a red out of the gate only after proving it is not yours
- class: RULE
- promoted: 2026-08-01 (queue 0701 0702 0703 0591)
- incident: `update-docs` failed with `[DocDr] Error [503] ... Under Construction` — plainly an
  external service, and the tempting call was to wave it through as "not our bug" and merge. Instead
  the head timestamped the workflow's recent runs: the same job SUCCEEDED on another branch at
  23:40:07Z, 108 seconds AFTER this failure at 23:38:20Z. So it was a transient window, not an
  outage — which means a re-run settles it definitively for ~30 seconds of wall clock, and merging
  past a red on a plausible story was never necessary.
- rule: An external-looking red still gets one cheap disproof before it is excluded from the gate:
  check whether the same job passed elsewhere, and when. If it passed after your failure, re-run it
  rather than reasoning about it. Only exclude a red you have evidence is unfixable by retry.
- falsification: delete if two consecutive runs show a re-run of an external-service red failing
  again, proving the retry step wasteful.
- fired: 2026-08-01
- runs: 0

### L024 — `cargo test --exact` with a bare name runs nothing and exits 0
- class: RULE
- promoted: 2026-08-01 (queue 0701 0702 0703 0591)
- incident: tester-1 ran `cargo test --bin plexi stable_and_rc_channels_disable_v1_stubbed_surfaces
  -- --exact` and got `test result: ok. 0 passed; 0 failed; 0 ignored; 2178 filtered out` — exit 0,
  the word "ok" in the output, and not one test executed. `--exact` matches the FULL module path
  (`release::tests::<name>`), so a bare function name matches nothing. Had the tester read that as
  its baseline, the whole falsification round would have been vacuous: revert the guard, "test still
  ok", conclude the guard holds. Fourth instance tonight of a green signal that cannot distinguish
  "worked" from "did nothing" (L013, L018, L020).
- rule: Any test result with `0 passed` is INCONCLUSIVE, never a pass — check the passed count, not
  the exit code. Falsification requires BOTH halves observed: the test green on the unmodified tree,
  then red with the guard reverted. Brief testers with the full module path.
- falsification: delete if the toolchain starts failing a filter that matches zero tests (cargo has
  `--no-tests=fail` behind an unstable flag; delete when it is stable and adopted).
- fired: 2026-08-01
- runs: 0

### L025 — "Agent idle" is not "work delivered"; read the remote, never the report
- class: RULE
- promoted: 2026-08-01 (merge-queue drain, PRs 2522/2532/2536/2555/2556/2557)
- incident: Five separate agents in one session finished real work and then went idle without
  pushing or reporting. Every time, the artifact existed and only the delivery was missing: 0654
  had opened its PR and never said so; 0654's assertion fix sat as a local commit twice; the 2536
  rebase was complete and clean while the head believed it was mid-conflict; the 2532 rebase had
  been done for twenty minutes with `pgrep cargo|rustc` returning zero. One auditor produced
  nothing across four idle notifications and had to be replaced. The head recovered every case by
  reading the worktree and `git ls-remote` directly, never by waiting.
- rule: Treat an idle notification as "check the remote," not as a status. Ground truth is
  `git ls-remote origin <branch>` against the worktree HEAD, plus `gh pr view --json
  mergeable,mergeStateStatus,updatedAt` — a PR whose `updatedAt` predates the agent's last claim
  means nothing was pushed. When a deliverable is a report rather than a commit, have the agent
  write it to a FILE incrementally and message only "file complete"; a message that is never sent
  loses everything, a partially written file loses nothing.
- falsification: delete if two consecutive runs show every agent reporting completion within a
  minute of its last artifact, making the remote check redundant.
- fired: 2026-08-01
- runs: 0

### L026 — A locator that silently widens on a miss is a false-failure generator
- class: RULE
- promoted: 2026-08-01 (stint 0654, PR #2557)
- incident: A regression test scoped its assertion to output after the command echo via
  `.rposition(|l| l.trim() == "› cm").map_or(observed.as_slice(), |i| &observed[i+1..])`. `"› cm"`
  is the LOCAL harness prompt; CI's shell renders `host:dir runner$ cm`. The anchor never matched
  on CI, `map_or` fell back to the entire scrollback — including the test's own setup lines, which
  legitimately name the completion candidates being asserted absent — and the test failed three CI
  rounds while the product fix was correct the whole time. It passed 5/5 locally each round. Same
  shape as L018's `install-action` with no `tool:` input: a required thing missing degrades to a
  permissive default instead of an error.
- rule: A search that fails to find its anchor must panic, never fall back to a wider scope. Never
  anchor a terminal assertion on a rendered prompt — match by suffix or by an emitted token that is
  environment-independent. Generally: in tests and CI alike, "could not locate X" is a failure of
  the harness and must say so; degrading to "check everything" or "do nothing" produces a signal
  that cannot distinguish broken from working (L013, L018, L020, L024).
- falsification: delete if a linting pass starts rejecting `map_or`/`unwrap_or` fallbacks on
  locator results across the test suite, making the rule mechanically enforced.
- fired: 2026-08-01
- runs: 0

### L021a — L021 fired the same night it was written
- class: RULE (evidence for L021)
- promoted: 2026-08-01 (queue 0701 0702 0703 0591)
- incident: PR #2555 merged as `cf444208` with the new roadmap-evidence gate never once executed —
  it cannot run on the PR path by design, and `schedule:`/`workflow_dispatch` only run from the
  default branch. The head dispatched it on alpha immediately after merge (run 30724105791): it
  FAILED, 16 tests, all `wait_app_frame_failed: timed out after 60s waiting for first app frame`.
  Root cause: `roadmap-evidence.yml` omits the `WASI_BUNDLE_MODE`/`PLEXI_CPYTHON_BUNDLE_DIR` env and
  both CPython WASI bundle steps that `rust-host.yml`'s `test` job has, so no Python app pane can
  render a frame. Without the attended dispatch this would have surfaced as a silent 09:00 UTC
  nightly failure with nobody reading it, on the gate that IS the stable-v1 release authority.
- rule: (L021 stands as written) — a gate's first successful execution belongs to the PR that
  introduces it. Corollary learned here: when a new workflow runs a suite an existing workflow
  already runs, diff the two jobs' SETUP, not just their commands. The tests were identical; the
  environment was not.
- falsification: same as L021.
- fired: 2026-08-01 (immediately)
- runs: 0
