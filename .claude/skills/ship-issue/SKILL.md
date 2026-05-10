---
name: ship-issue
description: "Full PLEXI ship cycle. Three modes: /ship-issue (auto-find next issue), /ship-issue <issue-number> (specific issue), /ship-issue <priority> (e.g. /ship-issue P1)."
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

**First — check issue state before touching anything:**
```bash
gh issue view <number> --json state,title --jq '{state: .state, title: .title}'
```
If state is `CLOSED`: stop immediately. Tell the user: "Issue #<n> is already closed — nothing to do." Do NOT add labels, create worktrees, or proceed further.

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

Mark the issue in progress, capture the origin pane ID, and update the pane title:
```bash
gh issue edit <number> --add-label "in progress"
SHIP_PANE=$(plexi pane list | python3 -c "import json,sys; print(next(p['id'] for p in json.load(sys.stdin) if p['focused']))")
plexi pane set-title "#<number> — <short-title>"
```

Hold `$SHIP_PANE` for the rest of the cycle — every decision-point and end-of-run notification uses it as the `pane_focus` target so the user can route back to this conversation from the notification UI.

---

## Phase 1b — Implementation Audit

Before creating a worktree, check whether the work is already done or partially landed on alpha.

Grep alpha `src/` for the key identifiers from the issue's **Done When** criteria. Scan `git log --oneline -20` for related commits.

- **All criteria already met:** close the issue, remove `in progress`, skip to Phase 6.
- **Partial or ambiguous:** surface what's done vs missing, state the plan, and ask any open design questions. Wait for confirmation before branching.
- **Nothing done and no questions:** proceed, but note "Audit: nothing on alpha yet."

---

## Phase 2 — Worktree Setup

Run from the repo root:
```bash
wtp add -b feature/<issue-number>-short-description
```

**If `wtp add -b` fails with "branch already exists":** check whether a worktree is already open for it (`git worktree list`). If not, add without `-b`: `wtp add feature/<issue-number>-short-description`. Then run `git log --oneline -5` on the branch to surface any prior commits — treat them as partial implementation to review in Phase 3 rather than starting from scratch.

**If rebase of an existing branch leaves `git log origin/alpha..HEAD` empty** (all commits were skipped as already upstream): the feature is already on alpha. Skip to cleanup — remove the worktree, delete the branch, and close the issue. Do not look for new work to commit on this branch.

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

**When prior commits exist on the branch:** Before reading worktree files, grep the issue's key identifiers (field names, function names listed in "Files to change" or "Done when") directly on alpha (`src/` in the repo root). If a grep hit confirms an identifier already exists on alpha, that criterion is already implemented — skip reading its worktree file and focus Phase 3 only on what's genuinely missing. This avoids reading files that are identical to alpha and misreading branch-vs-alpha diffs.

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

When done, open a PR targeting `alpha` and update the pane title with the PR number so both the issue and PR are searchable:
```bash
PR_URL=$(gh pr create --base alpha --title "<title>" --body "...")
PR_NUMBER=$(echo "$PR_URL" | grep -oE '[0-9]+$')
plexi pane set-title "#<issue-number> / PR #${PR_NUMBER} — <short-title>"
```

---

## Phase 4b — AI Review

Run after PR creation, before installing the PR build.

**1. CodeRabbit local review** — run from inside the feature worktree:
```bash
cr 2>&1
```
Read the output. Fix any `error` or `warning` severity findings on the feature branch before proceeding — these are blocking. `info`/`suggestion` items are advisory; apply if trivial, skip if out of scope. If fixes were made, commit, push, then re-run to confirm clean.

**2. Poll for Gemini / CodeRabbit bot feedback** — wait 5 minutes after PR creation, then check every 60s up to the 10-minute mark (6 checks total).

> **Note:** Gemini posts as a PR **review** (not a comment). Check both endpoints — `gh pr view --json comments` only surfaces issue-style comments; inline and summary bot feedback lands in `pulls/<n>/reviews` and `pulls/<n>/comments`.

```bash
echo "Waiting 5 minutes for AI reviewers to post..."
sleep 300
for i in $(seq 1 6); do
  COMMENT_COUNT=$(gh pr view $PR_NUMBER --json comments \
    --jq '[.comments[] | select(.author.login | test("gemini|coderabbit"; "i"))] | length')
  REVIEW_COUNT=$(gh api repos/ianjamesburke/PLEXI/pulls/$PR_NUMBER/reviews \
    --jq '[.[] | select(.user.login | test("gemini|coderabbit"; "i"))] | length')
  INLINE_COUNT=$(gh api repos/ianjamesburke/PLEXI/pulls/$PR_NUMBER/comments \
    --jq '[.[] | select(.user.login | test("gemini|coderabbit"; "i"))] | length')
  TOTAL=$((COMMENT_COUNT + REVIEW_COUNT + INLINE_COUNT))
  if [ "$TOTAL" -gt 0 ]; then
    echo "Found bot feedback ($COMMENT_COUNT comments, $REVIEW_COUNT reviews, $INLINE_COUNT inline):"
    gh pr view $PR_NUMBER --json comments \
      --jq '.comments[] | select(.author.login | test("gemini|coderabbit"; "i")) | "[\(.author.login)] \(.body)"'
    gh api repos/ianjamesburke/PLEXI/pulls/$PR_NUMBER/reviews \
      --jq '.[] | select(.user.login | test("gemini|coderabbit"; "i")) | "[\(.user.login)] \(.body)"'
    gh api repos/ianjamesburke/PLEXI/pulls/$PR_NUMBER/comments \
      --jq '.[] | select(.user.login | test("gemini|coderabbit"; "i")) | "[\(.user.login)] \(.body)"'
    break
  fi
  if [ "$i" -lt 6 ]; then
    echo "AI review check $i/6 — no bot feedback yet, waiting 60s..."
    sleep 60
  fi
done
```
After the loop, read all surfaced feedback. For each:
- **Correctness / bug / type safety** → fix on the feature branch, commit, push
- **Style / naming / docs** → apply if trivial, skip if out of scope
- **False positive** → note and ignore

If any fixes were applied from bot feedback, re-run the CodeRabbit local review to confirm clean before proceeding:
```bash
cr 2>&1
```
Fix any remaining `error` or `warning` findings, commit, push, then continue.

**The final approver is always the user in this Claude Code instance.** AI/bot reviews inform the fix pass, but no PR merges without explicit user confirmation in Phase 4.

If no bot feedback appears after 10 minutes, proceed — the bots may be slow or not configured for this repo.

**Required: emit `[AI REVIEW]` block before proceeding to install/test.** This is mandatory — never silently skip to the next phase. Output exactly this format:

```
[AI REVIEW] PR #<n>

CodeRabbit (local):
  <one bullet per finding, or "No findings.">
  - [error/warning/info] <short description> → FIXED: <what changed> | SKIPPED: <reason> | N/A (false positive)

Gemini / bot:
  <one bullet per finding, or "No feedback received.">
  - [<severity>] <short description> → FIXED: <what changed> | SKIPPED: <reason> | N/A (false positive)

Net: <N> fix(es) committed | no changes needed
```

Rules for the block:
- Every finding gets a one-line disposition — FIXED, SKIPPED, or N/A. No vague "addressed" or "reviewed".
- FIXED means code was changed, committed, and pushed. Name the file/function.
- SKIPPED means deliberately ignored. Give the reason in one clause (e.g. "out of scope", "stylistic only", "pre-existing issue").
- N/A means a false positive — explain why in one clause.
- If `cr` was not available or produced no output, say "cr not available" or "No findings."
- If no bot feedback arrived within 10 minutes, say "No feedback received (10 min timeout)."

---

## Phase 4 — Install & Test

### Install gate

Before running `just pr-install`, assess whether a binary install is actually needed.

**Diff-review only (skip install) when all changed files fall into these categories:**
- Non-Rust files only: justfile, TOML config, scripts, docs, skills, completion files, markdown
- Rust changes that are pure code deletion with no behavioral change
- Rust test-only additions (`#[cfg(test)]` or `tests/` only, no production path changes)
- CLI output changes verifiable without a running host (help text, error messages, completions)

Check the diff:
```bash
git -C worktrees/<branch> diff origin/alpha --name-only
```

**If install is not needed:** surface a diff-review testing block instead (see "Diff-review testing block" below). Do not run `just pr-install`. Skip the pr-install and pr-clean steps in Phase 5 as well.

**If install is needed:** proceed below.

---

**Run the install and confirm it completes before writing the testing block.** Do not surface the testing block while the install is pending or if it hasn't been run yet.

Run from inside the **feature worktree**:
```bash
just pr-install <pr-number>
```

> **CWD check:** If your shell is at the repo root, `cd` to the feature worktree first — `just pr-install` runs `cargo bundle` from CWD, so running it from alpha silently builds the old code. A compile time under ~10s is proof the change wasn't picked up.

> **Compile check:** If `just pr-install` output shows no `Compiling plexi` line, the release binary was cached and the Rust source change was not picked up. Run `touch src/<changed-file>.rs` then `just pr-install <N>` again to force a recompile. A compile time under ~10s is proof the change wasn't picked up.

Installs as `/Applications/Plexi PR<number>.app` with isolated profile `~/.plexi-pr-<number>/`. Wait for it to complete.

**Note for justfile/config-only changes:** `just pr-install` installs the Python binary only — justfile or config changes are not visible in the repo root until after merge. For these changes, direct test steps to run from `worktrees/feature/<branch>/` instead.

**Before writing the testing block:** scan `examples/` in the feature worktree — the subagent may have already added a POC app for this feature as part of the implementation. If one exists, use it directly rather than writing a new test app. If the fix changes event delivery or routing (key events, mouse events, protocol messages), read the source of any example app being used for testing — it may have key bindings or handlers that silently encoded assumptions about the old (broken) behavior, making the test appear to fail when the fix is correct. Then verify whether the PR build can actually exercise the feature's golden path. If the feature requires a configuration, environment, or runtime condition that the PR build cannot satisfy (e.g. a stable-only feature gated on a non-PR app directory, a capability requiring a device not present, a server-side dependency unavailable on the PR profile) — state that limitation explicitly at the top of the testing block as a `Note:` line rather than discovering it mid-instruction.

**Command formatting rule:** Any command you give the user to run must appear alone in its own code block — never inline inside prose. One command per block, nothing else on the line.

**PR build CLI rule:** When testing instructions require running a `plexi` CLI command, use `plexi-pr-<N>` (e.g. `plexi-pr-757 open terminal`), not the bare `plexi` command — `plexi` resolves to the stable build, not the PR build.

**Log verification rule:** Never include `tail` or log-reading commands in the user-facing testing block. If pass/fail criteria involve checking the log, read it yourself before surfacing the testing block:
```bash
tail -100 ~/.plexi-pr-<number>/plexi.log
```
If the log already confirms pass or fail, state your finding directly rather than asking the user to check. The user should never be asked to tail a log — that's the agent's job.

**Test fixture rule:** If testing requires a helper binary, shim, or fixture that can be installed in advance, install it yourself before surfacing the testing block. Remove it after the user confirms pass. Never include a multi-step shell heredoc in the testing instructions for the user to run — that's the agent's job.

**CLI output verification rule:** Before including any command in the testing block, run it yourself with `plexi-pr-<N>` and verify the output is what the user will see — no log noise, unexpected errors, or extraneous text mixed into stdout. If the output is noisy, investigate and note it in the testing block rather than letting the user discover it.

**Test discrimination rule:** For bug-fix PRs, the test command must produce *different* output in the broken vs fixed case — not just confirm the feature ran. Before adding any command to the testing block, mentally simulate what the broken behavior would output. `echo 'a b'` and `echo a b` produce identical output, making it useless for arg-splitting bugs; prefer `printf '%s\n' "a b"` (shows 1 line fixed vs 2 broken) or `python3 -c "import sys; print(len(sys.argv)-1)"` to count args.

**CLI self-test rule:** For changes where behavior is verifiable without a running Plexi host (completions output, help text, arg parsing, command exit codes, stdout format), run those commands yourself using `plexi-pr-<N>` after install and report the result directly. Do not surface a testing block asking the user to run a pure CLI command you could have run yourself. Reserve the testing block for behavior that genuinely requires human eyes or a running app (UI, notifications, terminal pane output, socket-dependent responses).

**Python test script rule:** For any PR where the user must run more than one command, or where copy-pasting commands across panes is awkward, write a `test_pr<N>.py` at the repo root instead of a markdown testing block. The script:
- Runs all non-interactive checks itself (CLI exit codes, error message content, log assertions via `tail`)
- Emits `PASS` / `FAIL` per check with color, and a final summary
- Blocks only on genuinely interactive steps (e.g. clicking a notification button) with clear printed instructions
- At the end, fires `plexi notify` to pull the user back to the ship pane with the result
- Is deleted by the agent in Phase 5 (before the bump commit) — never committed

```python
# Template structure:
import subprocess, json, sys
CLI = "plexi-pr-<N>"
PASS = "\033[32mPASS\033[0m"; FAIL = "\033[31mFAIL\033[0m"
failures = []
def check(label, ok, detail=""):
    if ok: print(f"  {PASS}  {label}")
    else: print(f"  {FAIL}  {label}{': ' + detail if detail else ''}"); failures.append(label)
# ... checks ...
if failures: sys.exit(1)
```

Tell the user to run it with:
```
python3 test_pr<N>.py
```

**Diff-review testing block** (used when install is not needed):
```
[TESTING] PR #<n> — <title>

No binary install needed for this change.

Please review the diff and confirm it looks correct:
<pr-url>

Pass criteria:
- <concrete observable change visible in the diff>

Fail criteria:
- <what would look wrong in the diff>

Reply with one of:
- "pass" — diff looks correct, ready to merge
- "fail: <exact description of what's wrong>"
- "modify: <specific change needed>"
```

Then send the notification:
```bash
RESULT=$(plexi notify --title "PR #<n> ready to review" \
  --body "<title>. No install needed — just check the diff." \
  --choice "a:Talk to Claude:pane_focus:$SHIP_PANE" \
  --choice "b:Open PR" \
  --choice "c:Open PR")

case "$RESULT" in
  b|c) open "<pr-url>" ;;
esac
```

**STOP. Do not proceed until the user replies with pass/fail/modify.**

---

Surface the testing block — output EXACTLY this format:

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

Then send a synchronous notification so the user gets pulled in only when ready to test:
```bash
RESULT=$(plexi notify --title "PR #<n> ready to test" \
  --body "<title>. Pass criteria: <one-line summary>. Reply pass/fail/modify in pane." \
  --choice "a:Talk to Claude:pane_focus:$SHIP_PANE" \
  --choice "b:Open PR build" \
  --choice "c:Open PR")

case "$RESULT" in
  b) open -a "Plexi PR<n>" ;;
  c) open "<pr-url>" ;;
esac
```
Choice `c` is the only host-side action (focuses this pane). `a` and `b` return their key; the case statement runs the corresponding shell command. After the click, **STOP. Do not proceed until the user replies in the pane with pass/fail/modify.**

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
Do not revert without asking. Present options in chat AND fire a synchronous notification with the 3-choice convention:

```
Diff is ~<N> lines — reverting would discard significant work. Options:

A. Convert PR to draft, add "waiting for redesign" label — branch stays open for reference
B. Close PR and delete the branch — clean slate
C. Keep PR open with a comment explaining what failed and what needs rethinking before it can land

Either way I'll post a full failure comment on the issue so the next agent has complete context.
```

```bash
plexi notify --title "PR #<n> failed — ~<N> line diff" \
  --body "<title>. Reverting discards significant work. Pick a path." \
  --choice "a:Talk to Claude:pane_focus:$SHIP_PANE" \
  --choice "b:Draft + waiting-for-redesign" \
  --choice "c:Close + delete branch"
```
The notify return value (`a`/`b`/`c`) drives the next action directly — no follow-up chat reply needed for `a` or `b`. `c` pulls the user back to this pane for the conversation.

Regardless of option chosen: post the failure comment on the issue (same format as above). Remove `in progress` label. The issue stays open. Never close the issue on a fail.

---

## Phase 5 — Merge & Cleanup

After user confirms pass. Run without stopping.

**CWD for all Phase 5 commands: the repo root.** Start Phase 5 by running this — no exceptions:
```bash
cd /Users/ianburke/Documents/GitHub/PLEXI
```
This is mandatory, not a suggestion. Shell CWD drifts during Phase 4 testing (feature worktree installs, log reads). A Phase 5 `git` or `just` command that runs from the wrong directory silently corrupts the alpha branch state.

**0. Check PR state:**
```bash
gh pr view <number> --json state --jq '.state'
```
If `MERGED`: skip to step 4. Another session already merged — don't re-merge.

**1. Discard artifacts, stash local alpha edits:**
```bash
git restore Cargo.toml                              # discard pr-install artifact
git -C worktrees/<branch> restore Cargo.toml        # discard from feature worktree too
DIRTY=$(git status --porcelain | grep -v "^??" | grep -v "Cargo.toml")
if [ -n "$DIRTY" ]; then
  git stash push -m "session edits — restore after merge"
  STASHED=1
fi
```
> Stash instead of commit: a pre-merge commit on alpha gets replayed by any subsequent `--rebase` and conflicts with the squash content. Stash survives the reset in step 4 without replaying.

**2. Rebase feature branch on latest origin/alpha and push:**
```bash
git fetch origin
git -C worktrees/<branch> rebase origin/alpha
# if conflicts: resolve, then git -C worktrees/<branch> add <files> && GIT_EDITOR=true git -C worktrees/<branch> rebase --continue
git -C worktrees/<branch> push --force-with-lease origin HEAD
```
> If anything non-obvious happened this PR — add one entry to GOTCHAS.md **in the feature worktree** and commit it there before pushing. It will land in the squash commit. Do not write GOTCHAS.md directly to alpha (it would be overwritten by the reset in step 4).

**3. Squash-merge:**
```bash
gh pr merge <number> --squash
```
If branch protection blocks: `gh pr merge <number> --squash --admin`

**4. Sync alpha — reset, not rebase:**
```bash
git fetch origin
git reset --hard origin/alpha
```
> Why `reset --hard` instead of `pull --rebase`: after a squash merge, any local commit on alpha (including one from a previous dirty-state save) may share content with the squash commit. `--rebase` replays the local commit on top of the squash and conflicts. `reset --hard` moves HEAD cleanly to origin/alpha with no replay.

Then restore the stash if one was created:
```bash
if [ "${STASHED:-0}" = 1 ]; then
  git stash pop
  RESTORED=$(git status --porcelain | grep "^ M\|^M ")
  if [ -n "$RESTORED" ]; then
    git add -u && git commit -m "chore: restore session edits carried through merge"
  fi
fi
```
If `stash pop` conflicts (rare — means the squash and the stash both touched the same file), take the squash version (`git checkout --theirs <file>`) since it was the reviewed and tested change.

**5. Cleanup:**
```bash
just pr-clean <pr-number>          # skip if no just pr-install was run (diff-review path)
wtp remove <branch> --force
git push origin --delete <branch>
```

**6. Bump, install, push:**
```bash
just bump && just install          # from the repo root
git push
```

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

### End-of-run notification + pane close

After surfacing `[COMPLETE]`, fire a non-blocking notification and close this pane. Two cases:

**Clean exit (no unresolved questions, no pending improvements awaiting user decision):**
```bash
(RESULT=$(plexi notify --title "Shipped #<number>" --body "<title> — v<version>" \
  --choice "ok:Dismiss" \
  --choice "open:Open PR")
 [ "$RESULT" = "open" ] && open "<pr-url>") &
plexi pane close
```
The outer `&` makes it fire-and-forget — do not block on a choice. The subshell runs `open` only if the user clicks "Open PR". `plexi pane close` exits the pane cleanly without waiting for the click.

**Soft exit (improvements proposed, awaiting user vote/approval, or any deferred thread):**
Do NOT auto-close. Send a synchronous notification with the standard 3-choice convention so the user can route back:
```bash
plexi notify --title "Shipped #<number> — review needed" \
  --body "<title> — v<version>. <N> improvement(s) proposed for review." \
  --choice "a:Talk to Claude:pane_focus:$SHIP_PANE" \
  --choice "b:Approve all" \
  --choice "c:Skip all"
```
Pane stays alive; user can land back here via choice `c` to discuss.

**Decision: clean vs soft** — if `/improve` output had any tier-2 (codebase change) proposals filed as `proposed-improvement` issues that need user vote, OR any deferred threads from the cycle, that's a soft exit. Otherwise clean.

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
- **Build verification in worktrees:** always use `cargo build --manifest-path <worktree>/Cargo.toml` — never rely on CWD being the right worktree
- **Cross-repo changes:** when the ship cycle modifies files outside this repo (dotfiles, global skills, etc.), commit those to their own repo as a separate step before marking [COMPLETE]
- **Spawn-queue ≠ process down:** `PLEXI_SOCKET` unset means "outside a Plexi pane" — not that the host is absent; never write messaging implying Plexi is not running when taking the queue fallback path
- Alpha must be clean when the cycle ends
- Subagents stage only — orchestrator owns the commit and the PR
- Never dispatch a subagent without first producing the Phase 3 implementation spec
- Every implementation must include a logging plan — no feature ships without info-level traces and warn-level bail-outs
- **CLI changes must update `~/.claude/skills/plexi-cli/SKILL.md`** in the same PR — bump `skill_version` to match the new Plexi version
- **SDK changes must update the build-plexi-app skill** (issue #608, not yet created) in the same PR once that skill exists — bump its `skill_version` to match
- **Decision-point notifications** — when the cycle blocks for user input (Phase 4 testing block, Phase 4b fail/modify branch, Phase 5 large-diff conversation), in addition to surfacing the block in chat, send a `plexi notify` with the 3-choice convention: `a:Talk to Claude:pane_focus:$SHIP_PANE`, `b:<primary action>`, `c:<secondary action>`. Talk to Claude is always first — it's the escape hatch the user needs most. The notification is synchronous — the bash command blocks until the user clicks. This lets the user be away from this pane and still be pulled in only when a real decision is needed.
- **Origin pane ID** — `$SHIP_PANE` is captured in Phase 1 and reused for every notification's `pane_focus` choice. If the user closes the pane mid-cycle, `pane_focus` will fail silently — detect via `plexi pane list | grep -q "\"id\": $SHIP_PANE"` before each notify, and if absent, swap the `c` choice to a plain `c:Open new Claude pane` key whose handler runs `plexi terminal claude` after the click.
- **Choice action types** — only `pane_focus` is a host-side action. Every other "do something" choice is just `key:Label` — capture the return value into `$RESULT` and run the shell command (`open <url>`, `open -a <app>`, etc.) after the click in a `case` statement. Do not invent action types like `open_url` or `open_app` — they don't exist.
- **End-of-run** — every cycle ends with `plexi notify` + `plexi pane close` (clean exit) or `plexi notify` with `pane_focus` choice (soft exit, pane stays alive). Never let a ship cycle end without one or the other — silent exits leave the user with no signal.
- **Pane title lifecycle** — set to `"#<issue> — <short-title>"` at Phase 1 start; update to `"#<issue> / PR #<pr> — <short-title>"` immediately after `gh pr create` in Phase 4. Both forms must be set so the pane is findable by either number.
