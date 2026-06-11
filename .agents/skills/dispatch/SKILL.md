---
name: dispatch
description: Use when the user wants to ship one or more issues. Opens parallel implement-issue panes — one per issue, or one shared pane for a bundle. Each pane self-orchestrates the full pipeline inline (implement → open-pr → validate-pr → merge-pr). An issue number is always required.
---

# Dispatch

Open parallel implement-issue panes. Each pane self-orchestrates its own pipeline.

## Invocation

```
/dispatch 1671        # ship issue #1671
/dispatch 1671 1679   # ship two issues in parallel
```

An issue number is always required. Dispatch does not auto-pick. To choose lanes, read `docs/prm/app-framework-marketplace.md`, audit matching open GitHub issues, and pick ready unblocked issues whose `area:*` labels do not overlap.

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

## Step 3 — Name the sprint

Pick a short descriptive name for this dispatch batch. Base it on what the issues have in common — their shared `area:*` label, a common theme, or the feature being built. Examples: `sdk-frames`, `cli-completions`, `app-protocol`. If nothing ties them together, fall back to `dispatch/N1+N2+...`.

If the repo uses GitHub milestones, the milestone title is a good name — but this is optional, not the default.

---

## Step 4 — Create subcontext and open lanes

Split the current window to create a new anchor pane, push it into a sub-context, then open each lane as a window inside that sub-context. The orchestrator pane stays outside and untouched.

```bash
PLEXI=plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL}

# Create sub-context and all lanes atomically.
# Use an array (not eval + string concat) so sprint names with quotes don't break the command.
WINDOW_ARGS=()
for ISSUE in <issue1> [issue2...]; do
  WINDOW_ARGS+=("--window" "c '/implement-issue $ISSUE'")
done
$PLEXI context new "$SPRINT_NAME" --path "$PWD" "${WINDOW_ARGS[@]}"
```

**Bundle mode:** if the user passes multiple issues that should share a single PR, open ONE window with all numbers: `c '/implement-issue N1 N2 N3'`. Name it `#N1+N2+N3`.

The pipeline self-orchestrates inline from there:

```
implement-issue → open-pr → validate-pr (notify user, wait) → merge-pr
```

All phases run in the same window. Each successful window closes itself at the end of merge-pr without firing a redundant success notification; failures and user-action states still notify.

---

## Notes

- Dispatch is fire-and-forget after lanes are open. You will be notified for validation handoffs, failures, and user-action states; successful merge panes close quietly.
- If a pane crashes mid-pipeline: re-run `/dispatch N` for that issue. implement-issue will detect the in-progress state and ask for takeover confirmation, or open-pr/validate-pr will resume from the Ship Log.
- To add a lane to an existing dispatch: `bash .claude/skills/dispatch/scripts/add-to-dispatch.sh <pane_id> <issue>`
