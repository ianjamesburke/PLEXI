---
name: project-manager
description: Use when selecting and dispatching parallelizable issues from the GitHub backlog. Triggered by /project-manager, "what should we ship?", "pick issues to dispatch", or at session start when the board needs a fresh dispatch. Not for single-issue work — use /ship-issue for that.
---

# Project Manager

Conviction-scored parallel dispatch from the GitHub issue queue. Scores open issues 0–1, selects top 4 non-competing work units, and dispatches or surfaces for review.

---

## Pre-check

```bash
if [ -z "$PLEXI_PANE_ID" ]; then
  echo "ERROR: PLEXI_PANE_ID not set — /project-manager must run inside a Plexi pane." >&2
  exit 1
fi
```

Fail immediately if not inside a Plexi pane. Never skip this check.

---

## Conviction Weight Config

| Signal | Weight |
|---|---|
| Issue is in Up Next column | +0.30 |
| P0 priority label | +0.40 |
| P1 priority label | +0.30 |
| P2 priority label | +0.15 |
| Has at least one `area:*` label | +0.10 |
| Has acceptance criteria (Done When / `- [ ]` checklist) | +0.10 |
| Has `## Prior Attempts` in body | −0.20 |
| Shares `area:*` label with a higher-ranked candidate | −0.25 |

Cap: 0.0 min, 1.0 max. P3/P4 issues with no other signals score 0.00.

---

## Step 1 — Load conviction cache

```bash
CACHE=".claude/agent-memory/project-manager/MEMORY.md"
```

If the file exists, read it. Extract the `## Scored Issues` table — each row is `| #<N> | <score> | ... |`. These are cache hits for this session.

**Cache hit rule:** If an issue number appears in the cache with a `Last run` timestamp from the current date, skip re-scoring it — use the cached score and reasoning directly.

If the file does not exist or is from a previous date: treat all candidates as cache misses and re-score in Step 3.

---

## Step 2 — Fetch candidates

### Primary: Up Next column

```bash
gh project item-list 7 --owner ianjamesburke --format json \
  | jq '[.items[] | select(.status == "Up Next") | {number: .content.number, title: .title, labels: [.labels // [] | .[].name]}]'
```

### Fallback: `ready` label (if Up Next is empty or has < 4 issues)

```bash
gh issue list --label "ready" --state open \
  --json number,title,labels,body --limit 100
```

### Exclusion filter — for each candidate, skip if any of:

1. Has `in progress` label
2. Has `blocked` label
3. Has an open PR referencing this issue:
   ```bash
   gh pr list --state open --json headRefName \
     | jq --arg n "<N>" '[.[] | select(.headRefName | test("feature/\($n)-"))] | length > 0'
   ```
4. Body contains "Do not implement here" (epic tracker)

Collect the survivors as the **candidate list**.

---

## Step 3 — Score each candidate

For each candidate not in the cache:

**Read the issue body** (required for AC and prior-attempts signals):
```bash
gh issue view <N> --json number,title,labels,body
```

Apply the weight table from the config:

```
score = 0.0
score += 0.30  if issue is in Up Next list
score += 0.40 / 0.30 / 0.15  for P0 / P1 / P2 (pick at most one)
score += 0.10  if any label matches area:*
score += 0.10  if body contains "## Done When" or "- [ ]" checklist
score -= 0.20  if body contains "## Prior Attempts"
# area conflict penalty applied in Step 4 after ranking
```

Record per-issue reasoning as a breakdown string, e.g.:
`"Up Next +0.30, P1 +0.30, area +0.10, AC +0.10 = 0.80"`

---

## Step 4 — Select top 4 non-competing

1. Sort all scored candidates by score descending, then issue number ascending (tiebreak).
2. Walk the sorted list. For each candidate:
   - If it shares an `area:*` label with any already-selected candidate: apply −0.25 penalty to its score and skip it (it conflicts — queue behind the selected one instead).
   - Otherwise: add to the selected set.
3. Stop when 4 are selected or the list is exhausted.

**Result:** up to 4 issues with no shared `area:*` labels.

---

## Step 5 — Decision gate

Print the review table regardless of path:

```
#<N>   <score>  <title>
       Breakdown: <reasoning>
```

**Auto-dispatch path** — all selected scores ≥ 0.5:
```
All scores ≥ 0.5 — auto-dispatching 4 lanes.
```
Proceed to Step 6 immediately.

**Review path** — any selected score < 0.5:
```
One or more candidates score below 0.5. Review:

#<N>  0.42  <title>  [LOW CONFIDENCE — AC missing, no area label]

Dispatch anyway? (y/N)
```
Wait for explicit `y` before proceeding to Step 6. Any other input aborts — write memory (Step 7) but do not dispatch.

**`--dry-run` flag:** If invoked as `/project-manager --dry-run`, print the table and stop. Skip Step 6 (dispatch), skip `plexi notify`, skip Step 7 (memory write). Output:
```
[DRY RUN] Would dispatch: #<N1> #<N2> #<N3> #<N4>
```

---

## Step 6 — Dispatch

Run from the repo root:

```bash
bash .claude/skills/dispatch/scripts/open-lanes.sh <N1> [N2] [N3] [N4]
```

The script handles all pane mechanics (channel detection, split layout, naming, sending ship command). Do not re-implement it.

After lanes open, fire a non-blocking notify:

```bash
PLEXI=plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL}
$PLEXI notify \
  --title "Dispatched ${COUNT} lanes" \
  --body "#<N1>, #<N2>... — conviction scores: <score1>, <score2>..." \
  --choice "ok:Dismiss" &
```

---

## Step 7 — Write conviction cache

Create `.claude/agent-memory/project-manager/` if absent. Write (overwrite) `MEMORY.md`:

```markdown
# Project Manager — Conviction Cache

Last run: <ISO timestamp>

## Scored Issues

| Issue | Score | Breakdown | Dispatched |
|-------|-------|-----------|------------|
| #<N> | 0.80 | Up Next +0.30, P1 +0.30, area +0.10, AC +0.10 | yes |
| #<N> | 0.45 | P2 +0.15, area +0.10, AC +0.10, Prior Attempts −0.20, conflict −0.25 | no |
```

Include every candidate that was scored this run (dispatched or not). The cache is session-scoped — stale entries from previous dates are ignored by Step 1, but preserved in the file for audit.

---

## Notes

- **open-lanes.sh requires alpha to be clean.** It checks `git status --porcelain` and aborts with an error if dirty. Fix alpha state before invoking.
- **Channel binary** is auto-detected by open-lanes.sh via `$PLEXI_CHANNEL`. Never hardcode a channel name.
- **Up Next column** is the primary source because issues staged there have already been reviewed for dispatch-readiness. The `ready` fallback is for sessions where the board hasn't been curated.
- **Area conflict penalty (−0.25)** is applied during selection (Step 4), not during scoring (Step 3). This means a P1 issue that conflicts with a P0 still shows its raw score in the table but is excluded from the selected set.
- **Second invocation same session:** cache hits from Step 1 skip the `gh issue view` fetch in Step 3, making the second run faster. The cache does not expire within a session day.
