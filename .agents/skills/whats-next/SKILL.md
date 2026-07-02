---
name: whats-next
description: Session orientation skill. Re-audits stint tasks, checks open PRs, and rewrites WHATS_NEXT.md (the Arc + Priority Stack). Use at the start of any session when you don't know what to work on next, or after a batch of merges when priorities may have shifted. Also called inline by merge-pr after each merge to keep the file in sync.
---

# What's Next

Session orientation AND the shared `WHATS_NEXT.md` update routine. Called by `/merge-pr` after every merge so the file is always current. Also invoke manually at session start.

---

## Inclusion Rule (contract with create-stint)

This section is the single source of truth for what belongs in `WHATS_NEXT.md`. `create-stint` references it; do not restate it elsewhere.

An item may appear in the file only if **all** of these hold:

1. **Entirely actionable.** No open decisions remain except ordinary implementation choices an agent makes while coding. If a product, pricing, architecture, or scope decision is still open, the item is not ready for the file.
2. **The decision itself can be the task.** Unresolved-decision work enters the file only as an explicit spec/decision stint task (e.g. `0315` monetization spec). Making the decision is the actionable unit; downstream tasks stay `blocked by` it and appear only as indented children.
3. **Implementation detail lives on the stint task, never here.** The stint task body owns instructions, acceptance criteria, and context — written there by `create-stint` before the ID is slotted into this file. The file carries only the task ID plus a one-line outcome framing (what epoch/outcome it serves).
4. **Done work leaves immediately.** Landed tasks move to `docs/DEVLOG.md`; they never linger as history here.

Flow: idea → `/create-stint` (full detail on the task) → `/whats-next` slots the ID into the Arc under the outcome it serves. Never the reverse.

---

## Step 1 — Gather live state

Run all of these in parallel:

```bash
stint list 2>&1
stint status 2>&1
git log --oneline -10 2>&1
gh pr list --state open --limit 20 2>&1
```

Cross-check: any task the file lists as ready/P0/P1 must still exist and be non-done in `stint list`. `stint show <id>` anything suspicious — merged PRs referencing a stint ID mean the task is done even if the file says otherwise.

---

## Step 2 — Classify every task

For each task in `stint list` output, assign it to one of:

| Bucket | Criteria |
|--------|----------|
| **P0 — on fire** | Broken demo path, regression visible to any user |
| **P1 — ship blocker** | Required for first-user story or unblocks a P1 chain |
| **P2 — important** | Core feature, significant tech debt, or blocks P2 chain |
| **P3 — polish** | Nice to have, infra hygiene, non-blocking |
| **blocked** | Has an unresolved `blocked by` dependency |
| **in-pipeline** | PR is open — check `gh pr list` |

Cross-reference `blocked by` chains. A task blocked by a blocked task is deep-blocked; note it.

Apply the Inclusion Rule: tasks failing it stay in `stint list` only (maintenance, undecided ideas) and do not enter the file.

---

## Step 3 — Audit the v1 finish-line gap

The finish line: a stranger installs Plexi, an agent builds a working app from the scaffold on the first try, a reviewed free app installs from the hosted registry. Check whether anything on that path lacks a task; flag gaps as missing rather than inventing entries.

---

## Step 4 — Rewrite WHATS_NEXT.md

Overwrite `.agents/skills/whats-next/WHATS_NEXT.md` preserving its current structure:

- **Header quote block** — pointers to this skill, `docs/DEVLOG.md`, `NORTH_STAR.md`. Sprints are retired; never reintroduce sprint fields.
- **Current State (date)** — alpha HEAD, what landed since last audit (one sentence, then move detail to DEVLOG), open PRs that affect priority reading, "not real yet" list.
- **The Arc** — epochs ordered toward `NORTH_STAR.md`. Tasks indented under the outcome they serve; nested tasks are blocked by their parent. One line per task: ID + outcome framing, nothing more (Inclusion Rule §3).
- **Priority Stack (flat view)** — P0/P1 as ID lists, `*` marks blocked, single **next recommended task** line.
- **Key Reference Docs** — table of doc pointers.
- **How To Update This File** — points back at this skill.

Fill every section from the live data gathered in Steps 1–3. Landed-work history goes to `docs/DEVLOG.md` (append a dated entry), never accumulates in the file.

---

## Step 5 — Present the summary

After writing the file, output a terse summary to the user:

```
WHATS_NEXT.md updated.

P0 (3): 0330 DX chain head, 0327 event bus, 0280 palette scroll
P1 (6): [ids + one-word hooks]
Blocked chain: 0285 → 0286 → 0287

RECOMMENDATION:
1. Start with 0330 (app-dev CLI audit, P0 chain head) — unblocks 0331/0332/0215.
```

The recommendation must be a single next task — the one the user should pick up right now.
