---
name: whats-next
description: Session orientation skill. Re-audits stint tasks, checks open PRs, and rewrites WHATS_NEXT.md with the current priority stack. Use at the start of any session when you don't know what to work on next, or after a batch of merges when priorities may have shifted. Also called inline by merge-pr after each merge to keep the file in sync.
---

# What's Next

Session orientation AND the shared `WHATS_NEXT.md` update routine. Called by `/merge-pr` after every merge so the file is always current. Also invoke manually at session start.

---

## Step 1 — Gather live state

Run all of these in parallel:

```bash
stint list 2>&1
stint status 2>&1
git log --oneline -10 2>&1
gh pr list --state open --limit 20 2>&1
```

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

---

## Step 3 — Audit "finding first users" gap

Check whether these exist as tasks. If not, flag as missing:

1. README that lets a stranger self-install (look for stint task referencing README or install docs)
2. Website that can be shared publicly (look for `plexiapp.com` / visual refresh task)
3. At least one demo app working end-to-end (todo app is canonical)
4. Onboarding flow for non-technical users (`plexi ai doctor` or equivalent)

---

## Step 4 — Rewrite WHATS_NEXT.md

Overwrite `.agents/skills/whats-next/WHATS_NEXT.md` with:

```markdown
# What's Next

> Read this at the start of any Plexi session. It is the single anchor for orientation.
> Skill: `/whats-next` — re-runs the audit and updates this file.

---

## Current State (<today's date>)

**Sprint <id>:** <X/Y done> — <remaining>h remaining. <N> in-progress, <M> ready.

---

## Priority Stack

### P0 — Ship These First
[table: task id | title | why it's P0]

### P1 — Core Feature Completeness
[table: task id | title | blocked by | why]

### P2 — Important, Not Blocking Ship
[table: task id | title | notes]

### P3 — Polish / Backlog
[one-liner summary: "N tasks, run `stint list` for full list"]

---

## Finding First Users

[Ordered list: what's missing before sharing publicly, with task IDs where they exist]

**First-user critical path:** [chain of task IDs → action]

---

## Blocked Chains

[ASCII tree of blocked dependencies, one per chain]

---

## Key Reference Docs

| Doc | What it covers |
|-----|----------------|
| `NORTH_STAR.md` | Vision, phases, what does/doesn't belong |
| `docs/app-framework-marketplace.md` | App framework + marketplace execution plan |
| `docs/wasm-runtime.md` | WASM runtime full spec |
| `docs/assistant-host-app.md` | Assistant app spec |
| `docs/SDK_EVOLUTION.md` | SDK v3 direction |
| `src/testing/TESTING.md` | Test infra reference |

---

## How to Update This File

Run `/whats-next` at the start of any session. Do not hand-edit the Priority Stack — it drifts. Edit "Finding First Users" and "Key Reference Docs" sections manually as strategy shifts.
```

Fill every section from the live data gathered in Steps 1–3.

---

## Step 5 — Present the summary

After writing the file, output a terse summary to the user:

```
WHATS_NEXT.md updated.

P0 (2): 0299 todo rebuild, 0280 palette scroll
P1 (5): 0241 subcontexts, 0292 README, 0285 WASM Python, 0295 manifest fix, 0296 install paths
P2 (8): [list]
Blocked chain: 0285 → 0286 → 0287

First-user gap: README (#0292) and todo app (#0299) must ship first. No onboarding task exists yet.

RECOMMENDATION:
1. Start with 0299 (todo rebuild, P0, ~2h) — clears the demo regression and unblocks showing the product.
```

The recommendation must be a single next task — the one the user should pick up right now.
