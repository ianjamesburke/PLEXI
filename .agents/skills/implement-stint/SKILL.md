---
name: implement-stint
description: "Phase 1 of the PLEXI ship pipeline when work is selected from .stint. Resolves a stint task, starts and commits the task timing on alpha, branches a feature worktree from that claim commit, implements the task, pushes the branch, then hands off to /open-pr."
risk: medium
source: local
date_added: "2026-06-10"
---

# Implement Stint

Use this when the requested work is a `.stint` task, or when no specific GitHub issue was provided and the next task should come from the sprint graph.

| Invocation | Behavior |
|---|---|
| `/implement-stint` | Run `stint next`, pick the first ready task, claim it on alpha, commit the claim, then create a worktree from that commit |
| `/implement-stint <task-id>` | Use that task directly, claim it on alpha, commit the claim, then create a worktree from that commit |

Output on successful implementation:

```text
[IMPLEMENTED] Stint <task-id>
Branch: feature/stint-<task-id>-<slug>
Files changed: <N>
Pipeline: pushed; invoking /open-pr inline
```

This skill is not complete when the worktree exists. Completion means the task was claimed on alpha, the claim was committed, the implementation branch was created from that commit, the task was implemented, committed, pushed, and handed to `/open-pr`.

## Non-Negotiables

- Run `stint claim <task-id>` from alpha before creating a fresh worktree.
- Immediately commit only the resulting `.stint/tasks/<task-id>-*.md` change directly to alpha with `git commit .stint/tasks/<task-id>-*.md -m "chore: claim stint <task-id>"`.
- Create the implementation worktree from that alpha claim commit.
- Default validation should use an isolated PR build. Do not install from a feature worktree, and do not replace alpha/main as proof of the change. Only mark install skippable when tests and scenes fully cover the behavior.
- Before claiming, check whether a matching local worktree or branch already exists. If it is dirty, ahead, or the user is clearly resuming, use resume mode instead of creating a new worktree.
- Name the Plexi pane before coding:
  ```bash
  plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL} pane name "stint-<task-id> · impl"
  ```
- Branch name:
  ```text
  feature/stint-<task-id>-<short-slug>
  ```
- Worktree path rule: after creation, all implementation reads, edits, tests, and commits run from the worktree path, not the repo root.
- If the task links a GitHub issue, keep issue labels as the live PR pipeline state. If it does not, still open a PR; just skip issue labels and Ship Log updates.
- Publish phase handoff state with `. .agents/skills/_lib/pipeline-slots.sh` and `pipeline_slots_set implement <issue-or-task> "" <status> "" ""` whenever pane title state changes.

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

Before reading linked issue bodies or broad PRDs, check for existing local work:

```bash
git worktree list
git branch -a --list '*<task-id>*' '*<gh_issue>*'
```

If a matching worktree exists, inspect it with low-output commands first:

```bash
git -C <worktree> status --short --branch
git -C <worktree> diff --stat
```

If that worktree is dirty, ahead, or this pane is resuming the task, enter resume mode. Do not create a new worktree. Do not read full issue bodies unless the task file and current diff are insufficient.

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
5. Matching worktree or branch already exists -> inspect it before claim or creation. If it is dirty, ahead, or this pane is resuming the task, use resume mode. Otherwise stop and surface the existing branch/worktree.

If `gh_issue` is present, fetch the linked issue status before proceeding:

```bash
gh issue view <issue-number> --json state,title,labels
```

Fetch `body` later only if the task file does not contain enough implementation detail.

Stop if the linked issue is closed or already labeled `in progress`, unless this pane is explicitly resuming that issue's existing worktree.

## Phase 2 - Claim, Create Worktree, Name Pane

Build a short slug from the task title: lowercase, ASCII, words separated by `-`, no punctuation, max about 8 words.

For fresh work, claiming and committing are one uninterrupted step from alpha. Do not do more discovery between `stint claim` and the claim commit:

```bash
stint claim <task-id>
git status --short
git diff -- .stint/tasks/<task-id>-*.md
git commit .stint/tasks/<task-id>-*.md -m "chore: claim stint <task-id>"
```

The claim commit must contain only that task file. If `git status --short` shows any other change, stop before committing. Do not include code, docs, config, generated files, or unrelated task updates in the claim commit.

Create the worktree from the alpha claim commit:

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
pipeline_slots_set implement <issue-or-task> "" working "" ""
```

If the task links a GitHub issue, mark the issue in progress after the worktree exists:

```bash
gh issue edit <issue-number> --add-label "in progress" --add-label "pipeline:implement"
```

Do not use `--restart` unless correcting bad timing data. If `started_at` already exists for a legitimate resumed task, keep it.

Run `stint check` in the implementation worktree. Do not run `stint claim` there unless intentionally localizing task state to the branch.

### Resume Mode

If the implementation worktree already exists:

- Check the canonical task file on alpha. If it already has `status: in-progress` and `started_at`, keep it.
- If the canonical task file is still backlog or missing `started_at`, run `stint claim <task-id>` on alpha and immediately commit only that `.stint/tasks/<task-id>-*.md` change.
- Preserve dirty implementation work before rebasing or merging the existing worktree onto the claim commit.
- Keep setup reads short: `status --short --branch`, `diff --stat`, and targeted `rg` before full diffs or issue bodies.
- Continue implementation in the existing worktree.

## Phase 3 - Research (Sub-agent R)

Spawn a **read-only** sub-agent. Its only job is to produce the implementation spec; it must not edit any file.

**Prompt to Sub-agent R:**

> You are Sub-agent R for stint task `<task-id>`: `<title>`.
> Worktree: `<worktree-path>`
> Task body: `<paste full task body>`
> Linked issue body (if any): `<paste or "none">`
>
> Your output is a structured implementation spec in exactly this format — nothing else:
>
> ```text
> Task: <task-id> <title>
> Linked issue: #<n> or none
> Files to change:
>   - <path>: <what changes>
> Files not to touch:
>   - <path>: <why out of scope>
> Test that must pass:
>   - <command>
> Invariants:
>   - <from task, CLAUDE.md, AGENTS.md>
> Logging plan:
>   - <new info/warn/error traces required by AGENTS.md>
> ```
>
> Rules:
> - Read `.agents/skills/implement-stint/SKILL.md` lines 1-50 for invariants.
> - If the task or issue names exact files/functions, verify them with `rg`/`git ls-files` first.
> - Short-circuit: if the task body already contains an explicit file+line implementation map, reformat it into the spec structure above without further discovery.
> - For File Explorer work, read `src/ui/AGENTS.md` first.
> - For app-framework, packaging, marketplace, MCPUI, WASM/WASI, or Bevy work, read `docs/app-framework-marketplace.md` first.
> - For architectural choices, read `NORTH_STAR.md` and `GLOSSARY.md`.
> - Do NOT edit any file. Return only the spec.

Receive the spec from Sub-agent R. If it is missing any required field, ask Sub-agent R to fill the gap before proceeding to Phase 4.

## Phase 4 - Implement (Sub-agent I)

Spawn **Sub-agent I** with the spec from Phase 3 and the worktree path. Sub-agent I owns all edits, tests, and the Gemini review loop. It must NOT commit.

**Prompt to Sub-agent I:**

> You are Sub-agent I for stint task `<task-id>`: `<title>`.
> Worktree: `<worktree-path>`
>
> Implementation spec:
> `<paste spec from Sub-agent R>`
>
> Rules:
> - Write tests before implementation for host logic. New `AppRequest` or `HostEffect` behavior needs a `HostHarness` test first.
> - Always run `cargo build` after edits.
> - Run the narrower relevant test command: `cargo test --bin plexi <test_name>` (adjust filter as needed).
> - After tests pass, run the `/code-review` skill (or equivalent Gemini diff review) on the staged diff. Maximum 2 review runs per validation attempt. Iterate on findings internally between runs.
> - Stage all changes with `git add <files>`. Do NOT commit — the orchestrator commits.
> - Return a summary in this format:
>
> ```text
> Files changed: <list>
> Test results: <N passed, M failed — command used>
> Gemini verdict: clean | findings remaining: <list>
> Staged: yes
> ```
>
> If Gemini still has unresolved findings after 2 runs, include them in the `findings remaining` field — do not block; just report.

Receive the summary from Sub-agent I.

**Orchestrator diff review:** Run `git -C <worktree-path> diff --staged --stat` and spot-check the diff before committing. If Sub-agent I reported unresolved Gemini findings, surface them to the user and ask whether to proceed or iterate.

Then run the `/testing` skill (`.agents/skills/testing/SKILL.md`) to produce the `**Test evidence:**` block — diff classification, harness tests, headless render screenshots for visual changes. Include the block in the Ship Log entry (or PR body when no issue is linked) during handoff.

Validation bias:

- Use `binary install required` as the default conclusion for visible UI, keyboard, app-launch, channel, filesystem, host/runtime, or interaction changes.
- Use `install skippable — full coverage` only for pure logic, docs-only, or changes with direct HostHarness/PlexiUiHarness/scene coverage that exercises the full user-visible behavior.
- If binary install is required, explicitly state that validation should install the PR build with `just pr-install <PR>` after `/open-pr` creates the PR. The validator should run `plexi-pr-<PR>` from the relevant workspace, not `plexi` or `plexi-alpha`.
- Never cite `cargo build` or a feature-worktree app install as install evidence.

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

**Test evidence (attempt <N>):**
- cargo test: <passed> passed, <failed> failed — filters: <module list or "full bin suite">
- PlexiUiHarness render: /tmp/plexi-render-<task>-<name>.png — <what it shows> (omit if no UI layer touched)
- Conclusion: install skippable — full coverage | binary install required — <why> | docs-only — no test evidence required
- PR install: required via `just pr-install <PR>` | skippable because <specific coverage reason>
```

Set labels:

```bash
gh issue edit <issue-number> --remove-label "pipeline:implement" --add-label "pipeline:open-pr" --add-label "ready"
```

Rename the pane:

```bash
plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL} pane name "stint-<task-id> · pushed"
pipeline_slots_set implement <issue-or-task> "" pushed "" ""
```

Then invoke `/open-pr` inline for the branch. Do not run `stint done` here; `/merge-pr` closes the task after the PR merges and alpha is verified.

If no linked GitHub issue exists, still invoke `/open-pr` for the branch. The PR body should name the stint task and state that there is no linked GitHub issue.

## Blocked Or Abandoned Work

If the work blocks after `stint claim`, leave `started_at` in place. Do not run `stint done`.

Record the blocker in the task body and linked issue if one exists. Rename the pane:

```bash
plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL} pane name "stint-<task-id> · blocked"
pipeline_slots_set implement <issue-or-task> "" blocked "" "<blocker summary>"
```
