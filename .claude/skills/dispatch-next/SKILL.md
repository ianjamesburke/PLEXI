---
name: dispatch-next
description: Use when the user says /dispatch-next or wants to initialize a multi-pane Plexi workspace from NEXT.md — creates up to 4 parallel Claude lanes for independent issues and queues sequential issues vertically below each lane.
---

# Dispatch Next

Reads `NEXT.md` (or a path argument) and initializes a Plexi multi-pane Claude workspace.

## Layout model

```
[Lane A: c "ship issue 934"]  [Lane B: c "ship issue 321"]  [Lane C: c "ship issue 918"]
[queued: ship #316]           [queued: ship #354]           (no queue)
```

- **Horizontal** (up to 4): each parallel lane gets a live Claude instance
- **Vertical** (below each): next sequential issue pre-typed, not started — user presses Enter when the lane above finishes

## Invocation

```
/dispatch-next              # reads NEXT.md in repo root
/dispatch-next path/to/next.md
```

---

## Step 1 — Parse NEXT.md

Read the file. Extract:
- The dependency graph (ASCII)
- The "What runs in parallel" lanes table
- The ordered next steps list

Build a lanes array (cap at 4):

```
lanes = [
  { lead: 934, queue: [316] },
  { lead: 321, queue: [354, 316] },
  { lead: 918, queue: [] },
]
```

`lead` = first unshipped issue in the lane.  
`queue` = ordered list of issues that follow sequentially.

If NEXT.md has no explicit lanes table, derive lanes yourself from the dependency graph — independent branches are separate lanes.

---

## Step 2 — Skip already-closed leads

For each lead issue, check state:

```bash
gh issue view <N> --json state --jq '.state'
```

If `CLOSED`, advance to the next issue in that lane's queue as the new lead. If the whole lane is closed, drop it.

---

## Step 3 — Snapshot existing panes

```bash
BEFORE_IDS=$(plexi pane list | python3 -c \
  "import json,sys; print(' '.join(str(p['id']) for p in json.load(sys.stdin)))")
```

---

## Step 4 — Open lane panes (horizontal)

For each active lane, open a terminal to the right of the current layout:

```bash
plexi terminal --layout split_h
```

After each `plexi terminal` call, find the new pane ID:

```bash
NEW_ID=$(plexi pane list | python3 -c "
import json, sys
before = set(map(int, '''$BEFORE_IDS'''.split()))
panes = json.load(sys.stdin)
new = [p['id'] for p in panes if p['id'] not in before]
print(new[-1] if new else '')
")
```

Label it and start Claude inside it (no newline = not submitted yet for first lane; use `\n` to immediately start):

```bash
plexi pane name $NEW_ID "Lane <X>: #<lead>"
plexi pane send $NEW_ID 'c "ship issue <lead>"\n'
```

`c` is the local alias for `IS_DEMO=1 claude --model sonnet --dangerously-skip-permissions --allow-dangerously-skip-permissions`.

Save each lane's pane ID: `LANE_<X>_ID=$NEW_ID`.  
Update `BEFORE_IDS` with the new ID before opening the next lane.

---

## Step 5 — Open queue panes (vertical)

For each lane that has a non-empty queue, focus the lane pane and open a split below it:

```bash
plexi pane focus $LANE_<X>_ID
plexi terminal --layout split_v
```

Find the new pane ID (same pattern as Step 4).

Pre-type the queued command **without** `\n` — it sits ready, not started:

```bash
plexi pane name $QUEUE_ID "queued: #<next>"
plexi pane send $QUEUE_ID 'c "ship issue <next>"'
```

If the lane has more than one queued issue (e.g. lane B has both #354 and #316 downstream), only queue the immediate next — not the full chain. The user will handle deeper queuing after #354 ships.

---

## Step 6 — Focus Lane 1

```bash
plexi pane focus $LANE_1_ID
```

Leave the user's cursor in the first active lane.

---

## Notes

- `plexi terminal` does not return a pane ID — always diff `plexi pane list` before/after to get it.
- `split_h` adds to the right of the focused pane. `split_v` adds below.
- For queue panes, omit `\n` from `plexi pane send` so the command is staged but not executed.
- `c` is a personal zsh alias (login shell) — available in all interactive panes opened by Plexi.
- If a lane's lead issue has a feature branch already in progress (`wtp list` or `git worktree list`), note it in the pane title: `"Lane A: #934 (branch exists)"`.
- If NEXT.md is absent or has no parseable lanes, surface the issue list from `gh issue list --label "ready" --json number,title` and ask the user to confirm the lane assignment before opening any panes.
