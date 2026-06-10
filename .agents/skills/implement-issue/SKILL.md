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
Pipeline: pipeline:open-pr + ready set — invoking /open-pr inline
```

> **Labels are the live state.** Never read the Ship Log to determine pipeline stage — read the issue labels. Ship Log is audit trail only.

> **Pane status title.** This skill runs in a dispatched pane named `#<n>` (or `#<n1>+<n2>` for a bundle). At each phase boundary, update the pane title with the current stage so the project-manager can read state from `plexi pane list` instead of capturing pane content. The call is:
> ```bash
> plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL} pane name "#<n> · <state>"
> ```
> Use the exact issue-number prefix the pane already has (`#<n1>+<n2>` for a bundle). **The status word must never contain a digit** — the PM maps panes to issues with `grep -oE '[0-9]+'` on the title, so a PR number in the suffix would corrupt the census. States this skill sets: `impl`, `pushed`, `noop`, `blocked`.

> **Stint timing is mandatory.** This skill opens linked stint work with `stint start <task-id>`. Do not close stint tasks here; `/merge-pr` runs `stint done <task-id>` after the PR merges and alpha is verified. Use `stint start --help` / `stint done --help` for exact flags instead of manually editing timing fields.

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
- No arg → try P0, P1, then P2→P4 (no `ready` filter at any level)
- Priority arg → search only that level

```bash
gh issue list --label "<priority>" --state open --json number,title,labels,body --limit 50
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

Run in parallel:
```bash
git fetch origin && git status --porcelain && git log origin/alpha..HEAD --oneline
gh issue view <number> --json state,title,body,labels
```

**Output immediately:**
```
ISSUE #<n>: <title>
```

Hold the full issue body in context — Phase 3 uses it directly. Do not re-fetch.

**Hard stops — check all before proceeding. Any failure exits immediately, no conditional paths:**

1. Unpushed commits on alpha → **allowed** as long as the working tree is clean. Log a warning: "NOTE: local alpha is ahead of origin — worktree will branch from local HEAD, not origin/alpha." Skip the `git pull --rebase` step below (pulling would try to reconcile unpushed local commits). Do NOT stop.
2. Dirty working tree → STOP. Print `git status --short`. Do not stash, do not proceed.
3. Issue is `CLOSED` → set pane `noop`, stop. "Issue #<n> is already closed."
4. Issue is labeled `in progress` → STOP. "ERROR: issue #<n> is already in progress." Surface existing worktree + any open PR. Do not proceed.

Then run in parallel:
```bash
# Only pull when in sync with origin (no unpushed commits). Skip when ahead — worktree already branches from local HEAD.
git pull --rebase origin alpha  # skip if local alpha is ahead of origin
git ls-remote origin "refs/heads/feature/<issue-number>-*"
```

If ls-remote is non-empty: another agent owns this branch. Surface it and any existing PR, then stop.

**Label + pane setup + worktree creation — run in sequence:**
```bash
gh issue edit <number> --add-label "in progress" --add-label "pipeline:implement"
plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL} pane name "#<number> · impl"
IMPL_PANE=$PLEXI_PANE_ID
wtp add -b feature/<issue-number>-short-description HEAD  # origin/alpha when in sync; local HEAD when ahead
```

**Start linked stint tasks immediately after the issue enters implementation:**

1. Find tasks whose frontmatter links the issue:
   ```bash
   rg -l 'gh_issue: .*"<number>"|gh_issue: .*\\[<number>\\]' .stint/tasks
   ```
2. For each linked task that will be materially worked, run:
   ```bash
   stint start <task-id>
   ```
3. Do not use `--restart` unless deliberately replacing bad timing data; normal resumed work keeps the original `started_at`.
4. For historical backfill only, use `stint start <task-id> --started-at <UTC-RFC3339>`.
5. If no linked task exists, continue but note the missing stint linkage in the issue Ship Log; do not invent timing only in GitHub.

If "branch already exists": check `git worktree list`. If no worktree, `wtp add` without `-b`. Check for prior commits.

**Verify base immediately:**
```bash
git -C worktrees/<branch> log --oneline -1
git log --oneline -1
```
If mismatch: delete worktree and branch, redo.

---

## Phase 2 — Formulate

Before writing code, produce a tight implementation spec.

**Issue body is already in context from Phase 1. Do not re-fetch.**
For bundles: fetch the additional issue bodies in parallel now:
```bash
gh issue view <n2> --json title,body,labels > /tmp/issue_n2.json &
# ... one per additional issue
wait && cat /tmp/issue_n*.json && rm /tmp/issue_n*.json
```

**Implementation Map — verify, then trust.** If the issue body has an `## Implementation Map` section, it lists the exact files to touch (written by `/create-issue` during its codebase research). Do NOT re-run the broad grep/discovery sweep. Instead:

1. Cheap existence check — one batched pass confirming each mapped path still exists and each named symbol is still present:
   ```bash
   git -C worktrees/<branch> ls-files <path1> <path2> ...
   grep -rn "<fn1>\|<fn2>" worktrees/<branch>/<mapped-files>
   ```
2. Every path/symbol resolves → read ONLY the mapped files. Skip GOTCHAS grep. Common case; skips discovery entirely.
3. A path is missing, a symbol moved, or the map is absent → the issue drifted since filing. Fall back to full discovery (grep affected modules, then read) for the unresolved parts only, note "Map stale: <what moved>" in the spec, **and** run the GOTCHAS grep below.

**Grep GOTCHAS.md — only when doing full discovery (step 3 above):**
```bash
grep -in "<term1>\|<term2>\|<term3>" GOTCHAS.md
```
Surface every match under **Gotchas found:**. Each requires disposition: **NOTED** or **N/A**.

**If any changed file is a Python app under `apps/` or `apps/dev/`:** invoke `create-plexi-app` skill before writing code.

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

## Phase 3 — Implement

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

**Bundle mode:** branch name `feature/bundle-<n1>-<n2>`, implement and commit issues sequentially so each commit is independently bisectable:

1. For each issue (N1, N2, ...), implement changes and commit individually:
   ```bash
   git -C worktrees/<branch> add <files>
   git -C worktrees/<branch> commit -m "fix/feat: <issue description> (#<issue_number>)"
   ```
2. Push all commits: `git -C worktrees/<branch> push -u origin HEAD`
3. After pushing, write a Ship Log entry to **each** issue body (see Ship Log Format).
4. Set Pipeline Labels on ALL N issues (see Pipeline Labels).

---

## Ship Log Format

After pushing, append this section to the issue body. In bundle mode, write a Ship Log entry to **each** issue body. If a `## Ship Log` section already exists (prior attempt), append a new entry under it. If not, add the section.

Do not run `stint done` in this skill. A pushed implementation is not task completion; validation and merge can still fail. `/merge-pr` closes linked stint tasks after the PR merges and alpha is verified.

```markdown
## Ship Log

### Attempt <N> — <YYYY-MM-DD>
**Branch:** feature/<n>-short-description
**Stint:** started <task-id> at <UTC ISO-8601 timestamp>, or missing linked task
**Files changed:** <list key files>
**Spec summary:** <one-line description of approach>
```

Append with:
```bash
CURRENT_BODY=$(gh issue view <number> --json body --jq '.body')
ATTEMPT_N=$(printf '%s' "$CURRENT_BODY" | grep -c '^### Attempt ' || true)
ATTEMPT_N=$((ATTEMPT_N + 1))
ATTEMPT_BLOCK=$(printf '### Attempt %s — %s\n**Branch:** feature/<branch>\n**Stint:** started <task-id> at <started_at>, or missing linked task\n**Files changed:** <files>\n**Spec summary:** <summary>' "$ATTEMPT_N" "$(date +%Y-%m-%d)")
if printf '%s' "$CURRENT_BODY" | grep -q '^## Ship Log$'; then
  NEW_BODY=$(printf '%s\n\n%s' "$CURRENT_BODY" "$ATTEMPT_BLOCK")
else
  NEW_BODY=$(printf '%s\n\n## Ship Log\n\n%s' "$CURRENT_BODY" "$ATTEMPT_BLOCK")
fi
gh issue edit <number> --body "$NEW_BODY"
```

---

## Pipeline Labels

After pushing and writing the Ship Log, set pipeline state on **every** issue in the bundle (or the single issue in single-issue mode):

```bash
# Repeat for each issue number N:
gh issue edit <N> \
  --add-label "pipeline:open-pr" \
  --add-label "ready" \
  --remove-label "pipeline:implement" \
  --remove-label "in progress"
```

Set the pane status to `pushed`, then invoke `/open-pr` inline in the same pane — do not spawn a new pane or wait for PM to dispatch:
```bash
plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL} pane name "#<n> · pushed"   # bundle: "#<n1>+<n2> · pushed"
```

---


## Rules

- When a specific issue number is given, skip Phase 0 entirely
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
- Start linked stint tasks here; never mark them done here. `/merge-pr` owns completion timing.
- On unrecoverable failure: set `plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL} pane name "#<n> · blocked"`, close PR if open, comment on issue under `## Prior Attempts`, remove `in progress`, add `ready`, exit
