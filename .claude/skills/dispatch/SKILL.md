---
name: dispatch
description: Use when the user wants to ship one or more issues. Runs open-lanes.sh to open parallel implement-issue panes — one per issue. Each pane self-orchestrates the full pipeline inline (implement → open-pr → validate-pr → merge-pr). An issue number is always required.
---

# Dispatch

Open parallel implement-issue panes. Each pane self-orchestrates its own pipeline.

## Invocation

```
/dispatch 1671        # ship issue #1671
/dispatch 1671 1679   # ship two issues in parallel
```

An issue number is always required. Dispatch does not auto-pick — use `/pick-parallel` first if nothing is queued.

---

## Step 1 — Alpha gate

```bash
git status --porcelain
git log origin/alpha..HEAD --oneline
```

If either has output: print `ALPHA BLOCKED — working tree is dirty or has unpushed commits.` and stop.

---

## Step 2 — Validate issue(s)

For each issue number, confirm it's open and not already in progress:

```bash
gh issue view <N> --json number,title,state,labels \
  --jq '{number, title, state, labels: [.labels[].name]}'
```

If state is `CLOSED`: stop, tell the user.
If it has `in progress` label: stop, tell the user it's already being worked.

Print what's about to be dispatched:
```
Dispatching #1671 — fix(infra/skills): implement-issue preflight optimization
```

---

## Step 3 — Open lanes

```bash
bash .claude/skills/dispatch/scripts/open-lanes.sh <issue1> [issue2...]
```

This opens one Plexi terminal pane per issue, each running `c '/implement-issue N'`. The pipeline self-orchestrates inline from there:

```
implement-issue → open-pr → validate-pr (notify user, wait) → merge-pr
```

All phases run in the same pane. Each pane closes itself at the end of merge-pr and fires a notify.

---

## Notes

- Dispatch is fire-and-forget after lanes are open. You will be notified when each pipeline completes via `plexi notify`.
- If a pane crashes mid-pipeline: re-run `/dispatch N` for that issue. implement-issue will detect the in-progress state and ask for takeover confirmation, or open-pr/validate-pr will resume from the Ship Log.
- To add a lane to an existing dispatch: `bash .claude/skills/dispatch/scripts/add-to-dispatch.sh <pane_id> <issue>`
