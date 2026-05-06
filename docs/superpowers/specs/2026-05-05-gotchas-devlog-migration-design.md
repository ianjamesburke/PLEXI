# Design: Replace DEV_LOG with GOTCHAS.md + Better Commit Messages

**Date:** 2026-05-05  
**Status:** Approved

---

## Problem

DEV_LOG.md has three issues:

1. **It's a conflict magnet.** Every PR writes to the top of the same file. Parallel agents collide on every merge into alpha. The git sync in Phase 5 of the ship skill regularly fails or requires manual conflict resolution because of this.

2. **Most entries are glorified commit messages.** "Changed X from Y to Z (PR #739)" adds no value beyond `git log`. The file has drifted toward noise, burying the genuinely valuable entries.

3. **It duplicates information.** Behavioral rules from DEV_LOG entries already live in CLAUDE.md's Lessons section. Framework gotchas belong in `coding-conventions`. The file has no clear ownership.

---

## Solution

Replace DEV_LOG.md with three properly-owned destinations:

| What | Where |
|---|---|
| What changed and why | Commit message (detailed, required) |
| Non-obvious PLEXI-specific discoveries | `GOTCHAS.md` (rare, optional) |
| Universal behavioral patterns | `~/.claude/CLAUDE.md` Lessons |
| Language/framework API quirks | `coding-conventions` skill |

---

## GOTCHAS.md

### Location
Repo root — same level as CLAUDE.md.

### Format
```
## YYYY-MM-DD — [area] Short title
What the gotcha is. What NOT to do. What to do instead.
```

**Area tags:** `git`, `macos`, `rust`, `egui`, `sdk`, `ship`, `cargo`, `python`, `cli`

### When to write an entry
Only when something genuinely surprised you:
- A failed approach that looks tempting and should be avoided
- A non-obvious environment or platform constraint
- A tool behavior that cost significant time to discover

**Do not write an entry for:** routine PR summaries, things visible in the diff, anything already documented in CLAUDE.md.

### Conflict behavior
Entries are rare by design. When conflicts do occur (two agents discover different gotchas simultaneously), resolution is always: keep both entries, newest-first.

---

## Commit Message Guidance

Commit messages replace DEV_LOG as the primary record of what happened and why.

**Format:**
```
type: what changed and why in one sentence

If the why needs more explanation, use the body. Explain the constraint,
the failed alternative, or the non-obvious decision. This is the record.
```

**Examples of good commit bodies:**
- "Replaced ff-only pull with rebase-based sync because local alpha can diverge from origin when prior sessions don't push bump commits."
- "Used socket-first open_cli instead of PATH lookup because PLEXI_SOCKET is the only reliable way to reach the correct running instance."

The ship skill will include explicit guidance to write detailed commit messages, especially for non-trivial changes.

---

## Ship Skill Changes

### Remove
Phase 5, step 3 — the mandatory DEV_LOG update — is removed entirely.

### Add
In Phase 4 (implementation), after committing: "Write a detailed commit message that explains the *why*. If you hit a non-obvious constraint or tried an approach that failed, either put it in the commit body or add one entry to GOTCHAS.md."

In Phase 5, after the squash merge: "If anything non-obvious happened during this PR, add one entry to GOTCHAS.md on alpha now. Skip if nothing surprised you."

### Phase 5 git sync — make divergence-proof
Replace all `git pull --ff-only` calls with a robust sync:

```bash
git fetch origin
AHEAD=$(git log origin/alpha..HEAD --oneline | wc -l | tr -d ' ')
BEHIND=$(git log HEAD..origin/alpha --oneline | wc -l | tr -d ' ')

if [ "$AHEAD" -gt 0 ] && [ "$BEHIND" -gt 0 ]; then
  # Diverged — rebase local on origin, resolve conflicts (GOTCHAS.md: keep all entries)
  git rebase origin/alpha
elif [ "$AHEAD" -gt 0 ]; then
  # Local ahead — push unpushed chore/bump commits
  git push
elif [ "$BEHIND" -gt 0 ]; then
  # Origin ahead — fast forward
  git merge --ff-only origin/alpha
fi
```

Document in the ship skill: when rebasing alpha and GOTCHAS.md conflicts, always keep both entries, newest-first. Same for any other append-only file.

---

## Improve Skill Changes

Add GOTCHAS.md as a third routing destination:

| Lesson type | Destination |
|---|---|
| Recurring behavioral pattern ("always do X before Y") | `~/.claude/CLAUDE.md` Lessons |
| Skill/agent workflow friction | skill's `SKILL.md` |
| One-time PLEXI-specific platform/process quirk | `GOTCHAS.md` |
| Language/framework API gotcha | `coding-conventions` skill |

---

## Migration Plan

### Step 1 — Audit DEV_LOG.md entries
Classify each entry:
- **Genuine PLEXI gotcha** → copy to GOTCHAS.md
- **Universal behavioral rule** → verify it's already in CLAUDE.md Lessons (most are); if not, add it
- **Framework/language quirk** → verify it's in `coding-conventions`; if not, add it
- **Routine PR summary** → drop

### Step 2 — Audit CLAUDE.md Lessons section
Classify each lesson:
- **Universal behavioral rule** (fires on every relevant task) → keep in CLAUDE.md
- **One-time PLEXI-specific discovery** → move to GOTCHAS.md
- **Language/framework API gotcha** → move to `coding-conventions` skill
- **DEV_LOG-related** → delete (obsolete)

Goal: reduce the Lessons section by ~40%, keeping only entries that meaningfully change Claude's behavior on a regular task.

### Step 3 — Delete DEV_LOG.md
Remove from the repo. No archive — the real gotchas will be in GOTCHAS.md, the rest wasn't worth keeping.

### Step 4 — Update skill files
- **Ship skill** — remove DEV_LOG step, add GOTCHAS guidance, replace ff-only with robust sync
- **Improve skill** — add GOTCHAS.md routing destination
- **dev-log skill** — retire (update description to redirect to GOTCHAS.md or delete)

### Step 5 — Update CLAUDE.md references
Remove all DEV_LOG references from project CLAUDE.md and global CLAUDE.md. Add GOTCHAS.md reference where relevant.

---

## Weekly Review

Read GOTCHAS.md top-to-bottom once a week. If the same area tag appears 3+ times, that's a signal to fix the underlying system rather than keep documenting the workaround — file a GitHub issue to address the root cause.

---

## Done When

- `GOTCHAS.md` exists at repo root with genuine gotchas extracted from DEV_LOG.md
- `DEV_LOG.md` is deleted
- Ship skill has no mandatory log step; has robust git sync; has GOTCHAS guidance
- Improve skill routes to GOTCHAS.md for one-time PLEXI-specific discoveries
- CLAUDE.md Lessons section is visibly shorter, containing only universal behavioral rules
- `coding-conventions` skill has any framework lessons extracted from CLAUDE.md
- `dev-log` skill is retired
