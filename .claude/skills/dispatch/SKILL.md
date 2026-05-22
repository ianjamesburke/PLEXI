---
name: dispatch
description: Use when the user says /dispatch or wants to initialize a multi-pane Plexi workspace — auto-computes parallel lanes from live issue state (grouped by area:* label), writes NEXT.md, then opens up to 4 Claude panes with queued sequential issues below each.
---

# Dispatch

Auto-computes lane assignments from live GitHub issue state, writes `NEXT.md`, then initializes a Plexi multi-pane Claude workspace.

## Layout model

```
[Orchestrator]  |  [Lane A: #934]
                |  [Lane B: #321]
                |  [Lane C: #918]
```

- **First lane:** horizontal split right of the orchestrator pane
- **Lane 2+:** vertical splits below the previous lane (stacking on the right side)
- **Queue panes:** vertical split below their lane, command staged but not started

## Invocation

```
/dispatch              # auto-computes lanes, writes NEXT.md, opens panes
/dispatch path/to/next.md   # skip Step 0, read that file directly
```

---

## Step -1 — Alpha gate check (inline, non-interactive)

Run these two commands directly. If either returns output, STOP immediately — do not open any panes.

```bash
git status --porcelain
git log origin/alpha..HEAD --oneline
```

If `git status --porcelain` has output: print `ALPHA BLOCKED — working tree is dirty. Run stabilize-alpha first.` and stop.

If `git log origin/alpha..HEAD` has output: print `ALPHA BLOCKED — unpushed commits on alpha. Run stabilize-alpha first.` and stop.

If both return empty: proceed to Step 0.

> This is a gate, not a fixer. Stabilize-alpha is the separate pre-dispatch command that handles commits, rebases, PR review, and cargo check. Dispatch trusts that gate was already cleared.

---

## Step 0 — Compute lanes from issue state

Skip if a file path was passed as an argument.

### 0a — Fetch ready issues

```bash
gh issue list --label "ready" --state open --json number,title,labels,body --limit 100
```

### 0b — Classify each issue

```
{ number, title, priority (P0–P4; 99 if none), area (first area:* label; null if none), is_bundle, in_progress, blocked }
```

Exclude `in_progress` and `blocked` from lane assignment. Surface issues with no `area:*` label separately.

### 0c — Check for unlabeled P0/P1 issues

If any unlabeled issue has priority P0 or P1: STOP. Do not open panes. Triage first.

### 0d–0f — Build lane map, coalesce bundles, select top 4

Group by area, sort each lane by priority ASC then number ASC. Coalesce consecutive leading `bundle` issues into `+`-joined strings (e.g. `"1149+1150"`). Take the top 4 areas by lead issue priority.

### 0g — Write NEXT.md

```markdown
# NEXT.md
<!-- computed: <ISO timestamp> -->

## Lanes
| Lane | Area | Lead | Queue |
|------|------|------|-------|
| A | area:host/navigation | #934 | #316 |
| B | area:ui/overlays | #321+#322 (bundle) | #354 |

## Deferred (beyond 4-lane cap)
- **area:sdk/pgap**: #1149

## Needs area label
- #1152 — ux(close): closing a pane should advance focus to next sibling [P0 ⚠]
```

---

## Step 1–2 — Parse NEXT.md, skip closed leads

Extract lanes array (cap 4). For each lead, check `gh issue view <N> --json state --jq '.state'`. Advance past closed leads; drop fully-closed lanes.

---

## Step 3 — Open lane panes

```bash
bash .claude/skills/dispatch/scripts/open-lanes.sh 934 321 918
```

Script handles: channel detection (`$PLEXI_CHANNEL`), `$PLEXI_PANE_ID` anchor, `split_h` for lane 1, `split_v` for lanes 2+, naming, sending ship command. All panes open with `--no-focus`.

---

## Step 4 — Open queue panes (staged, not started)

```bash
PLEXI=plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL}
QUEUE_ID=$($PLEXI terminal --layout split_v --from-pane-id $LANE_X_ID --no-focus)
$PLEXI pane name $QUEUE_ID "queued: #<next>"
$PLEXI pane send $QUEUE_ID 'c "/ship-issue <next>"'
```

No `\n` — command is staged, user presses Enter when the lane above finishes.

---

## Step 5 — Focus Lane 1

```bash
PLEXI=plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL}
$PLEXI pane focus $LANE_1_ID
```

---

## Adding to an existing dispatch

Check active panes with `$PLEXI pane list` (title field shows what's running), then:

```bash
bash .claude/skills/dispatch/scripts/add-to-dispatch.sh <existing_pane_id> <issue_number>
```

---

## Recovering failed lanes

Before re-sending commands to an existing pane, **always capture its scrollback first** to check if a Claude session is already active:

```bash
$PLEXI pane capture $PANE_ID --lines 20
```

If the pane has an active Claude session, close it and open a fresh one. Never `pane send` a ship command into a pane that already has Claude running — it types the command as literal user input into the active session.

## Notes

- **Step -1 is mandatory.** If alpha is dirty or has unpushed commits, dispatch stops immediately. Run `stabilize-alpha` first — it's a separate command that handles commits, rebases, and cargo check.
- **Channel binary:** `PLEXI=plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL}` — stable sets no `$PLEXI_CHANNEL`, resolves to `plexi`. Never hardcode a channel name.
- **Scripts** in `scripts/` own all pane mechanics. Run them; don't reconstruct bash by hand.
- `c` is a zsh alias (login shell) — available in all interactive panes opened by Plexi.
- NEXT.md is a cache — re-run `/dispatch` to recompute as issues close.
- P0/P1 unlabeled issues block dispatch entirely — triage first.
