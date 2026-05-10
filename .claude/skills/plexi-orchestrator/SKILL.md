# Plexi Orchestrator

Spawns parallel Claude Code `/ship` sessions in a tiled Plexi layout, with dependent groups stacking below.

**Trigger**: `/orchestrate [priority]`
- No arg → P1 first, then P2, then P3 (same discovery order as ship-issue)
- Priority arg → that level only (e.g. `/orchestrate P2`)
- Issue list → explicit override (e.g. `/orchestrate 101 102 103 -> 201 202`)

---

## Layout Rules

- **Parallel issues** (no dependency between them) → `split_v` = new pane to the **right**
- **Dependent group** (depends on previous group completing) → `split_h` = new pane **below** the first pane of the prior group, then `split_v` within the group

Example for groups `[101, 102, 103] → [201, 202]`:
```
[Coordinator][#101][#102][#103]
             [#201][#202]
```

---

## Phase 0 — Discover

Fetch unblocked issues at the target priority (same logic as ship-issue Phase 0):

```bash
gh issue list --label "<priority>" --label "ready" --state open \
  --json number,title,body,labels --limit 50
```

For each issue, parse `depends_on` from the body front matter:
```bash
gh issue view <n> --json body --jq \
  '.body | match("depends_on: \\[(?P<deps>[^\\]]*)\\]") | .captures[0].string'
```

Build dependency groups:
- Issues with no mutual `depends_on` relationships → same parallel group
- Issues that `depends_on` all members of a prior group → next sequential group

If an explicit issue list was passed with `->` syntax, use that directly as the groups.

**Present the proposed layout** before spawning anything:
```
Proposed layout:
  Group 1 (parallel): #101 — <title>, #102 — <title>, #103 — <title>
  Group 2 (after group 1): #201 — <title>, #202 — <title>

Spawn? (yes / edit)
```

Wait for confirmation. If "edit", let the user adjust the groups, then re-present.

---

## Phase 1 — Spawn Group 1

Capture coordinator pane and spawn the first parallel group:

```bash
COORD=$PLEXI_PANE_ID

# First issue — split_v from coordinator (to the right)
PANE_101=$(plexi terminal --layout split_v "claude '/ship 101'")
plexi pane name $PANE_101 "#101 — <title>"

# Each subsequent parallel issue — split_v from the previous pane
PANE_102=$(plexi terminal --layout split_v --from-pane-id $PANE_101 "claude '/ship 102'")
plexi pane name $PANE_102 "#102 — <title>"

PANE_103=$(plexi terminal --layout split_v --from-pane-id $PANE_102 "claude '/ship 103'")
plexi pane name $PANE_103 "#103 — <title>"

# Remember the anchor pane for each group (used for split_h when spawning next group)
GROUP_1_ANCHOR=$PANE_101
```

---

## Phase 2 — Monitor Group 1

Poll until all group 1 issues are closed:

```bash
echo "Monitoring group 1: #101 #102 #103"
for N in 101 102 103; do
  echo "  Waiting for #$N to close..."
  until [ "$(gh issue view $N --json state --jq '.state')" = "CLOSED" ]; do
    sleep 60
  done
  echo "  #$N closed."
done
echo "Group 1 complete."
```

Fire a non-blocking notification:

```bash
(RESULT=$(plexi notify \
  --title "Group 1 complete" \
  --body "#101 #102 #103 all closed — spawning group 2" \
  --choice "ok:Dismiss" \
  --choice "view:Talk to Claude:pane_focus:$COORD")
) &
```

---

## Phase 3 — Spawn Next Group

Spawn the dependent group below the prior group's anchor:

```bash
# First issue of group 2 — split_h from group 1 anchor (below)
PANE_201=$(plexi terminal --layout split_h --from-pane-id $GROUP_1_ANCHOR "claude '/ship 201'")
plexi pane name $PANE_201 "#201 — <title>"

# Each subsequent issue in group 2 — split_v (to the right)
PANE_202=$(plexi terminal --layout split_v --from-pane-id $PANE_201 "claude '/ship 202'")
plexi pane name $PANE_202 "#202 — <title>"

GROUP_2_ANCHOR=$PANE_201
```

Repeat Phase 2 → Phase 3 for each additional group.

---

## Phase 4 — All Groups Complete

```bash
plexi notify \
  --title "Orchestration complete" \
  --body "All groups closed." \
  --choice "ok:Dismiss"
```

---

## Notes

- `split_v` puts the new pane **to the right** (side-by-side). Use for parallel issues.
- `split_h` puts the new pane **below**. Use for the first issue of each dependent group.
- `--from-pane-id` requires `PLEXI_SOCKET` (must be inside a running Plexi pane).
- The coordinator pane must stay alive during monitoring — do not close it.
- Each spawned pane runs an independent `/ship` cycle. They do not coordinate with each other.
- If a ship cycle fails mid-run, the issue stays open and the group won't complete — the monitor loop will wait indefinitely. Surface this to the user via a manual check if a group takes unusually long.
