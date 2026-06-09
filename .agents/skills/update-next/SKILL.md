---
name: update-next
description: Regenerate NEXT.md — snapshot north star, in-progress/P0/P1/P2 issues, and align the roadmap. Run when starting a session, after shipping, or when the user asks to update the roadmap.
---

# Update NEXT.md

Regenerates `NEXT.md` at the repo root as a **live roadmap snapshot**. This is the dispatch source for `/dispatch-next`.

## When to run

- User says `/update-next`
- After a batch of issues ship (post-merge)
- Start of a planning session
- When the user asks "what's next" or "update the roadmap"

---

## Step 1 — Gather state

Run these queries in parallel:

```bash
# North star one-liner
head -30 NORTH_STAR.md

# P0 issues (drop everything)
gh issue list --state open --label "P0" --json number,title,labels --jq '.[] | {number, title, labels: [.labels[].name]}'

# P1 issues (shipping blockers)
gh issue list --state open --label "P1" --json number,title,labels --jq '.[] | {number, title, labels: [.labels[].name]}'

# P2 ready issues
gh issue list --state open --label "P2" --label "ready" --json number,title,labels --jq '.[] | {number, title, labels: [.labels[].name]}'

# In-progress issues
gh issue list --state open --label "in progress" --json number,title,labels,assignees --jq '.[] | {number, title, labels: [.labels[].name]}'

# Blocked issues
gh issue list --state open --label "blocked" --json number,title,labels --jq '.[] | {number, title, labels: [.labels[].name]}'

# Open PRs
gh pr list --state open --json number,title,headRefName --jq '.[] | {number, title, branch: .headRefName}'

# Recent closed issues (last 2 weeks)
gh issue list --state closed --limit 20 --json number,title,closedAt --jq '.[] | {number, title, closed: .closedAt}'

# Current version
grep '^version' Cargo.toml | head -1
```

---

## Step 2 — Read existing NEXT.md

If `NEXT.md` exists, read it. Preserve:
- The **title and horizon** (update dates if stale)
- The **"Mode" section** (editorial — keep as-is unless user says otherwise)
- The **"What's Already Solid" section** (update only if new capabilities shipped)
- The **"Parking Lot" section** (append new ideas, don't remove existing ones)
- Any **YouTube video recording notes**

---

## Step 3 — Reconcile stages

For each stage in the existing NEXT.md:

1. **Check every issue in the stage table** against `gh issue view <N> --json state,labels --jq '{state, labels: [.labels[].name]}'`
2. Mark completed issues with ✅ and note the PR number if closed by a PR
3. If ALL issues in a stage are closed, mark the stage header as **DONE ✅**
4. If an issue was moved to a different priority or relabeled, note the change
5. If a new P0/P1 issue appeared that isn't in any stage, flag it as **NEW — needs placement**

---

## Step 4 — Build the dispatch block

At the bottom of the file (before Parking Lot), add or update a `## Dispatch Queue` section. This is what `/dispatch-next` reads.

Format:

```markdown
## Dispatch Queue

_Auto-generated YYYY-MM-DD. Issues ordered by priority, then by number._

### Parallel (no dependencies between these)
| Lane | Issue | Title | Priority |
|------|-------|-------|----------|
| A | #NNN | title | P1 |
| B | #NNN | title | P2 |

### Sequential (must complete in order)
| Order | Issue | Title | Blocked by |
|-------|-------|-------|------------|
| 1 | #NNN | title | — |
| 2 | #NNN | title | #NNN |
```

Rules for building the dispatch queue:
- Only include issues labeled `ready` (not `blocked`, not `needs-info`)
- P0 always goes first, alone in its own lane
- P1 `ready` issues fill remaining lanes (up to 4 total)
- P2 `ready` issues queue behind P1 lanes
- Issues labeled `in progress` get lane priority (they're already started)
- If an issue has a blocking dependency (check `gh api repos/{owner}/{repo}/issues/{number}` for sub-issues or cross-references), put it in Sequential
- Cap at 4 parallel lanes

---

## Step 5 — Write NEXT.md

Overwrite the file with the updated content. The structure is:

```
# NEXT — [title from existing or user-provided]

**Horizon:** [date range]
**Goal:** [from existing or north star]
**Current version:** [from Cargo.toml]

---

## Mode: [from existing]

[preserved editorial content]

---

## What's Already Solid (don't touch)

[preserved or updated capability list]

---

## Stage N — [date range]: [theme]  [DONE ✅ if complete]

| # | Title | Status |
|---|---|---|
| #NNN | title | ✅ / In progress / Ready / PR #N open |

[YouTube notes if any]

---

## Dispatch Queue

[generated dispatch block from Step 4]

---

## Ship Checklist

[all issues across all stages as a flat checklist]

---

## Parking Lot

[preserved from existing, new ideas appended]
```

---

## Step 6 — Surface changes

After writing, report to the user:
- How many issues changed state since last snapshot
- Any new P0/P1 issues not yet placed in a stage
- Whether the dispatch queue changed
- Whether any stage completed

Keep the report to 5 lines max.

---

## Notes

- Never remove issues from the file — mark them ✅ or note they were closed/moved
- The Dispatch Queue is the machine-readable part; everything else is human-readable context
- If NEXT.md doesn't exist yet, ask the user for: title, horizon dates, and goal — then generate from scratch using the issue data
- `dispatch-next` reads the Dispatch Queue section — keep the table format exact
- P3/P4 issues don't appear in the dispatch queue unless the user explicitly adds them
