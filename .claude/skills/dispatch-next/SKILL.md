---
name: dispatch-next
description: Use when the user says /dispatch-next or wants to initialize a multi-pane Plexi workspace — auto-computes parallel lanes from live issue state (grouped by area:* label), writes NEXT.md, then opens up to 4 Claude panes with queued sequential issues below each.
---

# Dispatch Next

Auto-computes lane assignments from live GitHub issue state, writes `NEXT.md`, then initializes a Plexi multi-pane Claude workspace.

## Layout model

```
[Lane A: c "ship issue 934"]  [Lane B: c "ship issue 321"]  [Lane C: c "ship issue 918"]
[queued: ship #316]           [queued: ship #354]           (no queue)
```

- **Horizontal** (up to 4): each parallel lane gets a live Claude instance
- **Vertical** (below each): next sequential issue pre-typed, not started — user presses Enter when the lane above finishes

## Invocation

```
/dispatch-next              # auto-computes lanes, writes NEXT.md, opens panes
/dispatch-next path/to/next.md   # skip Step 0, read that file directly
```

---

## Step 0 — Compute lanes from issue state

Skip this step only if a specific file path was passed as an argument.

### 0a — Fetch ready issues

```bash
gh issue list --label "ready" --state open \
  --json number,title,labels,body --limit 100
```

### 0b — Classify each issue

For each issue build a record:

```
{
  number: N,
  title: "...",
  priority: 0-4,       # extracted from P0..P4 label; 99 if none
  area: "area:X/Y",    # first area:* label found; null if none
  is_bundle: bool,     # has "bundle" label
  in_progress: bool,   # has "in progress" label
  blocked: bool,       # has "blocked" label
}
```

**Exclude** from lane assignment:
- `in_progress` — already running
- `blocked` — dependency unresolved

**Surface separately** (do not exclude — just flag):
- Issues with no `area:*` label

### 0c — Check for unlabeled P0/P1 issues

If any excluded-from-lanes issue has priority P0 or P1:

```
STOP. The following high-priority issues have no area label and cannot be auto-dispatched:
  #N — <title> [P0]
  #M — <title> [P1]

Triage these first: gh issue edit <N> --add-label "area:<namespace>/<module>"
```

Do not open any panes. Do not proceed to Step 1.

### 0d — Build lane map

Group remaining (area-labeled, non-excluded) issues by their `area` value.

```python
lanes = {}
for issue in eligible:
    area = issue.area
    lanes.setdefault(area, []).append(issue)

# Sort each lane: priority ASC, then number ASC
for area in lanes:
    lanes[area].sort(key=lambda i: (i.priority, i.number))
```

### 0e — Coalesce bundle issues

Within each lane, if the lane's leading issues are ALL labeled `bundle`, merge them into a single ship call:

```
# Before: lane = [#1149(bundle), #1150(bundle), #1160]
# After:  lane = [bundle(#1149+#1150), #1160]
```

A bundle lead is represented as a `+`-joined string: `"1149+1150"`.  
Non-bundle issues that follow are individual queue entries.

If a lane has a mix of bundle and non-bundle issues at the front, only coalesce the consecutive leading bundle issues.

### 0f — Select active lanes

Sort areas by their lane's lead issue priority (ASC), then lead issue number (ASC).  
Take the first **4** as active lanes. Remaining areas are deferred.

### 0g — Write NEXT.md

Write to the repo root:

```markdown
# NEXT.md
<!-- computed: <ISO timestamp> -->

## Lanes

| Lane | Area | Lead | Queue |
|------|------|------|-------|
| A | area:host/navigation | #934 | #316 |
| B | area:ui/overlays | #321+#322 (bundle) | #354 |
| C | area:host/pane-ops | #918 | |

## Deferred (beyond 4-lane cap)

- **area:sdk/pgap**: #1149, #1144

## Needs area label

- #1153 — quick-note: ArrowRight/ArrowLeft not bound in destination picker [P3]
- #1152 — ux(close): closing a pane should advance focus to next sibling [P0 ⚠]
```

If there are no deferred areas or no unlabeled issues, omit those sections.

Print the NEXT.md content to stdout so the user sees the computed plan.

---

## Step 1 — Parse NEXT.md

Read the file (written by Step 0, or the path argument). Extract the lanes table.

Build a lanes array (cap at 4):

```
lanes = [
  { lead: "934",       queue: ["316"] },
  { lead: "321+322",   queue: ["354"] },
  { lead: "918",       queue: [] },
]
```

`lead` = first unshipped entry in the lane. May be a `+`-joined bundle string.  
`queue` = ordered list of issue numbers (or bundle strings) that follow.

If NEXT.md has no explicit lanes table, derive lanes from any dependency graph present — independent branches are separate lanes.

---

## Step 2 — Skip already-closed leads

For each lead issue (or each issue in a bundle lead), check state:

```bash
gh issue view <N> --json state --jq '.state'
```

If a single-issue lead is `CLOSED`, advance to the next queue entry as the new lead.  
If all issues in a bundle lead are `CLOSED`, advance to the next queue entry.  
If the whole lane is closed, drop it.

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

Build the ship command from the lead:

- Single issue `"934"` → `c "ship issue 934"`
- Bundle `"321+322"` → `c "ship issue 321 322"`

Label and start:

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

If the lane has more than one queued issue, only queue the immediate next — the user handles deeper queuing after that issue ships.

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
- NEXT.md is a cache. It goes stale as issues close. Re-run `/dispatch-next` to recompute.
- The "Needs area label" section in NEXT.md is the triage backlog for the area labeling pass. P0/P1 items there block dispatch entirely.
