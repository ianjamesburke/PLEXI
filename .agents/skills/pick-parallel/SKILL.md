---
name: pick-parallel
description: Use when the dispatch queue is empty, Up Next has no issues, or you need to select the next batch of parallelizable issues without a pre-built NEXT.md. Triggered by "what should we dispatch?", "nothing in Up Next", "pick 4 issues", or at session start when board has no staged work.
---

# Pick Parallel Issues

Selects 3–4 parallelizable issues from the full backlog and writes them to NEXT.md. NEXT.md is the handoff to `/dispatch` — once written, the dispatch skill reads it instead of re-querying GitHub.

## Step 1 — Check NEXT.md cache

```bash
cat NEXT.md 2>/dev/null | head -5
```

If NEXT.md exists and the `<!-- computed:` timestamp is < 4 hours old, show it to the user and ask: use as-is or recompute? Skip to "Output" if reusing.

## Step 2 — Fetch candidates

```bash
gh issue list --repo ianjamesburke/PLEXI --state open --limit 100 \
  --json number,title,labels \
  | jq '[.[] | select(
      (.labels[].name | test("P0|P1|P2")) and
      (.labels[].name | test("blocked|in-progress") | not)
    )] | sort_by(.labels[] | select(.name | test("^P[0-9]")) | .name)'
```

## Step 3 — Classify and group

For each issue, extract:
- **priority**: first `P0`–`P4` label (default `P9`)
- **area**: first `area:*` label (null if none)
- **is_bundle**: body contains `- [ ] #` checklist linking related issues

Group by area. Sort each area group by priority ASC, then number ASC. Issues with no `area:*` label go into a separate "Needs area label" list — surface them but don't assign to a lane.

## Step 4 — Select top 4 lanes

Pick the 4 areas whose lead issue has the lowest priority number (P0 beats P1, etc.). Within ties, lower issue number wins.

**Parallelizability check:** Two issues conflict if they share the same `area:*` label. Same-area issues must queue behind each other (not run in parallel). Different areas are safe to run in parallel.

**Blocking check:** For any P0 or P1 candidate, run:
```bash
gh issue view <N> --json body,labels | jq '{body: .body, labels: [.labels[].name]}'
```
Look for `blocked by #N` in the body, or a `blocked` label. Exclude blocked issues; note what they're waiting on.

Issues marked `in-progress` (active worktree exists): skip.

## Step 5 — Write NEXT.md

```markdown
# NEXT.md
<!-- computed: 2026-05-21T14:30:00Z -->

## Lanes
| Lane | Area | Lead | Queue |
|------|------|------|-------|
| A | area:host/permissions | #1596 | — |
| B | area:cli/commands | #1530 | #1612 |
| C | area:assets | #1543 | — |
| D | area:ui/overlays | #1601 | — |

## Deferred (beyond 4-lane cap)
- **area:host/input**: #1598 — blocked by #1596

## Needs area label
- #NNNN — title [P1 ⚠]

## Parallelizability notes
- #1598, #1599, #1601 blocked by #1596 (CapabilityModal) — dispatch after #1596 merges
- #1530 and #1612 both touch area:cli/commands — queue #1612 behind #1530
```

Overwrite any existing NEXT.md.

## Output

Print the lane table and the one-line reasoning for each pick. Format:

```
#1543 (P0) — Dock icon stale logo — area:assets — isolated build/asset fix
#1596 (P1) — CapabilityModal FocusLayer — area:host/permissions — prior attempts documented, action plan pinned
#1530 (P1) — plexi open github:owner/repo — area:cli/commands — no prior attempts, well-specified
#1612 (P2) — app init no-workspace fallback — area:cli/commands — queued behind #1530 (same area)
```

Then ask: "dispatch these? Run `/dispatch NEXT.md` or adjust."

## Notes

- NEXT.md is the cache. Next session reads it instead of re-querying. Re-run `/pick-parallel` only when it goes stale or issues close.
- The `blocked` label check in Step 2 is a quick filter. The per-issue body check in Step 4 catches dependencies not yet labeled.
- P0/P1 issues with no `area:*` label should be triaged before dispatch. Flag them; don't silently drop them.
- After any lane merges, NEXT.md is stale — rerun to advance the queue.
