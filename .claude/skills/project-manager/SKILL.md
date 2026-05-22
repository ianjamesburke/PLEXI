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
     | jq --arg n "<N>" '[.[] | select(.headRefName | test("^feature/\($n)-"))] | length > 0'
   ```
4. Body contains "Do not implement here" (epic tracker)

Collect the survivors as the **candidate list**.

---

## Step 2b — Resume stalled pipeline issues

Before scoring fresh candidates, check for issues stuck mid-pipeline. These are dispatched with priority over new work — they already have a PR and should complete the cycle.

```bash
for LABEL in "pipeline:implement" "pipeline:open-pr" "pipeline:validate" "pipeline:merge"; do
  gh issue list --label "$LABEL" --label "ready" --state open \
    --json number,title,labels --limit 20
done
```

For each result, map label → skill and dispatch immediately (skip the scoring/selection steps):

| Label found | Skill to dispatch | Notes |
|---|---|---|
| `pipeline:implement` + `ready` | `/implement-issue <N>` | Re-implement (prior attempt failed) |
| `pipeline:open-pr` + `ready` | `/open-pr feature/<N>-...` | Detect branch via `git ls-remote origin "refs/heads/feature/<N>-*"` |
| `pipeline:validate` + `ready` | `/validate-pr <PR#>` | Detect PR# via `gh pr list --json number,headRefName` |
| `pipeline:merge` + `ready` | `/merge-pr <PR#>` | Detect PR# via `gh pr list --json number,headRefName` |

**Detecting the PR number for validate/merge dispatch:**
```bash
gh pr list --state open --json number,headRefName \
  | jq --arg n "<ISSUE_NUMBER>" \
    '.[] | select(.headRefName | test("^feature/\($n)-")) | .number'
```

**Testing notification ownership for validate dispatch:** When dispatching `/validate-pr`, inject `PM_PANE_ID=$PLEXI_PANE_ID` into the worker pane's environment so the pass/fail notification routes back to the PM pane. The `open-lanes.sh` script passes env vars via the command prefix — prepend `PM_PANE_ID=$PLEXI_PANE_ID` before the `c` command.

If any pipeline-labeled issues are found and dispatched, subtract the number of resumed lanes from the 4-lane capacity and continue to Step 3 to fill remaining slots with fresh issues. Example: 2 resumed lanes → capacity for 2 more from scoring. If resumed lanes already fill all 4 slots, end the run after dispatch.

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
score += 0.40 / 0.30 / 0.15  for P0 / P1 / P2 (pick at most one)
score += 0.10  if any label matches area:*
score += 0.10  if body contains "## Done When" or "- [ ]" checklist
score -= 0.20  if body contains "## Prior Attempts"
# area conflict penalty applied in Step 4 after ranking
```

Record per-issue reasoning as a breakdown string, e.g.:
`"P1 +0.30, area +0.10, AC +0.10 = 0.50"`

---

## Step 4 — Select top 4 non-competing

1. Sort all scored candidates by score descending, then issue number ascending (tiebreak).
2. Walk the sorted list. For each candidate:
   - If it shares an `area:*` label with any already-selected candidate: record a −0.25 conflict penalty in its breakdown, subtract it from its displayed score, and skip it (it conflicts — queue behind the selected one instead).
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
| #<N> | 0.50 | P1 +0.30, area +0.10, AC +0.10 | yes |
| #<N> | 0.45 | P2 +0.15, area +0.10, AC +0.10, Prior Attempts −0.20, conflict −0.25 | no |
```

Include every candidate that was scored this run (dispatched or not). The cache is session-scoped — stale entries from previous dates are ignored by Step 1, but preserved in the file for audit.

---

## Notes

- **open-lanes.sh requires alpha to be clean.** It checks `git status --porcelain` and aborts with an error if dirty. Fix alpha state before invoking.
- **Channel binary** is auto-detected by open-lanes.sh via `$PLEXI_CHANNEL`. Never hardcode a channel name.
- **Area conflict penalty (−0.25)** is applied during selection (Step 4), not during scoring (Step 3). A conflicting P1 shows its penalized score (e.g. 0.55 = 0.80 raw − 0.25) in the review table and is excluded from the selected set. The penalty appears in the breakdown column for audit.
- **Second invocation same session:** cache hits from Step 1 skip the `gh issue view` fetch in Step 3, making the second run faster. The cache does not expire within a session day.
- **Labels are the live state; Ship Log is audit trail only.** `pipeline:*` labels answer "where is this issue in the pipeline right now." The Ship Log records what happened and why. Never read the Ship Log to determine current stage — always check labels. This is the canonical boundary, enforced in all pipeline skills.
- **pipeline:* labels on stalled issues:** If a skill crashes or exits mid-cycle, the `pipeline:*` label remains on the issue. PM Step 2b will detect and re-dispatch it on the next run. This is the self-healing recovery path — no manual intervention needed for crashed worker panes.
