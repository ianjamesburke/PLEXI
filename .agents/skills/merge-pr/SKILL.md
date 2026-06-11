---
name: merge-pr
description: "Phase 4 of the PLEXI ship pipeline. Takes an approved PR number, squash-merges to alpha, bumps the version, closes the issue, and cleans up. Input: PR number. Output: merged alpha at new version."
risk: medium
source: local
date_added: "2026-05-20"
---

# Merge PR

Phase 4 of the ship pipeline. Input: approved PR number. Output: clean alpha at new version.

> **Labels are the live state.** On success, all `pipeline:*` labels are removed when the issue closes. On failure, remove all `pipeline:*` labels and `in progress`, add `ready`.

> **Stint timing closure.** Close linked stint tasks with `stint done <task-id>` after the script exits 0.
>
> **Pane slots.** Source `.agents/skills/_lib/pipeline-slots.sh` and publish `pipeline_slots_set merge "$ISSUE" "$PR_NUMBER" <status> "" <last-error>` at status changes.

**Entry:** `/merge-pr <pr-number>`

---

## Step 0 — Set CWD and Flip Pane Status

```bash
cd "$(git rev-parse --show-toplevel)"
plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL} pane name "#<n> · merging"
pipeline_slots_set merge "$ISSUE" "$PR_NUMBER" merging "" ""
```

> Labels are already correct when invoked inline from validate-pr. No label edit needed here.

---

## Step 1 — Run the Merge Script

```bash
just merge-pr <PR_NUMBER>
```

Handles: rebase, squash-merge, alpha sync, artifact cleanup, version bump, issue close, ship log.

**If it exits non-zero**, read the error and recover with sub-steps:

| Error | Recovery |
|-------|----------|
| Rebase conflict | Resolve in `worktrees/$BRANCH`, then: `just merge-rebase $BRANCH` → `just merge-squash $PR` → continue below |
| PR already merged | `just merge-sync && just merge-cleanup $PR $BRANCH && just merge-bump && just merge-close $ISSUE $PR` |
| Dirty root worktree | Commit or restore changed files, re-run `just merge-pr $PR` |
| `>1 unexpected commits` on local alpha | Investigate before proceeding; do not force-reset blindly |

**Available sub-steps:**
```bash
just merge-rebase <BRANCH>       # fetch + rebase feature branch on origin/alpha + force-push
just merge-squash <PR>           # squash-merge only
just merge-sync                  # reset local alpha to origin/alpha (safe: fails if >1 unexpected commit)
just merge-cleanup <PR> <BRANCH> # channel-clean + wtp remove + remote branch delete
just merge-bump                  # just bump + git push
just merge-close <ISSUE> <PR>    # strip pipeline labels + close issue + append ship log
```

---

## Step 2 — Close Linked Stint Tasks

```bash
rg -l 'gh_issue: .*"<issue>"|gh_issue: .*\[<issue>\]' .stint/tasks
stint done <task-id>
```

If estimate was off >2x, add one sentence to the task body explaining why. If no linked task exists, note it in the ship log.

---

## Step 3 — Unblock Downstream Issues

```bash
gh issue-ext blocking list $ISSUE --json number,title,state 2>/dev/null
```

For each open blocked issue, check if all blockers are now closed. If so:
```bash
gh issue edit <n> --remove-label "blocked" --add-label "ready"
```

---

## Step 4 — Complete and Close Pane

```bash
git status  # must be clean
```

```
[COMPLETE]
- Merged: PR #<n> — <title>
- Closed: Issue #<n> — <title>
- Version: v<x.y.z>
```

```bash
plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL} pane name "#<n> · done"
pipeline_slots_set merge "$ISSUE" "$PR_NUMBER" done "" ""
plexi pane close
```

**Soft exit** (deferred threads): use `needs-you` pane status instead of `done`; do not call `plexi pane close`.

---

## Rules

- CWD must be repo root for all commands
- `just install` is the user's responsibility post-merge
- Never pass `--delete-branch` to `gh pr merge` — git refuses to delete a branch checked out by a worktree
- Never commit directly to alpha, beta, or main
- Alpha must be clean when this skill exits
- Never close a linked stint task until `just merge-pr` exits 0
- On unrecoverable failure: set pane to `blocked`, run `pipeline_slots_set merge "$ISSUE" "$PR_NUMBER" blocked "" "<error summary>"`, comment on issue, remove `in progress` + all `pipeline:*` labels, add `ready`, exit
