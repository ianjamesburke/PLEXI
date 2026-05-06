---
name: sprint-plan
description: "Batch scan all open PLEXI issues and produce two outputs: (1) a clarification sweep — every issue with open questions that must be answered before it can be picked up, and (2) a prioritized execution plan with parallel lanes based on file-touch overlap. Read-only — no GitHub writes. Use when you want a complete picture of what's ready, what's blocked, and what can run in parallel."
risk: low
source: local
---

# Sprint Plan

Produce a complete, prioritized, parallelized view of all open PLEXI issues. Read-only output — nothing is written back to GitHub.

---

## Step 1 — Fetch All Open Issues

```bash
gh issue list --state open --json number,title,labels,body --limit 200
```

Parse each issue for front matter fields. A well-triaged issue has:

```yaml
---
touches: [src/app/mod.rs, sdk/python/]
clarification_needed: []
---
```

For issues missing front matter entirely, treat both fields as unknown — flag them in the output as **untriaged**.

**Dependencies use native GitHub blocking relations**, not front matter. Query them per-issue in Step 2.

---

## Step 2 — Dep Unblocking Pass

Before planning, check whether any `blocked` issues can be unblocked now:

For each issue labeled `blocked`:
1. Query native blocking relations:
   ```bash
   gh issue-ext blocking list <number>
   ```
2. Parse the "Blocked by" list from the output. For each blocker, check if its state is `CLOSED`.
3. If **all** blockers are `CLOSED`: remove `blocked`, add `ready`:
   ```bash
   gh issue edit <n> --remove-label "blocked" --add-label "ready"
   ```
   Report these as `[UNBLOCKED]` in the output.

This is the only write this skill performs.

---

## Step 3 — Clarification Sweep

Scan every open issue. Flag it in this pass if **any** of the following are true:
- `clarification_needed` is non-empty
- The issue has no front matter at all (untriaged)
- The issue is not labeled `ready`, `blocked`, or `in progress` and has no `clarification_needed` field (labeling gap)

Output format:

```
── CLARIFICATION NEEDED ──────────────────────────────

#542 — New windows don't inherit working dir [P2, bug]
  ? Should this apply to splits, new contexts, or both?
  ? What is the expected CWD when the source pane has no active process?

#517 — Video decode capability [P1, enhancement]
  ⚠ UNTRIAGED — no front matter. Run /triage-issues 517 first.

#113 — Parallax viewer [P2, enhancement, backlog]
  ⚠ LABELING GAP — has no ready/blocked/in-progress status and no clarification questions recorded.
```

If no issues need clarification: output `── No clarification needed. All open issues are actionable. ──`

---

## Step 4 — Build the Dependency Graph

For each open issue, collect:
- `number`
- `priority` (P0–P4, extracted from labels)
- `blockers` (from `gh issue-ext blocking list <number>`, empty list if none)
- `touches` (from front matter, empty list if absent)
- `status` (`ready` / `blocked` / `in progress` / unlabeled)

Filter to **only `ready` and `in progress` issues** for the execution plan — blocked issues cannot be scheduled.

Topological sort:
1. Issues with no open blockers are roots — they can start immediately
2. For each root, children become schedulable once their parent is done
3. Within the same dependency level, sort by priority (P0 first)

---

## Step 5 — Compute Parallel Lanes

Two issues can run in parallel if their `touches` sets are **disjoint** (no shared file or directory prefix).

Conflict check: issue A and issue B conflict if any path in A's `touches` is equal to or a prefix of any path in B's `touches`, or vice versa.

Example:
- A touches `[src/app/mod.rs, sdk/python/]`
- B touches `[src/style.rs, src/widgets.rs]`
- → No overlap → parallel-safe

- A touches `[src/app/mod.rs]`
- C touches `[src/app/]`
- → `src/app/mod.rs` is under `src/app/` → conflict → sequential

Issues with empty `touches` are treated as **unknown overlap** — assume they conflict with everything and assign to a single-issue lane.

Group issues into lanes: a lane is a set of issues that can all be worked simultaneously. Use a greedy bin-packing approach — assign each issue to the first lane it doesn't conflict with.

---

## Step 6 — Output the Execution Plan

```
── EXECUTION PLAN ────────────────────────────────────
Priority order within each level. Lanes are parallel-safe.

▶ LEVEL 1 (no dependencies — start immediately)

  Lane A
  ├── #635 [P2] feat(triage): triage-issues skill + sprint-plan batch skill
  │         touches: [.claude/skills/]
  └── #486 [P2] feat(updater): once-a-day cached update check with toolbar badge
            touches: [src/cli.rs, src/app/mod.rs]

  Lane B
  └── #542 [P2, bug] New windows don't inherit working dir
            touches: [src/tiling.rs, src/app/mod.rs]  ← conflicts with Lane A #486

  Lane C
  └── #413 [P2] feat(tiling): keyboard pane swap + animated transitions
            touches: [src/tiling.rs]  ← conflicts with Lane B

▶ LEVEL 2 (unblocked after level 1 completes)

  Lane A
  └── #625 [P2] arch(sdk): Rust-owned canonical PGAP schema
            touches: [src/, sdk/python/]
            blocked by: none (ready now — shown here for sequencing context only)

── SUMMARY ───────────────────────────────────────────
  Ready to start:    8 issues across 3 parallel lanes
  In progress:       1 issue
  Blocked:           2 issues (deps still open)
  Untriaged:         3 issues (run /triage-issues on each)
  Need clarification: 4 issues
```

**Notes on the output:**
- Issues with `in progress` label are shown at the top of their lane with a `▶` marker
- If `touches` is empty for an issue, note it as `touches: unknown` and place it in its own lane
- The plan is a point-in-time snapshot — re-run after merges or triage updates

---

## Step 7 — Triage Recommendations

After the plan, append a short action list:

```
── RECOMMENDED ACTIONS ───────────────────────────────

1. Answer clarification questions on #542 and #517 (see sweep above) — blocks scheduling
2. Run /triage-issues on untriaged issues: #113, #517, #291
3. Issues #513 and #541 are labeled P1 but have no ready/blocked status — verify or label blocked
```

Keep this list to ≤ 5 items. The most important action is always first.
