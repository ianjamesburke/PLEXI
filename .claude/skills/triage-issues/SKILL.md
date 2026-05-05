---
name: triage-issues
description: "PLEXI issue triage — turn a raw GitHub issue into an actionable, prioritized, slotted ticket with file-touch tracking and clarification questions. Use when the user says 'triage this', 'label this issue', or wants to slot an issue into the right priority/version."
risk: low
source: local
---

# PLEXI Issue Triage

Turn a raw, unlabeled GitHub issue into an actionable, prioritized, slotted ticket.

---

## North Star

**Plexi is an all-in-one Linux-style productive environment for macOS — terminal-native, keyboard-driven, with a plugin infrastructure that lets you build anything inside it.**

The core value proposition: one compositor that makes your terminal tools, agents, and custom apps feel like a single coherent system. Panes compose freely. Apps speak one protocol (PGAP). The host brokers capabilities (AI, audio, secrets, net) so apps stay sandboxed. Everything is keyboard-accessible.

**Use this to audit every issue for North Star alignment:**

| Aligns | Diverges |
|--------|----------|
| Terminal ergonomics, pane management | Anything requiring a browser/Electron runtime |
| App/agent plugin infrastructure | Standalone GUI features unrelated to pane composition |
| Capability brokering (AI, audio, secrets, net, MIDI) | Features that only make sense outside a terminal context |
| SDK primitives that unlock whole categories of apps | One-off integrations with no generalization |
| Composability: pipes, linked terminals, context nesting | Features that centralize control away from the user |

If an issue is off-brand, note it in the triage comment and suggest closing or converting to an idea-only label with `future` era.

---

## Step 1 — Load Live Project State

Before touching the issue, fetch current labels and milestones. Never use static version lists — they go stale.

```bash
gh label list --limit 100
gh api repos/ianjamesburke/PLEXI/milestones --jq '.[] | {number:.number, title:.title, state:.state}'
```

From the label list, identify:
- **Era labels** — any label matching `v*` (e.g. `v3.5+`, `v4.0+`, `future`). These are your valid era options.
- **Priority labels** — `P1`, `P2`, `P3`, `P4`
- **Type labels** — `bug`, `enhancement`, `idea`
- **Status labels** — `ready`, `in progress`, `blocked`

From the milestones, identify open ones — these are valid slots. Note the lowest-numbered open milestone as the current sprint.

---

## Step 2 — Fetch the Issue

```bash
gh issue view <number> --json number,title,body,labels,milestone,state
```

If no number was provided, ask: "Which issue number?"

Summarize the issue in 1–2 sentences. Then note whether the issue author already included a LOC estimate or action plan — you will cross-check these independently in Step 3.

---

## Step 3 — LOC Estimation + File Touch List

The goal is a *realistic* estimate grounded in the actual codebase, not a hand-wave.

**Process:**

1. **Identify affected subsystems** from the issue body: host (Rust in `src/`), Python SDK (`sdk/python/`), PGAP protocol (`src/app_protocol.rs` or similar), UI widgets (`src/widgets.rs`, `src/style.rs`), apps (`~/.plexi-alpha/apps/` or `examples/`), CI/config.

2. **Grep for relevant symbols in each subsystem:**
   ```bash
   grep -r "SymbolName\|keyword" src/ --include="*.rs" -l
   grep -r "keyword" sdk/python/ --include="*.py" -l
   ```
   List every file that would need to change.

3. **Estimate delta per file/area:**

   | Change type | Rough range |
   |-------------|-------------|
   | Bug fix in one function | 5–30 LOC |
   | New method on existing type | 20–80 LOC |
   | New `HostCommand`/`HostEffect` pair (enum + handler + SDK binding) | 150–300 LOC |
   | New module (< 1 file) | 100–400 LOC |
   | New system (harness, protocol layer, SDK extension) | 500–2000 LOC |
   | Cross-cutting architectural change | 2000–5000+ LOC |

   **PLEXI multipliers to apply:**
   - PGAP changes always touch *both* host (Rust) and SDK (Python) — double your estimate
   - UI-only (widgets, layout) — no multiplier, typically stays S or M
   - App-level only — no host changes, typically XS or S

4. **Sum the areas.** Assign a size bucket:

   | Size | LOC Range |
   |------|-----------|
   | XS | < 50 |
   | S | 50–200 |
   | M | 200–1000 |
   | L | 1000–5000 |
   | XL | > 5000 |

5. **Estimate confidence:** High (you found the exact files), Medium (you found the relevant module but not every callsite), Low (the issue is vague or the affected area is unclear).

6. **If the issue already contains a LOC estimate:** state where yours agrees or differs and why. Authors tend to underestimate changes to existing code and overestimate new-code areas.

7. **Record the touch list** — the top-level files and directories identified in step 2. This becomes the `touches` field in front matter and is used by `/sprint-plan` to compute parallelization groups. Use the shortest unambiguous path (e.g. `src/app/mod.rs`, `sdk/python/`, `src/widgets.rs`). Aim for ≤ 5 entries; consolidate to a parent dir when 3+ files in the same module are affected.

Note: `size:*` labels don't exist in the repo yet — include size in the triage comment but don't apply it as a label.

---

## Step 4 — Priority

Score P0–P4:

| Priority | Criteria |
|----------|----------|
| P0 | On fire — drop everything; the app is broken or data is at risk right now |
| P1 | Breaks a shipped feature for real users; blocks the current milestone from shipping |
| P2 | High-value — users actively need this; should happen in the near term |
| P3 | Clear value but not blocking anything; nice-to-have |
| P4 | Backlog / speculative — "someday maybe" |

Ask: is this a regression? Is it blocking a release? Is there active user demand, or is this an improvement someone thought of?

---

## Step 5 — Type Label

Pick exactly one from the live label list (fetched in Step 1):

- `bug` — something is broken or behaves contrary to documented intent
- `enhancement` — new feature or improvement to existing behaviour
- `idea` — speculative; needs validation or design before committing to build

---

## Step 6 — Version Era

Use only the era labels you found in Step 1. Do not invent or assume labels.

Decision logic:
- **Current sprint era** (e.g. `v3.5+` if that's the active label): post-v3.0 work that belongs in the near-term v3.x roadmap but isn't critical path
- **Next major era** (e.g. `v4.0+`): work that **changes the protocol contract**, requires breaking SDK changes, or involves architectural redesign. Open design questions alone are *not* a reason to bump to the next major era.
- **`future`**: speculative ideas with no clear "done" state — needs design thinking before it can even be scoped

When in doubt between the current era and future: if you can write an action plan for it today, it's current era. If the action plan would require a design document first, it might still be current era — just flag it as not ready.

---

## Step 7 — Milestone Slot

Use only milestones from Step 1. The current sprint is the lowest-numbered open milestone.

**Only slot if all three hold:**
1. Work is being actively planned for that sprint
2. Priority is P1 or P2
3. Size is XS, S, or M — L/XL issues must be broken down before slotting

If any condition fails, leave unslotted. Era label is enough. Premature slotting pollutes milestone burn-down.

```bash
# Only if slotting:
gh issue edit <number> --milestone "<title>"
```

---

## Step 8 — Actionability + Clarification Questions

**`size:L` / `size:XL` issues are almost never individually actionable.** For these, recommend splitting into:
1. A design ticket (small, becomes `ready` once open questions are answered)
2. One or more implementation tickets (each `ready` after design is resolved)

For `size:XS` through `size:M`, score actionability on three axes:

| Axis | Question |
|------|----------|
| Specificity | Is the affected file/module/function named? |
| Outcome | Is the expected behaviour described concretely enough to test? |
| Blockers | Are there unresolved design questions or external dependencies? |

Don't treat this as pass/fail. Note which axes are incomplete and what specifically would close the gap — this becomes the comment you post.

Apply `ready` only if all three axes are fully satisfied AND there are no open blocking dependencies.

**Clarification questions** — for any axis that is incomplete, write a concrete question that, when answered, would close that gap. These become the `clarification_needed` list in front matter. If all axes are satisfied, `clarification_needed` is empty.

Example:
```yaml
clarification_needed:
  - "Should the rename modal appear when the sidebar is hidden, or only when visible?"
  - "Which existing DrawCommand variant does this extend, or is it a new variant?"
```

**Dependency check:** If the issue depends on other open issues before work can start, populate the `depends_on` front matter and apply `blocked` instead of `ready`. If the issue body doesn't already have the front matter block, prepend it:
```
---
depends_on: [N, M]
---
```
Then: `gh issue edit <number> --add-label "blocked"`

---

## Step 9 — Agentic Reproduction Path

An agent cannot verify a fix without a failing test that proves the bug exists first. Before an agent starts implementation work, it needs a reproduction path it can drive programmatically.

Ask: **can the expected behavior be expressed as a `HostHarness` assertion?**

Score the issue on this axis:

| Verdict | Signal |
|---------|--------|
| **Test-first required** | Bug in host behavior — hitbox wrong, command dropped/misrouted, focus lost, state corrupt, event not delivered. The symptom maps directly to a `HostHarness` assertion (e.g. `click(x, y)` → `state.selected == pane_id`, or `inject(AiQuery)` → `effects not empty`). |
| **Test-first recommended** | UI regression or interaction bug where the failure mode is observable but not purely state-based. Write a snapshot test or a state test that approximates the failure. |
| **Test not applicable** | New feature (no existing behavior to assert against), pure refactor, docs, config, or CI change. Skip. |

**If test-first required or recommended:** record the concrete failing assertion in plain English. This becomes the first line of work — the agent writes the test before touching any implementation code. A fix without a passing test does not ship.

Example verdicts to include in the triage comment:
- `Reproduction test: click sidebar row at pane_2's y-coordinate → assert sidebar_selected == pane_2. Must fail before the fix lands.`
- `Reproduction test: inject DrawCommand::AiQuery → run 2 frames → assert effects_drain() is non-empty.`
- `Reproduction test: not applicable — new feature, no prior behavior to assert.`

---

## Step 10 — Apply Labels + Write Front Matter

Write or update the front matter block. If the issue body is missing the block entirely, prepend it. If it exists, update in place.

The canonical front matter shape:
```yaml
---
depends_on: []
touches: [src/app/mod.rs, sdk/python/]
clarification_needed: []
---
```

```bash
gh issue edit <number> --body "---
depends_on: []
touches: [src/app/mod.rs, sdk/python/]
clarification_needed: []
---

$(gh issue view <number> --json body --jq '.body')"
```

Then apply labels:

```bash
gh issue edit <number> --add-label "<type>,<priority>,<era>"
# If actionable and no open dependencies:
gh issue edit <number> --add-label "ready"
# If has open dependencies (depends_on populated):
gh issue edit <number> --add-label "blocked"
# If slotted:
gh issue edit <number> --milestone "<title>"
```

---

## Step 11 — Post Triage Summary

```bash
gh issue comment <number> --body "$(cat <<'COMMENT'
**Triage**

- **Type:** enhancement
- **Priority:** P2
- **Era:** v3.5+
- **Milestone:** unslotted
- **Size:** M (~400 LOC, high confidence) — [1-sentence reasoning]
- **Touches:** [src/app/mod.rs, sdk/python/]
- **Actionable:** partial — [what's missing and what would close the gap]
- **Clarification needed:** [question 1] / [question 2] (or "none")
- **Reproduction test:** [failing assertion in plain English, or "not applicable"]
- **Recommended next step:** [one concrete action — e.g. "write the HostHarness reproduction test, then implement the fix in the same PR"]
COMMENT
)"
```

The "Recommended next step" line is required. It should name one concrete action that moves the issue forward — not "needs more info" but "answer question X in a comment and update the action plan."

For bug issues with a test-first verdict: the recommended next step is always "write the `HostHarness` reproduction test first, confirm it fails, then implement the fix."
