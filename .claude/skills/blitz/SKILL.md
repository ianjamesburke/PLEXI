---
name: blitz
description: "Persistent roadmap executor. Reads ROADMAP.md + .claude/state/blitz.json, fans out parallelizable issues to sub-agents on worktrees, tracks progress across context resets. Invoke with /blitz to continue where you left off. /blitz reset to start fresh. /blitz status to see progress. /blitz feedback to record notes."
---

# Blitz

Persistent roadmap executor. Keeps the orchestrator context clean by delegating all implementation to sub-agents. State survives context resets via `.claude/state/blitz.json`.

## Mental Model

You are a **dispatcher**, not an implementer. You never read issue bodies in detail, never grep source code, never write production code. You read the state file, decide what to fan out next, spawn sub-agents, collect results, update state, and report.

**You are proactive.** You never ask "want me to dispatch?" or "should I fan out more work?" You just do it. The user invoked `/blitz` because they want maximum throughput. Your job is to keep 4 lanes saturated at all times. The only time you stop dispatching is when `spot_check_pending` is true (a merge round just completed and the user hasn't passed/failed yet).

**Continuous dispatch, not batch dispatch.** The old model was: fill 4 lanes, wait for all 4, merge all, spot check, repeat. The new model is: fill 4 lanes, as each lane completes immediately merge its PR and backfill the slot with the next issue, spot check when a configurable number of PRs have merged (default: 4). Lanes are independent pipelines, not synchronized batches.

---

## Step 0 -- Load State

```bash
cat .claude/state/blitz.json 2>/dev/null || echo "NO_STATE"
```

If `NO_STATE` or user said `/blitz reset`:
- Read `ROADMAP.md` to populate initial state
- Write fresh state file (see State Schema below)
- Then proceed to Step 1

If state exists: parse it and proceed to Step 1.

---

## Step 1 -- Assess Current Layer

From state, identify `current_layer` (the active phase). Check:

1. **In-flight issues** -- any branches/PRs still open from a prior session?
   ```bash
   for ISSUE in <in_flight_issues>; do
     gh pr list --search "head:feature/$ISSUE" --state open --json number,title,mergeable --jq '.[0]'
   done
   ```
   - If PR exists and mergeable: merge it (invoke `/merge-pr` inline)
   - If PR exists but failing: note as `blocked` in state, skip it this round
   - If branch exists but no PR: resume from `/open-pr` phase

2. **Completed count** -- if all issues in current layer are done, advance to next layer and trigger QA (Step 5b)

3. **Remaining issues** -- the dispatch candidates

Report current state to user in this format, then **immediately proceed to Step 2** (no pause, no question):
```
BLITZ -- Layer <N>: <name>
Completed: <X>/<total> issues
In-flight: #1234 (PR #56), #5678 (branch only)
Remaining: #9012, #3456, #7890
Blocked: #1111 (depends on #2222)
```

---

## Step 2 -- Fill Open Lanes (Continuous)

Count active lanes (issues with status `in_flight`). If fewer than 4, **immediately** fill the empty slots. Do not present candidates. Do not ask permission. Just dispatch.

**Lane selection priority:**
1. Remaining issues in the current layer (same layer first)
2. If current layer has fewer remaining than open slots, pull from the next layer (cross-layer dispatch is fine for independent work)
3. Skip any issue with `blocked` label or `depends_on` referencing an open issue

**Parallelizability rules:**
- Different `area:*` labels = safe to parallelize
- Same area = must serialize (queue the later one)
- Any issue with `blocked` label or `depends_on` referencing an open issue = skip
- Bundle-labeled issues go together in one lane
- Never dispatch an issue that conflicts with an already in-flight lane (same area)

**Model selection for sub-agents:**
- `load:S` or `bundle` label = Sonnet sub-agent
- `load:M` or `load:L` or no load label = Opus sub-agent
- Issues touching `src/` (Rust host code) = always Opus

**Output format (informational, not a proposal):**
```
DISPATCHING:
  Lane A: #1546 (S, Sonnet) -- plexi -h crashes
  Lane B: #1547 (M, Opus) -- pane focus misbehaves
  Lane C: #1549 (S, Sonnet) -- strip trailing punctuation
```

This is a notification, not a question. The agents are already being spawned.

---

## Step 3 -- Fan Out Sub-Agents

For each issue in the batch, spawn a sub-agent using the Agent tool. Each agent gets a self-contained prompt that includes:

1. The issue number and title
2. The instruction to run `/implement-issue <N>` which handles the full pipeline
3. The worktree path it should use
4. The rule: implement, open-pr, stop (do NOT merge)

**Sub-agent prompt template:**

```
You are implementing a single issue for the PLEXI project.

ISSUE: #<N> -- <title>
BRANCH: feature/<N>-<slug>

INSTRUCTIONS:
1. Run: gh issue view <N> --json title,body,labels
2. Read the issue fully -- it contains an Implementation Map with exact files to change
3. Create worktree: wtp add -b feature/<N>-<slug> origin/alpha
4. Verify worktree base matches alpha HEAD
5. Implement the fix in the worktree (all edits go to worktrees/feature/<N>-<slug>/)
6. Run: cargo build --manifest-path worktrees/feature/<N>-<slug>/Cargo.toml
   OR for Python-only: python3 -c "import ast; ast.parse(open('<file>').read())" for each changed file
7. Commit and push: git -C worktrees/feature/<N>-<slug> add . && git -C ... commit -m "<msg>" && git -C ... push -u origin HEAD
8. Create PR: gh pr create --base alpha --head feature/<N>-<slug> --title "<title> (#<N>)" --body "Closes #<N>\n\n## Summary\n<bullets>"
9. Report: PR_URL=<url> or BLOCKED=<reason>

RULES:
- All file paths must be absolute, targeting the worktree
- Never touch alpha directly
- cargo build must pass (or python syntax check for Python-only changes)
- Do NOT merge the PR -- just create it and report back
- If blocked or confused, report BLOCKED with reason -- do not guess
- NEVER touch apps/dev/ -- dev apps are throwaway POCs, not maintained. Only Core 9 apps get work. If an issue references a dev app, report BLOCKED="dev app, skip per policy"
```

**Spawning:**

Use the Agent tool with:
- `model: "sonnet"` for S-load issues
- `model: "opus"` for M/L-load issues
- `mode: "auto"` for all

Spawn all lanes in a single message (parallel Agent calls). Name each agent `blitz-<issue-number>`.

---

## Step 4 -- Collect Results, Merge, Backfill

When a sub-agent completes, handle it **immediately** (don't wait for other lanes):

1. Parse the result:
   - `PR_URL` present: merge the PR inline (squash merge, same as Step 6)
   - `BLOCKED`: mark as `blocked` in state with reason
   - Agent errored: mark as `failed` with error summary

2. Update `.claude/state/blitz.json` immediately.

3. **Backfill the lane.** If `spot_check_pending` is false and there are remaining issues, go directly to Step 2 to fill the open slot. This happens in the same turn, not after waiting.

4. **Spot check trigger.** After every `spot_check_interval` merges (tracked in state as `merges_since_spot_check`, default 4), set `spot_check_pending = true` and go to Step 5. Until that threshold, keep merging and backfilling.

The goal: lanes are always full. A lane finishing is a trigger to do more work, not a trigger to wait.

---

## Step 5 -- Spot Check Gate (periodic, not per-merge)

Spot checks happen after every N merges (default 4), not after every single merge. Between spot checks, the orchestrator keeps merging and dispatching at full speed.

**Trigger:** `merges_since_spot_check >= spot_check_interval` (default 4).

1. Set `spot_check_pending = true` in state. **This is the ONLY thing that pauses dispatch.**

2. Build and install:
   ```bash
   just bump && just install 2>&1 | tail -8
   ```

3. Present the spot-check prompt with **actionable test instructions** for each merged PR. Don't just list PR titles. For each PR, extract what the user should actually look for in the alpha build:

   ```
   SPOT CHECK -- v<version> installed
   Merged since last check: 4 PRs

   TEST:
   1. PR #1939 (QuickNote click-dismiss) -- open QuickNote with Cmd+0, click outside it, confirm it dismisses
   2. PR #1940 (Auto pane title) -- open a new terminal, run `ls`, confirm pane title updates to show the command
   3. PR #1941 (SDK error messages) -- open an app with a bad manifest, confirm error message names the field
   4. PR #1942 (Remove Quick Note app) -- run `plexi-alpha app list`, confirm quick-note is gone

   pass / fail / feedback?
   ```

   **How to write test instructions:** Each line is one sentence describing what to do and what to observe. Use the PR title and commit message to infer the user-visible behavior. If you can't infer a concrete test (e.g. internal refactor), write "internal change, no user-visible test". Keep it to 1-2 lines per PR max.

4. **HALT.** Do not dispatch more work while `spot_check_pending` is true. In-flight lanes continue running (don't kill them), but no new lanes are started and no PRs are merged.

5. On user response:
   - **Pass:** set `spot_check_pending = false`, reset `merges_since_spot_check = 0`, record in history, immediately fill all open lanes (Step 2)
   - **Fail:** record failure in `state.feedback[]`, mark the responsible issue as `needs_rework`, keep `spot_check_pending = true`
   - **Feedback (not pass/fail):** record in `state.feedback[]`, set `spot_check_pending = false`, resume dispatch

---

## Step 5b -- Layer QA Pass (when layer complete)

When all issues in a layer reach `done` status, this is the big gate:

1. Create a combined diff of all changes since the layer started:
   ```bash
   git log --oneline <layer_start_sha>..HEAD
   git diff <layer_start_sha>..HEAD --stat
   ```

2. Build and install:
   ```bash
   just bump && just install 2>&1 | tail -8
   ```

3. Report to user:
   ```
   LAYER <N> COMPLETE -- FULL QA
   Issues closed: #1546, #1547, #1549, #1550, #1601
   PRs merged: #1900, #1901, #1902, #1903, #1904
   Build: PASS
   Version: v<x.y.z>

   This is the layer gate. Please do a thorough spot-check of:
   <list of user-visible changes from this layer>

   Pass/fail/feedback?
   ```

4. **HALT.** Do not advance to next layer without explicit "pass" from user.

5. On pass: update state, advance `current_layer`, record `layer_start_sha` as current HEAD.

---

## Step 6 -- Inline Merge (called from Step 4)

Merging happens immediately when a lane's PR is ready, not as a separate batch step.

```bash
gh pr view <PR_NUMBER> --json state,mergeable,statusCheckRollup
```

- If mergeable and checks pass: squash merge
  ```bash
  gh pr merge <PR_NUMBER> --squash
  git fetch origin && git reset --hard origin/alpha
  ```
  Increment `merges_since_spot_check`. Mark issue as `done` in state.
- If checks failing: mark `blocked` in state, report to user, backfill the lane with next issue
- **Do NOT bump-and-install here.** That happens at spot check time only.

---

## Step 7 -- Record Feedback

User feedback is the most valuable signal. It persists across context resets in the state file.

**Trigger:** user says `/blitz feedback <text>`, or provides feedback during a spot check, or says anything corrective during the session.

**Write to state:**
```json
{
  "feedback": [
    {
      "timestamp": "2026-06-03T14:30:00Z",
      "layer": 3,
      "context": "spot-check after PR #1905 merge",
      "text": "Button click area is too small on the calc app footer",
      "resolution": null,
      "affects_issues": ["1527"]
    }
  ]
}
```

**Sub-agents receive relevant feedback.** When dispatching a sub-agent, check `state.feedback[]` for entries whose `affects_issues` overlap with the issue being dispatched. Include those in the sub-agent prompt as `PRIOR FEEDBACK FROM USER:` block.

**Resolution tracking:** When a subsequent PR addresses the feedback, update `resolution` to the PR number. Unresolved feedback is always surfaced at spot-check time.

**Feedback also captures:**
- Architectural preferences discovered during the session ("don't use X pattern", "prefer Y approach")
- Rework instructions ("PR #1905 approach was wrong, try Z instead")
- Scope adjustments ("skip this issue, it's not needed for v1")

At every spot-check, display unresolved feedback:
```
UNRESOLVED FEEDBACK:
- [L3] "Button click area too small on calc footer" -- affects #1527
- [L3] "Don't use ScrollArea for short lists" -- general
```

---

## State Schema

`.claude/state/blitz.json`:

```json
{
  "version": 3,
  "source": "ROADMAP.md",
  "current_layer": 3,
  "layer_start_sha": "abc123",
  "spot_check_pending": false,
  "spot_check_interval": 4,
  "merges_since_spot_check": 0,
  "layers": {
    "3": {
      "name": "Lock the Protocol",
      "status": "in_progress",
      "sections": {
        "3b_sdk": {
          "issues": {
            "1527": {"status": "in_review", "pr": 1910, "branch": "feature/1527-layout-fundamentals"},
            "1645": {"status": "remaining"}
          }
        }
      }
    }
  },
  "feedback": [
    {
      "timestamp": "2026-06-03T14:30:00Z",
      "layer": 3,
      "context": "spot-check after PR #1905",
      "text": "Button click area too small on calc footer",
      "resolution": null,
      "affects_issues": ["1527"]
    }
  ],
  "history": [
    {"event": "spot_check_pass", "layer": 3, "round": 1, "prs": [1905, 1906], "timestamp": "2026-06-03T12:00:00Z"},
    {"event": "spot_check_fail", "layer": 3, "round": 2, "prs": [1910], "reason": "calc footer regression", "timestamp": "2026-06-03T14:30:00Z"}
  ],
  "last_updated": "2026-06-03T14:30:00Z"
}
```

Status values: `remaining` | `in_flight` | `in_review` | `needs_rework` | `blocked` | `failed` | `done`

Key fields:
- `spot_check_pending` -- when true, orchestrator halts dispatch and waits for user pass/fail. **This is the ONLY thing that stops dispatch.**
- `spot_check_interval` -- number of merges between spot checks (default 4)
- `merges_since_spot_check` -- counter, reset to 0 after each spot check pass
- `feedback[]` -- persists across resets, fed to sub-agents when relevant
- `history[]` -- audit trail of spot checks and merges (never deleted)

---

## Invocation Variants

| Command | Behavior |
|---|---|
| `/blitz` | Continue from state. Assess, fill lanes, fan out |
| `/blitz status` | Read state, report progress, show unresolved feedback |
| `/blitz reset` | Delete state, re-read roadmap, rebuild from scratch |
| `/blitz merge` | Skip dispatch, just merge all `in_review` PRs, then spot-check |
| `/blitz layer <N>` | Force-advance to layer N (skip completed layers) |
| `/blitz issue <N>` | Fan out a single specific issue immediately |
| `/blitz feedback <text>` | Record feedback into state. Optionally tag with issue numbers |
| `/blitz pass` | Mark current spot-check as passed, advance to next batch |
| `/blitz fail <text>` | Mark current spot-check as failed, halt dispatch, record reason |

---

## Rules

1. **Never ask permission to dispatch.** If lanes are open and issues remain, fill them. The only gate is `spot_check_pending`.
2. **Never read issue bodies in the orchestrator context.** Sub-agents do that.
3. **Never grep source code in the orchestrator context.** Sub-agents do that.
4. **Never write production code in the orchestrator context.** Only state management.
5. **Always update state before reporting to user.** State is the source of truth across resets.
6. **Maximum 4 concurrent lanes.** More causes worktree/git conflicts.
7. **Bump-and-install only at spot checks, not per merge.** Merges happen inline as lanes finish.
8. **Report progress after every action.** The user should always know where things stand.
9. **If a sub-agent fails, don't retry immediately.** Mark blocked, backfill the lane with the next issue, report.
10. **Bundle batch runs as a single lane** -- one agent, one PR, all micro-issues together.
11. **State file is sacred.** Never proceed without reading it. Never exit without writing it.
12. **Cross-layer dispatch is fine.** If the current layer is nearly done and lanes are free, pull from the next layer. Don't let lanes sit idle waiting for a layer gate.
13. **Merge inline, not in batches.** When a lane produces a PR, merge it immediately (unless spot_check_pending). Don't accumulate PRs for a batch merge step.

---

## Recovery From Context Reset

When invoked after a context reset, the agent has zero memory of prior work. The state file provides full continuity:

1. Read `.claude/state/blitz.json` -- this tells you everything
2. Check in-flight items against GitHub (PRs may have been merged externally)
3. Reconcile state with reality (close issues that got merged, unblock downstream)
4. Continue from where state says you are

The orchestrator never needs to re-read the roadmap after initial setup -- the state file has the full issue list per layer, statuses, and dependency graph.
