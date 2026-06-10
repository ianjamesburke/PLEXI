---
name: implement-stint
description: "Phase 1 of the PLEXI ship pipeline when work is selected from .stint. Resolves a stint task, creates a feature worktree, names the Plexi pane, runs stint start from inside that worktree, implements the task, pushes the branch, then hands off to /open-pr."
risk: medium
source: local
date_added: "2026-06-10"
---

# Implement Stint

Use this when the requested work is a `.stint` task, or when no specific GitHub issue was provided and the next task should come from the sprint graph.

| Invocation | Behavior |
|---|---|
| `/implement-stint` | Run `stint next`, pick the first ready task, create a worktree for it, then start it inside that worktree |
| `/implement-stint <task-id>` | Use that task directly, create a worktree for it, then start it inside that worktree |

Output on successful implementation:

```text
[IMPLEMENTED] Stint <task-id>
Branch: feature/stint-<task-id>-<slug>
Files changed: <N>
Pipeline: pushed; invoking /open-pr inline
```

This skill is not complete when the worktree exists. Completion means the task was started inside the worktree, implemented, committed, pushed, and handed to `/open-pr`.

## Non-Negotiables

- Never run `stint start` from the base branch. Create the worktree first, then run `stint start <task-id>` inside that worktree.
- Do not use `stint next --claim` here. Claiming mutates `.stint` before the worktree exists and can create timestamp conflicts.
- Name the Plexi pane before coding:
  ```bash
  plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL} pane name "stint-<task-id> · impl"
  ```
- Branch name:
  ```text
  feature/stint-<task-id>-<short-slug>
  ```
- Worktree path rule: after creation, all file reads, edits, tests, commits, and `stint start` commands run from the worktree path, not the repo root.
- If the task links a GitHub issue, keep issue labels as the live PR pipeline state. If it does not, still open a PR; just skip issue labels and Ship Log updates.

## Phase 0 - Resolve Task

If a task id is provided, skip `stint next`.

For a provided task id:

```bash
ls .stint/tasks/<task-id>-*.md
sed -n '1,180p' .stint/tasks/<task-id>-*.md
```

For no argument:

```bash
stint next
```

Use the first task listed under `Ready:`. Resolve its task file with:

```bash
ls .stint/tasks/<task-id>-*.md
sed -n '1,180p' .stint/tasks/<task-id>-*.md
```

Stop if:

- no ready task exists
- the task file is missing
- the task is already `done`
- the task is `in-progress` and this pane is not explicitly resuming that same work
- `blocked_by` still points at an unfinished task or issue

Extract from frontmatter:

- `id`
- `title`
- `status`
- `gh_issue`
- `area`
- `blocked_by`
- `sprint`

## Phase 1 - Pre-Flight

Run from repo root:

```bash
git fetch origin
git status --porcelain
git log origin/alpha..HEAD --oneline
git worktree list
```

Hard stops:

1. Dirty repo root -> stop and print `git status --short`.
2. Local alpha is behind origin and has no local-only commits -> run `git pull --rebase origin alpha`.
3. Local alpha has unpushed commits but clean tree -> continue, and note that the worktree branches from local `HEAD`.
4. Matching branch already exists on origin -> stop and surface the branch.
5. Matching worktree already exists -> use it only if this pane is explicitly resuming; otherwise stop.

If `gh_issue` is present, fetch the linked issue before proceeding:

```bash
gh issue view <issue-number> --json state,title,body,labels
```

Stop if the linked issue is closed or already labeled `in progress`, unless this pane is explicitly resuming that issue's existing worktree.

## Phase 2 - Create Worktree, Name Pane, Start Task

Build a short slug from the task title: lowercase, ASCII, words separated by `-`, no punctuation, max about 8 words.

Create the worktree from `HEAD`:

```bash
wtp add -b feature/stint-<task-id>-<short-slug> HEAD
```

Verify the worktree base:

```bash
git -C worktrees/feature/stint-<task-id>-<short-slug> log --oneline -1
git log --oneline -1
```

If the commits differ, delete the new worktree/branch and recreate it before touching code.

Name the pane:

```bash
plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL} pane name "stint-<task-id> · impl"
```

If the task links a GitHub issue, mark the issue in progress after the worktree exists:

```bash
gh issue edit <issue-number> --add-label "in progress" --add-label "pipeline:implement"
```

Now start the task from inside the worktree:

```bash
cd worktrees/feature/stint-<task-id>-<short-slug>
stint start <task-id>
```

Do not use `--restart` unless correcting bad timing data. If `started_at` already exists for a legitimate resumed task, keep it.

Run `stint check` in the worktree after `stint start`.

## Phase 3 - Formulate

Read the task body first. If it links a GitHub issue, use the already-fetched issue body for implementation detail and prior attempts.

Before editing code, produce a short implementation spec in context:

```text
Task: <task-id> <title>
Linked issue: #<n> or none
Files to change:
  - <path>: <what changes>
Files not to touch:
  - <path>: <why out of scope>
Test that must pass:
  - <command>
Invariants:
  - <from task, PRM, GOTCHAS.md, AGENTS.md>
Logging plan:
  - <new info/warn/error traces required by AGENTS.md>
```

If the task or linked issue names exact files/functions, verify them cheaply with `rg`/`git ls-files` before broad discovery. If they do not, do the narrowest search that identifies the owning code.

For File Explorer work, read `docs/prm/host-ui-kit.md` first, then `docs/prm/file-explorer-overhaul.md`.

For app-framework, packaging, marketplace, MCPUI, WASM/WASI, or Bevy work, read `docs/prm/app-framework-marketplace.md` first.

For architectural choices, read `NORTH_STAR.md` and `GLOSSARY.md`.

## Phase 4 - Implement

Write tests before implementation for host logic. New `AppRequest` or `HostEffect` behavior needs a `HostHarness` test first.

Scope gate:

- 3 files or fewer: implement inline.
- More than 3 files or multiple subsystems: dispatch a subagent with the task body, linked issue body, implementation spec, worktree path, and the rule: stage changes but do not commit.

Always run at least:

```bash
cargo build
```

Run the narrower relevant test command too, usually:

```bash
cargo test --bin plexi <test_name>
```

## Phase 5 - Commit, Push, Handoff

Commit from the worktree:

```bash
git add <files>
git commit -m "<type>: <short task title> (stint <task-id>)"
git push -u origin HEAD
```

If a linked GitHub issue exists, append or update its Ship Log with:

```markdown
## Ship Log

### Attempt <N> - <YYYY-MM-DD>
**Branch:** feature/stint-<task-id>-<short-slug>
**Stint:** started <task-id> from worktree
**Files changed:** <key files>
**Spec summary:** <one-line approach>
```

Set labels:

```bash
gh issue edit <issue-number> --remove-label "pipeline:implement" --add-label "pipeline:open-pr" --add-label "ready"
```

Rename the pane:

```bash
plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL} pane name "stint-<task-id> · pushed"
```

Then invoke `/open-pr` inline for the branch. Do not run `stint done` here; `/merge-pr` closes the task after the PR merges and alpha is verified.

If no linked GitHub issue exists, still invoke `/open-pr` for the branch. The PR body should name the stint task and state that there is no linked GitHub issue.

## Blocked Or Abandoned Work

If the work blocks after `stint start`, leave `started_at` in place. Do not run `stint done`.

Record the blocker in the task body and linked issue if one exists. Rename the pane:

```bash
plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL} pane name "stint-<task-id> · blocked"
```
