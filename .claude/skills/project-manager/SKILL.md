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
```

---

## Step 1 — Parse user context

Read what the user said before fetching anything. Extract signals:

| Signal | Behaviour |
|---|---|
| "just added issues" / "new issues" / "just filed" | Newest-first, skip scoring |
| "score" / "prioritize" | Invoke `/score` skill first, then use scored order |
| "stop" / "done watching" | Cancel loop — do not call ScheduleWakeup |
| Number (e.g. "run 6 lanes", "use 3") | Set `MAX_LANES` to that number |
| No signal | Default: newest-first, watch loop on |

Set `ORDER_MODE` = `newest-first` (default) or `scored`.
Set `WATCH` = true (default) or false.
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

## Step 2b — Resume stalled pipeline issues (priority)

Stalled issues already have a PR or partial work — resume them before dispatching fresh ones. Each consumes one open slot.

| Label | Dispatch |
|---|---|
| `pipeline:open-pr` + `ready` | Detect branch: `git ls-remote origin "refs/heads/feature/<N>-*"` → `/open-pr <branch>` |
| `pipeline:validate` + `ready` | Detect PR#, dispatch `/validate-pr <PR#>` |
| `pipeline:merge` + `ready` | Detect PR#, dispatch `/merge-pr <PR#>` |

Detect PR# for validate/merge:
```bash
gh pr list --state open --json number,headRefName \
  | jq --arg n "<N>" '.[] | select(.headRefName | test("^feature/\($n)-")) | .number'
```

Subtract resumed issues from `OPEN_SLOTS`.

---

## Step 3 — If OPEN_SLOTS = 0, skip to Step 5

All `MAX_LANES` lanes full. Jump directly to the watch loop.

---

## Step 4 — Select candidates

Fetch ready issues sorted newest-first (highest number first):

```bash
gh issue list --label "ready" --state open \
  --json number,title,labels --limit 100 \
  | jq 'sort_by(-.number)'
```

Skip any issue that:
1. Is in `IN_PROGRESS_NUMBERS`
2. Has a `blocked` label
3. Has an open PR with `headRefName` matching `^feature/<N>-`
4. Body contains "Do not implement here" (epic tracker)

Walk remaining candidates in order. For each:
- If any of its `area:*` labels appear in `IN_PROGRESS_AREAS` or a previously selected candidate's areas: **skip** (parallel conflict).
- Otherwise: select it, add its areas to the conflict set, decrement `OPEN_SLOTS`.
- Stop when `OPEN_SLOTS` reaches 0 or candidates are exhausted.

If `ORDER_MODE` = `scored`: invoke `/score` on the candidate list first, then walk in scored order instead of newest-first.

Print selection:
```
[PM] Dispatching <N>:
  #1234 — Title          (area:cli)
  #1231 — Title          (area:sdk)
```

If no candidates:
```
[PM] Queue dry — <ACTIVE_LANE_COUNT>/4 lanes still running.
```
Skip dispatch, go to Step 5.

---

## Step 4b — Dispatch

```bash
bash .claude/skills/dispatch/scripts/open-lanes.sh <N1> [N2] [N3] [N4]
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

Schedule a wake-up in **90 seconds** using `ScheduleWakeup` with prompt `/project-manager`. On each wake, the skill re-enters at Step 1 and runs the full cycle again.

Print:
```
[PM] Watching — next check in 90s. Say "stop watching" to end.
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

- **open-lanes.sh requires alpha clean.** Aborts if `git status --porcelain` is dirty.
- **Channel binary** auto-detected via `$PLEXI_CHANNEL` — never hardcode.
- **Newest-first is the default.** Higher issue numbers were filed more recently. Right for "I just added a bunch of issues."
- **Conflict detection is live, not scored.** Two issues conflict only if they share an `area:*` label with something currently in-progress.
- **Scoring is opt-in.** Invoke `/score` before or say "score these first" to get conviction-ranked ordering.
- **pipeline:* labels are live state.** Never read Ship Log to determine current pipeline stage — check labels.
- **Stalled pipeline issues always take priority** — they already have partial work and should complete the cycle before fresh dispatch.
