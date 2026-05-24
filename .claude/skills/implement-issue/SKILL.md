---
name: implement-issue
description: "Phase 1 of the PLEXI ship pipeline. Finds an issue, sets up a worktree, and implements the code changes. Stops before creating a PR — output is a pushed feature branch. Entry points: /implement-issue (auto-find), /implement-issue <n> (specific), /implement-issue P1 (by priority), /implement-issue <n> <m> [...] (bundle)."
risk: medium
source: local
date_added: "2026-05-20"
---

# Implement Issue

Phase 1 of the ship pipeline. Output: a pushed feature branch ready for `/open-pr`.

| Invocation | Behavior |
|---|---|
| `/implement-issue` | Auto-find next unblocked issue (P0→P4) |
| `/implement-issue <n>` | Specific issue |
| `/implement-issue P1` | First unblocked at that priority |
| `/implement-issue <n> <m> [...]` | Bundle — implement multiple issues in one branch |

On completion, **append a Ship Log entry to the issue body**, set pipeline labels (see Pipeline Labels below), and output:

```
[IMPLEMENTED] Issue #<n>
Branch: feature/<n>-short-description
Files changed: <N>
Pipeline: pipeline:open-pr + ready set — PM will dispatch /open-pr on next run
```

> **Labels are the live state.** Never read the Ship Log to determine pipeline stage — read the issue labels. Ship Log is audit trail only.

---

## Phase 0 — Find the Issue

> **Skip this phase entirely when a specific issue number is provided as the argument.** Go directly to Phase 1 with that number — no git log, no in-progress scan.

Run in parallel:
```bash
git log --oneline -10
gh issue list --label "in progress" --json number,title
```

Surface the in-progress list as context.

**Determine search scope from argument:**
- No arg → try P0, P1 (no `ready` filter), then P2→P4 filtered to `ready`
- Priority arg → search only that level; apply `ready` filter for P2+

```bash
# P0/P1:
gh issue list --label "<priority>" --state open --json number,title,labels,body --limit 50

# P2/P3/P4:
gh issue list --label "<priority>" --label "ready" --state open --json number,title,labels,body --limit 50
```

Sort ascending by issue number. For each:
1. Skip if labeled `in progress`
2. If labeled `blocked`: parse `depends_on` from front matter; skip if any dependency is OPEN. If all CLOSED: strip `blocked`, add `ready`
3. Skip if body contains "Do not implement here" (epic tracker)
4. Skip if body has unlabeled "Depends on" section with open issues — add `blocked` label
5. Otherwise: this is the target → proceed to Phase 1

Advance to next priority level if no unblocked issues found (no-arg mode only).

**Front matter convention** — every issue body opens with:
```
---
depends_on: []
---
```

---

## Phase 1 — Pre-flight

**Check issue state first:**
```bash
gh issue view <number> --json state,title,labels --jq '{state: .state, title: .title, labels: [.labels[].name]}'
```

If `CLOSED`: stop. "Issue #<n> is already closed — nothing to do."

If labeled `in progress`: surface existing worktree + PR before proceeding. Ask for takeover confirmation — do not proceed until user confirms.

**Check if already done** (skip if issue is labeled `ready` — PM pre-screened it):
```bash
gh issue view <number> --json body --jq '.body'
```
Grep `src/` on alpha and `git log --oneline -20` against Done When criteria. If all criteria met: close the issue and stop.

**Sync alpha + check unpushed (batched):**
```bash
git fetch origin && git status --porcelain && git log origin/alpha..HEAD --oneline
```

If dirty: auto-stash:
```bash
git stash push -m "implement-issue auto-stash before #<number>"
IMPL_STASHED=true
```

If any unpushed commits listed: STOP. Tell user to push first. Pop stash, exit.

Then: `git pull --rebase origin alpha`

**Label + pane setup:**
```bash
gh issue edit <number> --add-label "in progress" --add-label "pipeline:implement"
IMPL_PANE=$PLEXI_PANE_ID
_PROJ_ITEM=$(gh api graphql -f query='query($n:Int!){repository(owner:"ianjamesburke",name:"PLEXI"){issue(number:$n){projectItems(first:5){nodes{id project{id}}}}}}' -F n=<number> --jq '.data.repository.issue.projectItems.nodes[]|select(.project.id=="PVT_kwHOAkOgys4BXaQY")|.id')
[ -n "$_PROJ_ITEM" ] && gh api graphql -f query='mutation($i:ID!,$v:String!){updateProjectV2ItemFieldValue(input:{projectId:"PVT_kwHOAkOgys4BXaQY",itemId:$i,fieldId:"PVTSSF_lAHOAkOgys4BXaQYzhSnRw8",value:{singleSelectOptionId:$v}}){projectV2Item{id}}}' -f i="$_PROJ_ITEM" -f v="47fc9ee4" > /dev/null
```

---

## Phase 1b — Implementation Audit

**Skip if issue is labeled `ready` and was dispatched with a specific number** — PM already screened it and implementation has not started. Proceed directly to Phase 2.

Re-read Done When criteria against alpha `src/` and `git log --oneline -20`.

- Partial: surface what's done vs missing, state the plan, ask open design questions. Wait for confirmation.
- Nothing done: proceed, note "Audit: nothing on alpha yet."

---

## Phase 2 — Worktree Setup

**Check for remote branch ownership:**
```bash
EXISTING=$(git ls-remote origin "refs/heads/feature/<issue-number>-*" | head -1)
```
If non-empty: another agent owns this. Surface branch + any existing PR, then stop.

Create worktree:
```bash
wtp add -b feature/<issue-number>-short-description origin/alpha
```

If "branch already exists": check `git worktree list`. If no worktree, `wtp add` without `-b`. Check for prior commits.

**Verify base immediately:**
```bash
git -C worktrees/<branch> log --oneline -1
git log --oneline -1
```
If mismatch: delete worktree and branch, redo.

---

## Phase 3 — Formulate

Before writing code, produce a tight implementation spec.

**Read in parallel:**
- Full issue body: `gh issue view <number> --json title,body,labels`
- Relevant source files (grep for affected modules, then read them)

**Grep GOTCHAS.md and DEV_LOG.md (required):**
```bash
grep -in "<term1>\|<term2>\|<term3>" GOTCHAS.md DEV_LOG.md
```
Surface every match under **Gotchas found:**. Each requires disposition: **NOTED** or **N/A**.

**If any changed file is a Python app under `apps/core/`, `apps/examples/`, or `dev-examples/`:** invoke `create-plexi-app` skill before writing code.

**If issue involves a third-party library:** invoke `coding-conventions` skill.

**Write implementation spec (in context only, not to disk):**
```
Files to change:
  - src/foo.rs:42 — <exactly what>

Files NOT to touch:
  - <adjacent but out of scope>

CLI rename check:
  - grep -rn "<old-command>" .claude/skills/ if renaming a subcommand

Test that must pass:
  - cargo test <test_name>

Invariants to preserve:
  - <constraints from GOTCHAS.md or CLAUDE.md>

Assumptions to validate:
  - <what must be true> — validated by: <cheapest check>

Logging plan (required):
  - Every new AppRequest/HostEffect/DrawCommand handler → log::info! at entry
  - Every user-visible state change → log::info! with what changed
  - Every early-return bail-out → log::warn! naming app/command/reason
  - Every unrecoverable failure → log::error! with full context
```

---

## Phase 4 — Implement

> **Worktree path rule:** All Read/Edit/Write tool calls must target the WORKTREE absolute path. Never the repo root.

**Scope gate:**
- ≤ 3 files: implement inline. Write tests first, run `cargo test`.
- > 3 files or multiple subsystems: dispatch Sonnet subagent.

### Subagent dispatch

Prompt must include: full implementation spec, contents of every file it will touch, worktree path, and the rule "stage changes but do NOT commit."

Report back: `DONE` | `DONE_WITH_CONCERNS <details>` | `NEEDS_CONTEXT <what>` | `BLOCKED <reason>`

- `DONE`: review staged diff, run `cargo test`, commit.
- `DONE_WITH_CONCERNS`: read concerns first.
- `NEEDS_CONTEXT`: provide + redispatch.
- `BLOCKED`: provide context or escalate.

**Orchestrator owns the commit:**
```bash
git -C worktrees/<branch> add <files>
git -C worktrees/<branch> commit -m "<message>"
git -C worktrees/<branch> push -u origin HEAD
```

**Bundle mode:** branch name `feature/bundle-<n1>-<n2>`, implement all issues before pushing.

---

## Ship Log Format

After pushing, append this section to the issue body. If a `## Ship Log` section already exists (prior attempt), append a new entry under it. If not, add the section.

```markdown
## Ship Log

### Attempt <N> — <YYYY-MM-DD>
**Branch:** feature/<n>-short-description
**Files changed:** <list key files>
**Spec summary:** <one-line description of approach>
```

Append with:
```bash
CURRENT_BODY=$(gh issue view <number> --json body --jq '.body')
ATTEMPT_N=$(printf '%s' "$CURRENT_BODY" | grep -c '^### Attempt ' || true)
ATTEMPT_N=$((ATTEMPT_N + 1))
ATTEMPT_BLOCK=$(printf '### Attempt %s — %s\n**Branch:** feature/<branch>\n**Files changed:** <files>\n**Spec summary:** <summary>' "$ATTEMPT_N" "$(date +%Y-%m-%d)")
if printf '%s' "$CURRENT_BODY" | grep -q '^## Ship Log$'; then
  NEW_BODY=$(printf '%s\n\n%s' "$CURRENT_BODY" "$ATTEMPT_BLOCK")
else
  NEW_BODY=$(printf '%s\n\n## Ship Log\n\n%s' "$CURRENT_BODY" "$ATTEMPT_BLOCK")
fi
gh issue edit <number> --body "$NEW_BODY"
```

---

## Pipeline Labels

After pushing and writing the Ship Log, set pipeline state:

```bash
gh issue edit <number> \
  --add-label "pipeline:open-pr" \
  --add-label "ready" \
  --remove-label "pipeline:implement" \
  --remove-label "in progress"
```

This is the only handoff mechanism. Never spawn a new pane or output "Next: /open-pr" as an instruction — PM reads the label and dispatches.

---


## Abort / Stash Pop

At every exit point (success, blocked, fail):
```bash
[ "$IMPL_STASHED" = "true" ] && git stash pop
```

---

## Rules

- When a specific issue number is given, skip Phase 0 entirely and skip Phase 1b
- If the issue body contains a complete `## Action Plan` with named files, trust it. Do not re-read and re-grep those files to re-derive the plan.
- Never branch from main — always from repo root (alpha)
- Never skip base verification after `wtp add`
- No `todo!()` or `unimplemented!()` outside `#[cfg(test)]`
- No `#[allow(dead_code)]` or `#[allow(unused)]`
- `cargo build` must pass after all changes
- Subagents stage only — orchestrator owns the commit
- Never dispatch subagent without the Phase 3 spec
- Every implementation needs a logging plan
- CLI changes must update `~/.claude/skills/plexi-cli/SKILL.md` in same PR
- `cargo build --manifest-path <worktree>/Cargo.toml` — never rely on CWD
- On unrecoverable failure: close PR if open, comment on issue under `## Prior Attempts`, remove `in progress`, add `ready`, exit
