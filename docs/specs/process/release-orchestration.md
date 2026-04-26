# Release Orchestration

How v3.x releases are planned, decomposed, dispatched to sub-agents, verified, and shipped. This document is the durable spec; `.claude/iteration-cycle.md` is the operational checklist Claude follows in-session.

## Cadence

- One release every ~2.5 weeks
- Five releases over a quarter (12 weeks): v3.1 → v3.5
- Each release has a single theme — never a grab bag

If a release would mix themes, split it. Theming forces every issue in the release to share a smoke-test invariant: "did this theme actually land?"

## Release theme rules

1. **One foundation, then capabilities on top.** v3.1 is rendering substrate. v3.2 is workflow built on that substrate. Never invert the order.
2. **No partial features across releases.** A capability ships fully in one release or it gets descoped from that release. Half-shipped features rot.
3. **Every release has one P1 headline issue.** That issue defines the release. The rest are supporting.
4. **The smoke test must observe the theme.** If the release is "media plumbing," the smoke test must capture audio, send MIDI, decode video — not just compile clean.

## Branch flow — alpha train, batched beta promotion

The user lives on `alpha` through the entire v3.1 → v3.5 cycle. PRs land on `alpha` continuously, the user tests each PR on the alpha build as it lands. `beta` and `main` are batched promotion targets, not per-release.

```
v3.1 PR1 → alpha → user verifies on alpha build
v3.1 PR2 → alpha → user verifies on alpha build
...
v3.5 PRN → alpha → user verifies on alpha build
                     ↓
            (user signals: alpha is at v3.5 and stable)
                     ↓
              alpha → beta (batch promotion)
                     ↓
              beta soaks 24–72h
                     ↓
              beta → main, tag v3.5
```

**Why this works:**
- Solo-dev QA is the user — no parallel beta channel needed mid-cycle.
- Continuous verification on alpha catches regressions immediately, not at release time.
- Beta + main are only updated when there's a coherent batch worth promoting.
- The user can choose to cut a beta partway (e.g. after v3.2) if they want a "stable enough" snapshot — but it's a deliberate decision, not a per-release ritual.

**Hard rules:**
- Sub-agent worktrees branch from `alpha`, PR back to `alpha`. Never to `beta` or `main`.
- `beta` only moves when the user explicitly says "promote alpha."
- `main` only moves when `beta` has soaked successfully.
- Hotfixes for `main` are the only exception — those branch from `main`, PR to `main`, and cherry-pick back to `alpha`.

## Roles

**Orchestrator (main session, human + Claude):**
- Owns release planning, decomposition, and PR review.
- Reviews PRs from sub-agents — diff, not summary.
- Pulls each PR's worktree, runs smoke test locally before merging.
- Writes the **Human verification** checklist into every PR description.
- Writes `DEV_LOG.md` entries with `Breaks if:` lines.
- Initiates batched beta and main promotions when the user says go.

**Sub-agents (background, one per issue):**
- Run with `isolation: "worktree"`, branched off `alpha`, `run_in_background: true`.
- Write the failing test first. Implementation is "make it pass."
- Run `cargo build` + `just install-alpha` + `scripts/smoke-test.sh` before opening PR.
- Open PR to `alpha` with `Breaks if:` line and **Human verification** checklist populated in the description.
- Never merge their own PR.

**Human (the user):**
- Runs the **Human verification** checklist on the alpha build after each PR merges.
- Reports back: green = next PR; red = revert + new sub-agent dispatch.
- Decides when alpha is ready to promote to beta.

## Per-release workflow

### 1. Decompose
For each issue in the release, the orchestrator writes a one-paragraph implementation brief covering:
- Target file paths (concrete, not "somewhere in the renderer")
- The test that defines done
- The SDK surface change, if any
- The smoke-test invariant the work must preserve
- The **Human verification** steps (what the user will click/type/observe to confirm it works)
- Anything from `DEV_LOG.md` the agent should read before starting

### 2. Group by file overlap
Issues that touch the same file get serialized. Everything else runs concurrent. v3.1's geometry refactor is mostly serial; v3.4's audio/MIDI/video is mostly parallel.

### 3. Dispatch
Spawn one sub-agent per issue (or per serial group) via `Agent` with `isolation: "worktree"` and `run_in_background: true`. Brief is identical in shape across all agents:

> "Read the issue. Read the linked spec. Write the failing test first. Implement until green. Run `cargo build` and `just install-alpha` and `scripts/smoke-test.sh`. Open PR to `alpha` with `Breaks if:` line and the **Human verification** checklist provided. Do not merge."

### 4. Review loop
When an agent reports done:
- Orchestrator reviews the diff (not the summary).
- Pulls the worktree, runs smoke test locally.
- If clean: merge to `alpha`.
- If stub (`todo!()`, missing test, smoke-test failure): reject with tightened brief, re-dispatch.
- After 3 rejections on the same issue, orchestrator takes it directly.

### 5. Human verification on alpha
After every merge to alpha:
1. User pulls latest alpha and runs `just install-alpha`.
2. User runs the **Human verification** checklist from the PR description.
3. Reports back: ✅ → next PR can dispatch; ❌ → revert merge, file regression issue, re-dispatch with diagnosis.

This step is non-optional. A merged PR that hasn't been human-verified blocks the next dispatch in the same theme. Other themes' PRs can continue in parallel.

### 6. Release-level checklist
When all PRs in a milestone are merged + human-verified, run the release-level checklist in `docs/specs/releases/v3.x.md`. This is the rolled-up smoke test for the theme — proves the release as a whole works, not just the individual PRs. Examples:
- v3.4: "Plug in audio interface + MIDI controller + Take 5 + a video file. Capture, route, play, decode all in one Plexi session."
- v3.5: "Run `gh --plexi`, `cargo --plexi`, and an unregistered CLI in three panes simultaneously. All three resolve correctly through their respective tiers."

### 7. Promotion (batched, user-initiated)
When the user says "promote alpha":
1. `just install-alpha` from a clean checkout — no local mods.
2. Run all release-level checklists for every milestone alpha contains since last beta.
3. Promote `alpha` → `beta`. Tag the beta build with the milestone(s) it contains.
4. Beta soaks 24–72h with normal use.
5. Promote `beta` → `main`. Tag `v3.x`.
6. Write `DEV_LOG.md` entry with one `Breaks if:` line per shipped feature, grouped by milestone.

If any step fails, the promotion is aborted. Fix lands on alpha first, then the promotion is re-attempted.

## PR description template

Every PR opened by a sub-agent must use this shape:

```markdown
Closes #<issue-num>

## What changed
<one paragraph>

## Breaks if
<concrete observable symptom — visible UI failure, missing log line, or specific broken behavior>

## Human verification
- [ ] Step 1: <action> — expect <result>
- [ ] Step 2: <action> — expect <result>
- [ ] Step 3: <action> — expect <result>

## Test added
- `<file::test_name>` — <one-line description>
```

## Sub-agent brief template

```
Issue: #<num> — <title>
Branch: feature/<num>-<slug>
Base: alpha

Read first:
- The issue body and all comments
- DEV_LOG.md (first 100 lines)
- <linked spec docs>

Target files:
- <path>:<reason>

Define done:
<the test that must pass>

SDK surface change:
<yes/no, and what>

Smoke-test invariant:
<what must still work after this lands>

Human verification (write into PR description):
- <step 1: action → expected result>
- <step 2: action → expected result>
- <step 3: action → expected result>

Process:
1. wtp add -b feature/<num>-<slug> from alpha
2. Write the failing test
3. Implement until green
4. cargo build && just install-alpha && scripts/smoke-test.sh
5. Open PR to alpha with `Breaks if:` line and Human verification checklist
6. Do not merge
```

## Failure modes and rules

- **Stub detection:** any `todo!()` or `unimplemented!()` outside `#[cfg(test)]` is an automatic rejection. Enforced at compile time by `#![deny(clippy::todo, clippy::unimplemented)]`.
- **Missing test:** PRs without a test that fails before the implementation are rejected. The test is the spec.
- **Missing human verification:** PRs without a populated **Human verification** checklist are rejected. The user can't be expected to invent verification steps.
- **Cross-issue collision:** if two sub-agents touch the same file in incompatible ways, the orchestrator serializes them — second one rebases and re-dispatches.
- **Three-strike rule:** after 3 rejections on the same issue, the orchestrator implements it directly. Repeated failure means the brief was wrong, not the agent.
- **No backwards-compat shims** during v3.x scope. If a refactor breaks existing apps, the apps get upgraded in the same PR, not a compat layer.
- **Human-verify blocking:** a merged but unverified PR blocks subsequent dispatches in the same theme. Other themes proceed.

## Release index

Per-release specs live at `docs/specs/releases/v3.x.md` — each contains the milestone's full human-verification checklist (release-level, not per-PR). Stubs created at planning time, fleshed out as PRs land.

Current roadmap:
- [v3.1 — Foundation](../releases/v3.1.md)
- [v3.2 — Workspace + Secrets](../releases/v3.2.md)
- [v3.3 — Agents](../releases/v3.3.md)
- [v3.4 — Media](../releases/v3.4.md)
- [v3.5 — `--plexi` Flag + Registry](../releases/v3.5.md)
