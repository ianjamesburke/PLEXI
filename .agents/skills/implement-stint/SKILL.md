---
name: implement-stint
description: "Phase 1 of the PLEXI ship pipeline when work is selected from .stint. Resolves a stint task, claims it (stint owns state — no git commit needed), branches a feature worktree from alpha HEAD, implements the task, pushes the branch, then hands off to /open-pr."
risk: medium
source: local
date_added: "2026-06-10"
---

# Implement Stint

Use this when the requested work is a `.stint` task, or when no specific GitHub issue was provided and the next task should come from the sprint graph.

| Invocation | Behavior |
|---|---|
| `/implement-stint` | Run `stint next`, pick the first ready task, claim it (`stint claim` owns state), then create a worktree from alpha HEAD |
| `/implement-stint <task-id>` | Use that task directly, claim it, then create a worktree from alpha HEAD |

Output on successful implementation:

```text
[IMPLEMENTED] Stint <task-id>
Branch: feature/stint-<task-id>-<slug>
Files changed: <N>
Pipeline: pushed; invoking /open-pr inline
```

This skill is not complete when the worktree exists. Completion means the task was claimed (`stint claim` owns state — no git commit), the implementation branch was created from alpha HEAD, the task was implemented, committed, pushed, and handed to `/open-pr`.

## Non-Negotiables

- Run `stint claim <task-id>` from alpha before creating a fresh worktree. `stint` owns task state — no git commit.
- Create the implementation worktree from alpha HEAD immediately after claiming.
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

**Before proposing any fix**, read the AGENTS.md for every directory the task touches. These files contain traps — patterns that look reasonable but are wrong for this codebase. Checking them first prevents pitching a solution the codebase explicitly prohibits.

```bash
# Example: task touches src/app/lifecycle.rs and src/pane_ops/workspace.rs
cat AGENTS.md  # root — cross-cutting traps
cat src/app/AGENTS.md 2>/dev/null || true
cat src/pane_ops/AGENTS.md 2>/dev/null || true
```

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

For fresh work, claim then immediately create the worktree. `stint` owns task state; no git commit is needed:

```bash
stint claim <task-id>
```

Create the worktree from alpha HEAD:

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
- If the task is still unclaimed, run `stint claim <task-id>` on alpha. No git commit needed.
- Preserve dirty implementation work before rebasing or merging the existing worktree onto the claim commit.
- Keep setup reads short: `status --short --branch`, `diff --stat`, and targeted `rg` before full diffs or issue bodies.
- Continue implementation in the existing worktree.

## Phase 3 - Research (Sub-agent R)

**Default: skip deep research.** Most stint tasks already contain sufficient scope, file references, and implementation detail. Only spawn Sub-agent R when the task body lacks actionable detail (no files named, no clear approach, ambiguous scope). When skipping, the orchestrator builds the spec inline from the task body and proceeds directly to Phase 4.

When research IS needed, spawn a **read-only** sub-agent. Its only job is to produce the implementation spec; it must not edit any file.

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
- After tests pass, run a Codex review against alpha (maximum 2 runs; iterate on findings between runs). Write output to a tmp file; only read it if findings are present:
>   ```bash
>   CODEX_OUT="/tmp/plexi-codex-review-$$.txt"
>   codex review -c model_reasoning_effort=low --base alpha \
>     "Review this diff for correctness bugs, unsafe patterns, missed error handling, and clear simplification opportunities. Do not flag style or cosmetic issues. Reply with a concise bulleted findings list referencing file and line, or 'No issues found.' if clean." \
>     > "$CODEX_OUT" 2>&1 || true
>   if grep -qi "no issues found" "$CODEX_OUT"; then
>     CODEX_VERDICT="clean"
>   else
>     CODEX_VERDICT=$(tail -80 "$CODEX_OUT")
>   fi
>   ```
> - Stage all changes with `git add <files>`. Do NOT commit — the orchestrator commits.
> - Return a summary in this format:
>
> ```text
> Files changed: <list>
> Test results: <N passed, M failed — command used>
> Codex verdict: clean | findings remaining: <list>
> Staged: yes
> ```
>
> If Codex still has unresolved findings after 2 runs, include them in the `findings remaining` field — do not block; just report.

Receive the summary from Sub-agent I.

**Orchestrator diff review:** Run `git -C <worktree-path> diff --staged --stat` and spot-check the diff before committing. If Sub-agent I reported unresolved Codex findings, surface them to the user and ask whether to proceed or iterate.

### Who reviews what

This phase is the **single owner of AI diff review** for the whole pipeline. Sub-agent I's Codex pass above is the one AI review the diff gets, and it runs pre-push where findings are cheapest to fix — the worktree is live and no PR churn is needed.

The other phases deliberately do not review:

| Layer | Reviews? |
|---|---|
| `/implement-stint` Phase 4 (here) | **Yes** — Codex, max 2 runs, pre-push |
| `/open-pr` | No — its own Rules already say so |
| `/validate-pr` | No — owns install + user acceptance; carries this phase's verdict into the testing block |
| Babysitter tester pane | Behavior only — live-drives the build, does not re-review the diff |

**CI is not a review layer today.** The `CodeRabbit` check reports "automatic reviews are disabled" on every PR, and the `claude` check is conditional and skips on Rust-only diffs. Treat a green `gh pr checks` as "nothing red," not as "the diff was reviewed." If this phase's review is skipped, the diff ships unreviewed — say so in the handoff rather than assuming CI caught it.

Then run the `/testing` skill (`.agents/skills/testing/SKILL.md`) to produce the `**Test evidence:**` block — diff classification, harness tests, headless render screenshots for visual changes. Include the block in the Ship Log entry (or PR body when no issue is linked) during handoff.

**Install-skip conclusion:** the `/testing` skill's Step 5 conclusion rules are the single source of truth. Apply them as written — do not re-derive skip criteria here.
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

**Stop-at-PR-open mode.** When the invoker says to stop once the PR is open — babysitter workers always do — hand off to `/open-pr` but tell it not to chain into `/validate-pr`. Report the PR number and green checks, then end the turn. A separate tester owns validation in that mode; running `/validate-pr` anyway costs a duplicate `just pr-install` (~5 min) and leaves the turn parked on a `[TESTING]` block waiting for a reply that is not coming.

## Blocked Or Abandoned Work

If the work blocks after `stint claim`, leave `started_at` in place. Do not run `stint done`.

Record the blocker in the task body and linked issue if one exists. Rename the pane:

```bash
plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL} pane name "stint-<task-id> · blocked"
pipeline_slots_set implement <issue-or-task> "" blocked "" "<blocker summary>"
```
