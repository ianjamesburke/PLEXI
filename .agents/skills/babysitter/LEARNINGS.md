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
