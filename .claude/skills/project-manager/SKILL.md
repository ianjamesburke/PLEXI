---
name: project-manager
description: Reactive 4-lane dispatcher. Keeps up to 4 agent panes running at all times — fills empty slots from the ready queue (newest-first), watches on a timer, and refills as lanes complete. Use when starting a dispatch session, told to watch the queue, or asked to keep agents running. Not for single-issue work — use /implement-issue for that.
---

# Project Manager

Reads current state, fills empty slots, watches on a timer. No scoring by default — use `/score` separately if needed.

---

## Pre-check

```bash
if [ -z "$PLEXI_PANE_ID" ]; then
  echo "ERROR: must run inside a Plexi pane" >&2
  exit 1
fi
PLEXI=plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL}
$PLEXI pane name "PLEXI PM"
```

---

## Step 1 — Parse user context

Read what the user said before fetching anything. Extract signals:

| Signal | Behaviour |
|---|---|
| "just added issues" / "new issues" / "just filed" | Newest-first, skip scoring |
| "score" / "prioritize" | Invoke `/score` skill first, then use scored order |
| "stop" / "done watching" | Cancel loop — do not call ScheduleWakeup |
| "wind down" / "drain" / "let it finish" / "no new lanes" | Set `DISPATCH=false` — existing lanes finish, no new ones open |
| Number (e.g. "run 6 lanes", "use 3") | Set `MAX_LANES` to that number |
| No signal | Default: newest-first, watch loop on |

Set `ORDER_MODE` = `newest-first` (default) or `scored`.
Set `WATCH` = true (default) or false.
Set `DISPATCH` = true (default) or false. When false: existing lanes run to completion, pipeline stages (validate/merge) still resume, but no fresh issues are dispatched.
Set `MAX_LANES` = number from message, or fall back to value in `.claude/agent-memory/project-manager/config.json` (`max_lanes`), or default 4.

Persist any explicit `MAX_LANES` change to config:
```bash
mkdir -p .claude/agent-memory/project-manager
echo '{"max_lanes": <N>}' > .claude/agent-memory/project-manager/config.json
```

---

## Step 2 — Fetch current state

Run in parallel:

```bash
# Active lanes (in-progress issues)
gh issue list --label "in progress" --state open \
  --json number,title,labels --limit 50

# Open PRs (being worked on — may not have label yet)
gh pr list --state open --json number,headRefName,labels --limit 50

# Stalled pipeline issues needing resume
gh issue list --label "pipeline:open-pr" --label "ready" --state open \
  --json number,title,labels --limit 20
gh issue list --label "pipeline:validate" --label "ready" --state open \
  --json number,title,labels --limit 20
gh issue list --label "pipeline:merge" --label "ready" --state open \
  --json number,title,labels --limit 20
```

Compute:
- `IN_PROGRESS_NUMBERS` — issue numbers with "in progress" label
- `IN_PROGRESS_AREAS` — all `area:*` labels on those issues
- `ACTIVE_LANE_COUNT` — count of in-progress issues
- `OPEN_SLOTS` = max(0, `MAX_LANES` − `ACTIVE_LANE_COUNT`)
- `STALLED` — stalled pipeline issues

Print one status line:
```
[PM] <ACTIVE_LANE_COUNT>/<MAX_LANES> lanes active — areas in use: <area1>, <area2>
```

---

## Step 2b — Pane census + health check

Build `ACTIVE_PANE_ISSUES` from live pane titles (authoritative — GitHub labels lag):

```bash
PLEXI=plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL}
$PLEXI pane list 2>/dev/null | jq -r '.[].title // ""' | grep -oE '[0-9]+' || true
```

Note: the JSON field is `title`, not `name` — `.name` returns null and breaks the census.

**Cursor store:** Load per-pane cursors from `.claude/agent-memory/project-manager/pane-cursors.json` (create as `{}` if missing). For each active issue pane ID:

```bash
# First capture (no stored cursor):
$PLEXI pane capture --lines 10 <PANE_ID>

# Subsequent captures (cursor from prior run):
$PLEXI pane capture --from-cursor <CURSOR> <PANE_ID>
```

Each response is `{"cursor": N, "lines": [...], "missed": bool}`. Save the new `cursor` back to the store keyed by pane ID and persist to disk after each cycle. `lines: []` + `missed: false` = no new output.

**Noop exit detection:** If captured lines show the agent exited without doing real work (e.g. "Issue is already closed", "Nothing to do", "already implemented", "already merged"), close the pane immediately and free the slot:

```bash
$PLEXI pane close <PANE_ID>
```

Remove its cursor entry from the store. Any issue whose pane was NOT closed must be skipped in Step 2c and Step 4.

---

## Step 2c — Resume stalled pipeline issues (priority)

Stalled issues already have a PR or partial work — resume them before dispatching fresh ones. Each consumes one open slot. **Skip any issue whose number is in `ACTIVE_PANE_ISSUES`** — a pane is already handling it.

| Label | Dispatch |
|---|---|
| `pipeline:open-pr` + `ready` | **PR-exists check first** (below). If no PR → `/open-pr <branch>`. If PR exists → advance label and dispatch `/validate-pr <PR#>` instead. |
| `pipeline:validate` + `ready` | Detect PR#, dispatch `/validate-pr <PR#>` |
| `pipeline:merge` + `ready` | Detect PR#, dispatch `/merge-pr <PR#>` |

Detect PR# for any stage:
```bash
gh pr list --state open --json number,headRefName \
  | jq --arg n "<N>" '.[] | select(.headRefName | test("^feature/\($n)-")) | .number'
```

**PR-exists check for `pipeline:open-pr` issues (mandatory — prevents the re-open loop):**
An issue can carry `pipeline:open-pr` while already having an open PR — this happens when `open-pr` errored after `gh pr create` but before the label flip, or when it was interrupted. Dispatching `/open-pr` again would error and re-strand the issue every cycle. So before dispatching open-pr, look up the PR# with the query above:
- **No PR found** → dispatch `/open-pr <branch>` as normal.
- **PR found** → the issue is actually past open-pr. Advance it and dispatch validate instead:
  ```bash
  gh issue edit <N> --add-label "pipeline:validate" --remove-label "pipeline:open-pr"
  ```
  then dispatch `/validate-pr <PR#>`. Print: `[PM] #<N> already has PR #<PR#> — advanced to pipeline:validate.`

Subtract resumed issues from `OPEN_SLOTS`.

---

## Step 2d — Post-merge issue cleanup

Check for PRs merged to alpha recently whose linked issues are still open, and close them:

```bash
gh pr list --state merged --base alpha --json number,headRefName --limit 20 \
  | jq -r '.[] | select(.headRefName | test("^feature/[0-9]+-")) | [(.headRefName | split("/")[1] | split("-")[0]), (.number | tostring)] | join(" ")'
```

For each `ISSUE PR` pair: if `gh issue view $ISSUE --json state -q .state` == `OPEN`, close it:

```bash
gh issue close $ISSUE --comment "Closed — merged via PR #$PR."
```

Print: `[PM] Closed #<issue> — PR #<pr> already merged.`

---

## Step 3 — If OPEN_SLOTS = 0 or DISPATCH = false, skip to Step 5

All `MAX_LANES` lanes full, or wind-down mode active. Jump directly to the watch loop.

In wind-down mode, print:
```
[PM] Wind-down — no new lanes. <ACTIVE_LANE_COUNT> remaining. Loop ends when all drain to 0.
```

The watch loop ends when `DISPATCH=false` AND `ACTIVE_LANE_COUNT = 0`.

---

## Step 4 — Select candidates

Fetch ready issues with bundle flag, sorted newest-first.

**Critical:** `ready` is added at every pipeline handoff (implement→open-pr→validate→merge), so it does NOT mean "fresh, dispatch me" — it means "ready for the next stage." A genuinely fresh issue has `ready` AND **no** `pipeline:*` label. Issues carrying any `pipeline:*` label belong to Step 2c (stalled resume), not fresh dispatch — the jq below excludes them so they never leak into a fresh lane:

```bash
gh issue list --label "ready" --state open \
  --json number,title,labels --limit 100 \
  | jq '[.[] | select([.labels[].name | startswith("pipeline:")] | any | not)
        | {number, title, areas: [.labels[].name | select(startswith("area:"))], bundle: ([.labels[].name] | contains(["bundle"]))}] | sort_by(-.number)'
```

**Bundle batching (do this first before selecting individual issues):**

Group all `bundle: true` issues by their primary area (first `area:*` label). For each group that has 2+ issues and none of whose areas conflict with `IN_PROGRESS_AREAS`:
- Treat the entire group as ONE candidate consuming ONE lane slot.
- The single lane will implement all issues in the group in one PR.
- Dispatch as ONE pane: `$PLEXI terminal "c '/implement-issue <N1> <N2> <N3>'"`. Name the pane `#N1+N2+N3`.

After bundle groups consume their slots, fill remaining slots with individual non-bundle issues.

**For each non-bundle candidate (newest-first):**

Skip any issue that:
1. Is in `IN_PROGRESS_NUMBERS`
2. Has a `blocked` label
3. Has an open PR with `headRefName` matching `^feature/<N>-`
4. Body contains "Do not implement here" (epic tracker)
5. Number is in `ACTIVE_PANE_ISSUES` (pane already running)

**Area conflict check — run this explicitly for every candidate before selecting:**

Build `CONFLICT_AREAS` = union of:
- All `area:*` labels from every issue currently in `ACTIVE_PANE_ISSUES` (pull their labels from the issue list or a `gh issue view` call)
- All `area:*` labels from candidates already selected this cycle

For each candidate, check: does ANY of its `area:*` labels appear in `CONFLICT_AREAS`?
- If yes: **skip** — log why: `skip #1234 (area:cli/commands conflicts with #1231)`
- If no: **select** — add all its `area:*` labels to `CONFLICT_AREAS`, decrement `OPEN_SLOTS`

Example:
```
Active: #1803 (area:sdk/python, area:sdk/pgap, area:ui/widgets)
Candidate #1820: area:sdk/python → CONFLICT → skip
Candidate #1812: area:ui/tile-tree, area:host/config → no conflict → SELECT
  CONFLICT_AREAS now includes area:ui/tile-tree, area:host/config
Candidate #1811: area:ui/tile-tree → CONFLICT with #1812 → skip
Candidate #1805: area:cli/commands → no conflict → SELECT
```

Stop when `OPEN_SLOTS` reaches 0 or candidates are exhausted.

If `ORDER_MODE` = `scored`: invoke `/score` on the candidate list first, then walk in scored order instead of newest-first.

Print selection:
```
[PM] Dispatching <N>:
  #1234 — Title                   (area:cli)
  #1231 + #1229 + #1227 — bundle  (area:ui/overlays)
```

If no candidates:
```
[PM] Queue dry — <ACTIVE_LANE_COUNT>/4 lanes still running.
```
Skip dispatch, go to Step 5.

---

## Step 4b — Dispatch

Auto-push alpha if needed:
```bash
UNPUSHED=$(git log origin/alpha..HEAD --oneline 2>/dev/null | wc -l | tr -d ' ')
[ "$UNPUSHED" -gt 0 ] && git push origin alpha
```

Then open panes — one per lane. Use `split_h` for the first, `split_v` for each subsequent:

```bash
PLEXI=plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL}
PREV_ID=$PLEXI_PANE_ID
LAYOUT=split_h

# Individual issue:
PANE_ID=$($PLEXI terminal "c '/implement-issue <N>'" \
  --layout $LAYOUT --from-pane-id $PREV_ID --cwd "$REPO_DIR" --no-focus)
$PLEXI pane name $PANE_ID "#<N>"
PREV_ID=$PANE_ID; LAYOUT=split_v

# Bundle (all issues → one pane, one PR):
PANE_ID=$($PLEXI terminal "c '/implement-issue <N1> <N2> <N3>'" \
  --layout $LAYOUT --from-pane-id $PREV_ID --cwd "$REPO_DIR" --no-focus)
$PLEXI pane name $PANE_ID "#<N1>+<N2>+<N3>"
PREV_ID=$PANE_ID; LAYOUT=split_v
```

Fire non-blocking notify:
```bash
PLEXI=plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL}
$PLEXI notify \
  --title "PM: dispatched ${COUNT} lanes" \
  --body "#<N1>, #<N2>... (<ACTIVE_LANE_COUNT + COUNT>/<MAX_LANES> active)" \
  --choice "ok:Dismiss" &
```

---

## Step 5 — Watch loop

If `WATCH` = false, stop here.

Schedule a wake-up in **240 seconds** using `ScheduleWakeup` with prompt `/project-manager`. On each wake, the skill re-enters at Step 1 and runs the full cycle again.

Print:
```
[PM] Watching — next check in 4m. Say "stop watching" to end, "wind down" to drain without refilling.
```

The loop ends when:
- User says "stop" or "done watching"
- Queue is dry AND active lane count = 0 (nothing left to do)

On each re-entry after initial dispatch, lead with:
```
[PM] ♻ <HH:MM> — <N>/4 active, <M> ready in queue
```

---

## Notes

- **Pane census uses `.title`, not `.name`.** `.name` returns null — always use `jq -r '.[].title // ""'`.
- **Always use `--from-cursor` on repeat pane captures.** `plexi pane capture --from-cursor <N> <ID>` reads only lines written since the last check. Store cursors in `.claude/agent-memory/project-manager/pane-cursors.json`, keyed by pane ID. `lines: []` + `missed: false` = no new output.
- **Close noop panes immediately.** If an agent exits without doing real work, `pane close` right away and free the slot.
- **Bare-shell pane recovery.** If a dispatch pane shows a prompt with no agent running ("← for agents"), re-send the command: `$PLEXI pane send <ID> 'c "/ship-issue <N>"\n'` — use `\n` for Enter, never a trailing `Enter` argument.
- **Bundle issues should be batched.** Issues labeled `bundle` are micro-changes. Group by primary area and send all same-area bundles to one lane in one PR. Never open one PR per bundle issue.
- **Post-merge cleanup runs every cycle.** Check merged PRs for still-open linked issues and close them automatically.
- **Auto-push alpha before dispatch.** `$PLEXI terminal` will clone a dirty tree — push first.
- **Channel binary** auto-detected via `$PLEXI_CHANNEL` — never hardcode.
- **Newest-first is the default.** Higher issue numbers were filed more recently.
- **Conflict detection is live, not scored.** Two issues conflict only if they share an `area:*` label with something currently in-progress.
- **Scoring is opt-in.** Invoke `/score` before or say "score these first" to get conviction-ranked ordering.
- **pipeline:* labels are live state.** Never read Ship Log to determine current pipeline stage — check labels.
- **Stalled pipeline issues always take priority** — they already have partial work and should complete the cycle before fresh dispatch.
