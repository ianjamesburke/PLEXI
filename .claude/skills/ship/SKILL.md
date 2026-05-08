---
name: ship
description: "Full PLEXI ship cycle. Three modes: /ship (auto-find next issue), /ship <issue-number> (specific issue), /ship <priority> (e.g. /ship P1)."
risk: medium
source: local
date_added: "2026-05-03"
---

# Ship

The full development lifecycle for PLEXI. One skill, three entry points:

| Invocation | Entry point |
|---|---|
| `/ship` | Auto-find next unblocked issue (P0→P1→P2→P3→P4) → Phase 0 |
| `/ship <issue-number>` | Start from a specific issue → Phase 1 |
| `/ship P0` / `/ship P1` / `/ship P2` / `/ship P3` / `/ship P4` | Find first unblocked issue at that level → Phase 0 |

---

## Phase 0 — Find the Issue

Run in parallel:
```bash
git log --oneline -10
gh issue list --label "in progress" --json number,title
```

Surface the in-progress list as context:
```
Currently in progress:
- #<n> — <title>
```

**Determine the search scope from the argument:**
- No arg → try P0, then P1 (no `ready` filter — these are must-fix), then P2 → P3 → P4 filtered to `ready` label only
- Priority arg (e.g. `P2`) → search only that level; apply `ready` filter for P2 and below

Fetch issues at the target priority:
```bash
# For P0 or P1:
gh issue list --label "<priority>" --state open --json number,title,labels,body --limit 50

# For P2, P3, P4:
gh issue list --label "<priority>" --label "ready" --state open --json number,title,labels,body --limit 50
```

Sort by issue number ascending. For each issue:
1. **Skip** if labeled `in progress`
2. **If labeled `blocked`:** extract `depends_on` from the front matter:
   ```bash
   gh issue view <n> --json body --jq '.body | match("depends_on: \\[(?P<deps>[^\\]]*)\\]") | .captures[0].string'
   # → "538, 541"  (empty string means no dependencies declared)
   ```
   - For each number in the result: `gh issue view N --json state,labels --jq '{state: .state, labels: [.labels[].name]}'`
   - If any dependency is `OPEN`:
     - Check if that dependency is labeled `in progress`
     - If **yes — dependency is in progress:** wait up to 3 × 5-minute intervals:
       1. Announce: "Dependency #<n> is in progress — waiting up to 15 minutes for it to land (check 1/3)."
       2. Wait 5 minutes, then re-check: `gh issue view <n> --json state --jq '.state'`
       3. If `CLOSED`: proceed — re-evaluate all deps and continue to Phase 1
       4. If still `OPEN`: repeat (checks 2/3 and 3/3). After 3 failed checks, skip this issue and move on.
     - If **not in progress (just open and blocked):** skip this issue immediately
   - If **all** are `CLOSED`: strip `blocked`, add `ready`:
     ```bash
     gh issue edit <n> --remove-label "blocked" --add-label "ready"
     ```
3. **If the body contains "Do not implement here":** skip — this is an epic tracker, not an implementation issue.
4. **If the body contains a "Depends on" or "Depends on:" section listing open issues (even without a `blocked` label):** skip and note it as an unlabeled dependency gap — add `blocked` label if all listed deps are still open:
   ```bash
   gh issue edit <n> --add-label "blocked"
   ```
5. **If not blocked or skipped:** this is the target → proceed to Phase 1

If the current priority level has no unblocked issues and the invocation was no-arg, advance to the next priority level and repeat.

**Front matter convention** — every issue body opens with:
```
---
depends_on: []
---
```
The `blocked` label is the fast filter; front matter is the data. Only parse body when `blocked` is present.

---

## Phase 1 — Pre-flight

Run in parallel:
```bash
git fetch origin
git status --porcelain
git log origin/alpha..HEAD --oneline
```

Handle each state automatically — do not stop and ask:

- **Dirty (uncommitted changes):** Stage and commit everything on alpha with `git add -A && git commit -m "chore: commit alpha changes before branching"`, then pull.
- **Local commits ahead of origin:** If they're chore/release commits (`chore: release`, `chore: bump`), push them with `git push`. If they look like real unmerged feature work, surface to the user and ask before proceeding.
- **Clean and synced:** `git pull --rebase origin alpha` to confirm up-to-date.

Mark the issue in progress and update the pane title:
```bash
gh issue edit <number> --add-label "in progress"
plexi pane set-title "#<number> — <short-title>"
```

---

## Phase 2 — Worktree Setup

Run from the repo root:
```bash
wtp add -b feature/<issue-number>-short-description
```

**If `wtp add -b` fails with "branch already exists":** check whether a worktree is already open for it (`git worktree list`). If not, add without `-b`: `wtp add feature/<issue-number>-short-description`. Then run `git log --oneline -5` on the branch to surface any prior commits — treat them as partial implementation to review in Phase 3 rather than starting from scratch.

**Immediately verify the base:**
```bash
git -C worktrees/<branch> log --oneline -1
git log --oneline -1
```

If they don't match: delete the worktree and branch, redo from the repo root. Never proceed on the wrong base.

---

## Phase 3 — Formulate

Before writing any code, read the issue and the relevant codebase to produce a tight implementation spec. This step is mandatory — it is what makes subagent dispatch reliable.

> **No feature ships without log visibility.** Every new capability, command, or user-visible behavior must emit at least one `info`-level trace confirming it ran. This is not optional polish — it's the first diagnostic tool when something breaks in testing or production. If the implementation spec has no logging plan, it's incomplete.

**Read in parallel:**
- Full issue body: `gh issue view <number> --json title,body,labels`
- Relevant source files: grep for the affected modules, then read them

**Assess scope:** count the files that will likely change.

**If the issue involves a third-party library or framework** (egui, egui_tiles, tokio, etc.) and the expected behavior isn't immediately obvious from the code: invoke the `coding-conventions` skill and read the relevant section before speculating on the approach. Unexpected API behavior not yet documented there is a signal to add it via `/improve` after the session.

**Write an implementation spec** (keep it in your context, do not write to disk):
```
Files to change:
  - src/foo.rs:42 — <exactly what>
  - src/bar.rs:10 — <exactly what>

Files NOT to touch:
  - <list any that are adjacent but out of scope>

Test that must pass:
  - cargo test <test_name>   (or: write HostHarness test for <thing>)

Invariants to preserve:
  - <any non-obvious constraints from GOTCHAS.md or CLAUDE.md>

Assumptions to validate:
  - <what must be true> — validated by: <cheapest CLI/log/source check>
  - Keybinds reach the app (not eaten by macOS system shortcuts) — validated by: log line in poll_actions confirming event delivery
  (Run each check NOW, before writing any code. A failed assumption here is cheaper than a failed PR.)

Logging plan (required):
  - Every new HostCommand/HostEffect/DrawCommand handler → log::info! at entry
  - Every user-visible state change → log::info! with what changed
  - Every early-return bail-out → log::warn! naming the app/command and reason
  - Every unrecoverable failure → log::error! with full context
  - App/SDK: ctx.info() or emit.info() at init, key actions, and errors
  No new capability, command, or user-visible behavior ships without at least
  one info-level trace confirming it ran.

Rules (always):
  - No todo!() or unimplemented!() outside #[cfg(test)]
  - No #[allow(dead_code)] or #[allow(unused)]
  - cargo build must pass after all changes
```

---

## Phase 4 — Implement

**Scope gate:**
- **≤ 3 files:** implement inline in the feature worktree. Write tests first, run `cargo test`.
- **> 3 files or multiple subsystems:** dispatch a Sonnet subagent.

### Subagent dispatch

Construct the subagent prompt with everything it needs — do not make it explore:
- The implementation spec (full text from Phase 3)
- Contents of every file it will touch (paste them inline)
- Worktree path: `worktrees/<branch>/`
- Explicit rules: write tests first, run `cargo test`, stage changes but do NOT commit
- Report back one of: `DONE` | `DONE_WITH_CONCERNS <details>` | `NEEDS_CONTEXT <what>` | `BLOCKED <reason>`

**Handle subagent status:**
- `DONE`: review staged diff, run `cargo test`, then commit.
- `DONE_WITH_CONCERNS`: read the concerns first. If correctness/scope — address before committing. If observations — note and commit.
- `NEEDS_CONTEXT`: provide the missing context, redispatch with same model.
- `BLOCKED`: if context problem — provide more + redispatch. If task too large — break it down. If plan is wrong — escalate to user.

**Never** ignore an escalation or force a retry without changes.

**Orchestrator owns the commit.** Subagent stages only. After verifying the diff and `cargo test` is green:
```bash
git -C worktrees/<branch> add <files>
git -C worktrees/<branch> commit -m "<message>"
git -C worktrees/<branch> push -u origin HEAD
```

When done, open a PR targeting `alpha`:
```bash
gh pr create --base alpha --title "<title>" --body "..."
```

---

## Phase 4 — Install & Test

**Run the install and confirm it completes before writing the testing block.** Do not surface the testing block while the install is pending or if it hasn't been run yet.

Run from inside the **feature worktree**:
```bash
just pr-install <pr-number>
```

> **CWD check:** If your shell is at the repo root, `cd` to the feature worktree first — `just pr-install` runs `cargo bundle` from CWD, so running it from alpha silently builds the old code. A compile time under ~10s is proof the change wasn't picked up.

Installs as `/Applications/Plexi PR<number>.app` with isolated profile `~/.plexi-pr-<number>/`. Wait for it to complete.

**Note for justfile/config-only changes:** `just pr-install` installs the Python binary only — justfile or config changes are not visible in the repo root until after merge. For these changes, direct test steps to run from `worktrees/feature/<branch>/` instead.

**Before writing the testing block:** verify whether the PR build can actually exercise the feature's golden path. If the feature requires a configuration, environment, or runtime condition that the PR build cannot satisfy (e.g. a stable-only feature gated on a non-PR app directory, a capability requiring a device not present, a server-side dependency unavailable on the PR profile) — state that limitation explicitly at the top of the testing block as a `Note:` line rather than discovering it mid-instruction.

**Command formatting rule:** Any command you give the user to run must appear alone in its own code block — never inline inside prose. One command per block, nothing else on the line.

**PR build CLI rule:** When testing instructions require running a `plexi` CLI command, use `plexi-pr-<N>` (e.g. `plexi-pr-757 open terminal`), not the bare `plexi` command — `plexi` resolves to the stable build, not the PR build.

**Log verification rule:** Never include `tail` or log-reading commands in the user-facing testing block. If pass/fail criteria involve checking the log, read it yourself before surfacing the testing block:
```bash
tail -100 ~/.plexi-pr-<number>/plexi.log
```
If the log already confirms pass or fail, state your finding directly rather than asking the user to check. The user should never be asked to tail a log — that's the agent's job.

**Test fixture rule:** If testing requires a helper binary, shim, or fixture that can be installed in advance, install it yourself before surfacing the testing block. Remove it after the user confirms pass. Never include a multi-step shell heredoc in the testing instructions for the user to run — that's the agent's job.

Surface the testing block — output EXACTLY this format, then stop:

```
[TESTING] PR #<n> — <title>

Instructions:
1. <exact step>
2. <exact step>
3. <exact step>

Pass criteria:
- <concrete observable outcome>

Fail criteria:
- <concrete observable symptom>

Reply with one of:
- "pass" — criteria met, ready to merge
- "fail: <exact description of what went wrong>" — notes are required
- "modify: <specific change needed>" — only if pass criteria not yet met; must stay within original scope
```

**STOP. Do not proceed until the user replies.**

---

### Handling the Response

**Pass** → proceed to Phase 5.

**Modify** — valid only if the stated pass criteria were not fully met. Must name a single, bounded change within the original issue scope. If the pass criteria *were* met and the user wants additional tweaks (aesthetic, behavioral, ergonomic) beyond what was specified — those are **not a modify**. Push back: "Pass criteria are met. Filing that as a separate issue rather than extending this PR." Create the issue, then proceed to Phase 5. Do not let scope creep through the modify path.

Fix the specific change on the feature branch, re-run `just pr-install`, re-surface the testing block. Do not expand scope.

**Fail** — "fail" without a description is not accepted. Ask for it. The description is preserved verbatim in the issue comment so the next agent has full context.

**Step 1 — Check the logs first.** Before any diagnosis, read the PR build log to get visibility on exactly what happened:
```bash
tail -100 ~/.plexi-pr-<number>/plexi.log
```
This is the isolated profile for the PR build — look for `ERROR`, `WARN`, or the last few `INFO` lines before the failure. Report what the log shows before drawing any conclusions about root cause. If the log is empty or missing, note that explicitly — it may mean the app never launched or the profile dir wasn't created.

**Step 2 — Assess diff size:**
```bash
git -C worktrees/<branch> diff origin/alpha --stat | tail -1
```

**If diff is under ~1000 lines (automatic revert path):**
1. Close the PR: `gh pr close <pr-number> --comment "Closing — approach didn't work. See issue #<n> for full context."`
2. Remove labels, re-mark ready:
   ```bash
   gh issue edit <number> --remove-label "in progress" --add-label "ready"
   ```
3. Post a comment on the issue:
   ```bash
   gh issue comment <number> --body "..."
   ```
   Comment must include:
   - **Attempt:** PR #<pr-number> — `<title>`
   - **What was tried:** the implementation approach (files changed, strategy from Phase 3 spec)
   - **Failure:** verbatim description from the user
   - **Rules out:** what this attempt establishes as non-viable for the next attempt
4. `wtp remove <branch>` then `git push origin --delete <branch>`

**If diff is over ~1000 lines (conversation required):**
Do not revert without asking. Present options:

```
Diff is ~<N> lines — reverting would discard significant work. Options:

A. Convert PR to draft, add "waiting for redesign" label — branch stays open for reference
B. Close PR and delete the branch — clean slate
C. Keep PR open with a comment explaining what failed and what needs rethinking before it can land

Either way I'll post a full failure comment on the issue so the next agent has complete context.

Which do you prefer?
```

Regardless of option chosen: post the failure comment on the issue (same format as above). Remove `in progress` label. The issue stays open. Never close the issue on a fail.

---

## Phase 5 — Merge & Cleanup

After user confirms pass. Run without stopping:

1. Sync alpha — rebase handles divergence from parallel agent merges:
   ```bash
   git pull --rebase origin alpha   # from the repo root
   ```
   This handles all cases: behind (fast-forward replay), ahead (push after), diverged (rebase). If a rebase conflict occurs in GOTCHAS.md, keep all entries, newest-first.

2. Rebase the feature branch on origin/alpha and push:
   ```bash
   git -C worktrees/<branch> rebase origin/alpha
   # resolve conflicts if any, then:
   git -C worktrees/<branch> add <resolved-files>
   GIT_EDITOR=true git -C worktrees/<branch> rebase --continue
   git -C worktrees/<branch> push --force-with-lease origin HEAD
   ```

3. Discard PR bundle metadata and merge:
   ```bash
   git -C worktrees/<branch> restore Cargo.toml  # pr-install rewrites Cargo.toml — discard it
   gh pr merge <number> --squash
   ```
   If branch protection blocks the merge, use `--admin`:
   ```bash
   gh pr merge <number> --squash --admin
   ```

   If anything non-obvious happened during this PR — a failed approach, an environment constraint, a tool behavior that cost time — add one entry to GOTCHAS.md on alpha now. Write a detailed commit message explaining the why. Skip GOTCHAS if nothing surprised you.

4. `just pr-clean <pr-number>` — run from the repo root
> **CWD check:** The shell may still be inside the feature worktree. Use `cd /Users/ianburke/Documents/GitHub/PLEXI &&` as a prefix for all remaining Phase 5 commands, or confirm CWD with `pwd` first.
5. `git pull --rebase origin alpha` — from the repo root
6. `wtp remove <branch> --force` then `git push origin --delete <branch>`
7. `git status` in the repo root — linter or hook changes (e.g. CLAUDE.md) can dirty alpha between testing and merge. Stage and commit any such changes before bumping. Then `just bump && just install` — from the repo root
8. `git push` — push bump commit to origin so alpha is not diverged at next session start

---

## Phase 6 — Complete

```bash
gh issue close <number> --comment "Closed by PR #<pr> — verified on alpha <version>"
git status   # must be clean
```

**Unblock downstream issues:** Now that `#<number>` is closed, check for open `blocked` issues that listed it as a dependency:
```bash
gh issue list --label "blocked" --state open --json number,body --limit 100 \
  | jq --arg n "<number>" '.[] | select(.body | test("depends_on:.*\\b" + $n + "\\b"))'
```
For each match, re-check all its dependencies: `gh issue view N --json state --jq '.state'`. If all are `CLOSED`:
```bash
gh issue edit <n> --remove-label "blocked" --add-label "ready"
```
Report any issues unblocked as part of the completion output.

Run `/improve` to surface friction from this session and suggest improvements. Wait for the improve output to land.

**Before accepting any CLAUDE.md addition from `/improve`:** ask whether the lesson is better encoded as a code change (stricter types, better error messages, a guard in the workflow) rather than a rule in the file. CLAUDE.md is already long — prefer making the architecture harder to misuse over adding another line to memorize. A new CLAUDE.md rule is only warranted if there's no reasonable code-level enforcement.

After `/improve` completes, copy its `[IMPROVEMENTS]` bullets verbatim into the completion block. If the user declined all suggestions or `/improve` made no changes, omit `Improvements made:` entirely.

Output:
```
- Merged: PR #<n> — <title>
- Closed: Issue #<n> — <title>
- Version: <version>
- Improvements made:
  - <bullet from /improve's [IMPROVEMENTS] output>
  - <bullet from /improve's [IMPROVEMENTS] output>
  (omit if none)

[COMPLETE]
```

---

## Rules

- Never branch from main — always from the repo root
- Never skip base verification after `wtp add`
- Never merge before user confirms testing passed
- Never claim [COMPLETE] without user verification
- On branch protection: use `--admin` rather than rebasing just to satisfy "branch is behind" — only rebase for actual merge conflicts
- `just pr-install` runs from the **feature worktree**
- `just pr-clean`, `just bump`, and `just install` run from the **repo root**
- Cosmetic issues spotted during testing go in separate issues — not blockers
- "modify" is only valid if pass criteria were not yet met — not for post-pass polish or bonus improvements
- "fail" without a description is not accepted — ask for it before taking any action
- On fail: always post a failure comment on the issue before closing/reverting anything — the issue stays open
- Never close a failing issue — only close on merge
- Never pass `--delete-branch` to `gh pr merge` — git refuses to delete a branch checked out by a worktree
- Alpha must be clean when the cycle ends
- Subagents stage only — orchestrator owns the commit and the PR
- Never dispatch a subagent without first producing the Phase 3 implementation spec
- Every implementation must include a logging plan — no feature ships without info-level traces and warn-level bail-outs
