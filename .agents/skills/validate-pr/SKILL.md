---
name: validate-pr
description: "Phase 3 of the PLEXI ship pipeline. Installs a PR build, generates test instructions, handles user pass/fail/modify responses, and manages the retry loop (max 3 soft rejects before escalating to hard reject). Input: PR number. Output: approved PR number or hard-reject."
risk: medium
source: local
date_added: "2026-05-20"
---

# Validate PR

Phase 3 of the ship pipeline. Manages the test loop and all rejection paths.

**Entry:**
- `/validate-pr <pr-number>` — start validation for a PR
- `/validate-pr <pr-number> --attempt <n>` — resume at attempt N (used internally on soft-reject retry)

**Outcomes:**
- **Pass** → sets `pipeline:merge + ready` → invokes `/merge-pr` inline
- **Soft reject** → push a fix commit, reinstall, re-run (max 3 attempts total)
- **Hard reject** → close PR, rewrite issue body, remove all pipeline labels, clean up

**Reads `## Ship Log` in the issue body to determine current attempt count.** Never trust an argument alone — always verify against the log.

**Testing notification routing:** If the env var `PM_PANE_ID` is set (injected by PM when it dispatches this skill), route the `--choice "Talk to Claude"` action to that pane so the user replies in the PM pane. Otherwise default to `$PLEXI_PANE_ID`.

> **Labels are the live state.** Never read the Ship Log to determine pipeline stage — read the issue labels. Ship Log is audit trail only.

> **Pane status title.** Runs in the same dispatched pane (named `#<n>`, or `#<n1>+<n2>` for a bundle). Update the title at each stage so the PM reads state from `plexi pane list` instead of capturing content:
> ```bash
> plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL} pane name "#<n> · <state>"
> ```
> **The status word must never contain a digit** (the PM maps panes to issues via `grep -oE '[0-9]+'` — a PR number in the suffix would corrupt the census). States this skill sets: `validate`, `needs-you` (waiting on the user — the PM surfaces this), `fixing`, `blocked`.

---

## Step 0 — Read Context

```bash
gh pr view <pr-number> --json title,headRefName,number,baseRefName,state
```

Extract:
- `BRANCH` = `headRefName` (e.g. `feature/1234-something`)
- `ISSUE_NUMBER` = parse from branch name
- `PR_NUMBER` = from arg

Read current attempt count from issue body:
```bash
gh issue view $ISSUE_NUMBER --json body --jq '.body'
```
Count `**Validate attempt` entries in `## Ship Log`. Set `ATTEMPT_COUNT` = that number (0 if no log yet).
```bash
ATTEMPT_COUNT=$(gh issue view $ISSUE_NUMBER --json body --jq '.body' \
  | grep -cE '^\*\*Validate attempt [0-9]+:' || true)
```

Mark issue as actively in this phase and update pane status:
```bash
gh issue edit $ISSUE_NUMBER \
  --add-label "in progress" \
  --remove-label "pipeline:open-pr" \
  --add-label "pipeline:validate" 2>/dev/null || true
plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL} pane name "#<n> · validate"
```

---

## Step 0b — Resume Guard After User Reply

Run this before handling any `pass`, `fail`, or `modify` reply, especially after context compaction. A reply to a `[TESTING]` block is part of PR validation by default; do not treat it as a fresh alpha bug report unless the user explicitly says to leave the PR flow.

Rehydrate the PR and force all subsequent reads, edits, tests, commits, pushes, and PR installs into the feature worktree:
```bash
if [ -z "${PR_NUMBER:-}" ]; then
  VALIDATION_STATE="/tmp/plexi-validate-${PLEXI_PANE_ID:-unknown}.env"
  test -f "$VALIDATION_STATE" || { echo "Missing PR_NUMBER and validation state: $VALIDATION_STATE"; exit 1; }
  . "$VALIDATION_STATE"
fi
PR_JSON=$(gh pr view $PR_NUMBER --json headRefName,baseRefName,state,isDraft)
BRANCH=$(printf '%s' "$PR_JSON" | jq -r '.headRefName')
PR_STATE=$(printf '%s' "$PR_JSON" | jq -r '.state')
test "$PR_STATE" = "OPEN" || { echo "PR #$PR_NUMBER is not open: $PR_STATE"; exit 1; }
ALPHA_ROOT=$(git worktree list --porcelain | awk '
  /^worktree / { path=$2 }
  /^branch refs\/heads\/alpha$/ { print path; exit }
')
test -n "$ALPHA_ROOT" || { echo "Could not resolve alpha worktree"; exit 1; }
WORKTREE="$ALPHA_ROOT/worktrees/$BRANCH"
test -d "$WORKTREE" || { echo "Missing PR worktree: $WORKTREE"; exit 1; }
CURRENT_BRANCH=$(git -C "$WORKTREE" branch --show-current)
test "$CURRENT_BRANCH" = "$BRANCH" || { echo "Wrong worktree branch: $CURRENT_BRANCH != $BRANCH"; exit 1; }
case "$CURRENT_BRANCH" in alpha|beta|main) echo "Refusing to validate from protected branch: $CURRENT_BRANCH"; exit 1 ;; esac
cd "$WORKTREE"
git status --short --branch
```

Hard stops:
- Current checkout is `alpha`, `beta`, or `main`.
- `WORKTREE` cannot be resolved from the PR head branch.
- The PR is closed or merged.
- You are about to edit files before the guard above has passed.

While handling validation feedback, `just install` is banned. Reinstall only with `just pr-install $PR_NUMBER` from `WORKTREE`.

---

## Step 1 — Install Gate

Check if a binary install is needed:
```bash
gh pr diff $PR_NUMBER --name-only
```

**Skip install if all changed files are:**
- Non-Rust: justfile, TOML config, scripts, docs, skills, completion files, markdown
- Rust with pure code deletion and no behavioral change
- Rust test-only additions (`#[cfg(test)]` or `tests/` only)
- CLI output changes verifiable without a running host

**Never skip install if any changed file is under `apps/` or `apps/dev/`** — Python apps are copied to the profile dir on install, not served from source. Skipping install means the old version runs.

If install not needed → use diff-review testing block (see below). Skip install steps.

---

## Step 2 — Install

Run from inside the feature worktree:
```bash
REPO_ROOT=$(git rev-parse --show-toplevel)
cd "$REPO_ROOT/worktrees/$BRANCH"
just pr-install $PR_NUMBER
```

> **CWD check:** `just pr-install` runs `cargo bundle` AND `rsync apps/dev/` from CWD. Running from the repo root (alpha) syncs alpha's `apps/dev/` — any app added only on the feature branch will be missing or stale in the profile dir. Always `cd` into the worktree first, every time, including fix reinstalls.

> **Compile check:** If no `Compiling plexi` line in output, the binary was cached. `touch src/<changed-file>.rs` then re-run.

Wait for completion. Read the log to confirm the build landed:
```bash
# Only read a runtime log if the PR app has already launched and created one.
# A fresh install does not create plexi.log. Do not block testing on this.
LOG_PATH="$HOME/.plexi-pr-$PR_NUMBER/plexi.log"
[ -f "$LOG_PATH" ] && tail -20 "$LOG_PATH" || true
```

---

## Step 2b — Automated Quality Checks

**Skip gate — assess before running.** Read the diff and changed file list, then classify the PR:

- **Run checks** if any changed file contains: new logic, new branches, new error paths, behavioral changes, new API surface, bug fixes, or anything systemic that could break silently.
- **Skip checks** if ALL changes are exclusively: color/spacing/font-size constants, help/doc strings, markdown files, config TOML values, label/copy text, or UI layout values that require human eyes to verify anyway. When skipping, set `AI_FINDINGS="skipped — cosmetic/style change, user verifies visually"`.

When in doubt, run. Skipping is only correct when you are confident a human looking at the UI is the only meaningful verification.

**If running — rigorous Codex review (gpt-5.5, xhigh reasoning):**

```bash
# codex review: [PROMPT] and --base are mutually exclusive — use --base alone.
# Config (~/.codex/config.toml) already sets model=gpt-5.5 and model_reasoning_effort=xhigh.
# Use git worktree list to find the alpha root — git rev-parse --show-toplevel returns the
# CWD's worktree path (not the main repo root) when run from inside a worktree.
ALPHA_ROOT=$(git worktree list --porcelain | grep -B2 "branch refs/heads/alpha" | grep "^worktree " | head -1 | cut -d' ' -f2)
REVIEW_OUT="/tmp/plexi-pr-$PR_NUMBER-codex-review.txt"
(cd "$ALPHA_ROOT/worktrees/$BRANCH" && codex review --base alpha > "$REVIEW_OUT" 2>&1) || true

# codex review can print tool transcripts before the actual findings. Keep that
# bulk out of the main agent context and testing block.
AI_FINDINGS=$(awk '
  /Findings|No findings|No issues|Issues found|Review findings/ { capture=1 }
  capture { print }
' "$REVIEW_OUT" | tail -120)

if [ -z "$AI_FINDINGS" ]; then
  AI_FINDINGS="Codex review completed; no concise findings section was detected. Full raw output: $REVIEW_OUT"
fi
```

Only surface `AI_FINDINGS` in the testing block. Do not paste the raw `codex review` transcript into chat; it can be thousands of lines of tool output. If you need to inspect raw output, read the temp file narrowly with `tail`, `rg`, or `sed`.

After the PR is installed, mergeable, and the review summary is available, surface the testing block immediately. Do not launch the PR app, browse logs, run broad full-suite tests, or keep investigating unless the install, build, targeted tests, or Codex findings show a real blocker.

---

## Step 3 — Write and Surface Testing Block

**Spec gate:** Re-read the issue's Done When checklist before writing. Pass criteria map 1:1 to checklist items. No extra criteria.

**Fetch issue brief:**
```bash
ISSUE_TITLE=$(gh issue view $ISSUE_NUMBER --json title --jq '.title')
ISSUE_WHAT=$(gh issue view $ISSUE_NUMBER --json body --jq '.body' \
  | sed '/^---$/,/^---$/d' \
  | grep -v '^#' \
  | grep -v '^$' \
  | head -3)
```
This is the one-line context shown at the top of every testing block so the reviewer knows exactly what they're evaluating.

**Surface testing block format:**
```
[TESTING] PR #<n> — <title> (attempt <attempt+1>/3)
PR: <pr-url>

Issue #<issue-number>: <ISSUE_TITLE>
What this ships: <ISSUE_WHAT — first non-header paragraph from issue body>

Codex review (gpt-5.5 xhigh):
<AI_FINDINGS verbatim>

Pass criteria (from Done When):
- <concrete observable outcome>

Fail criteria:
- <concrete observable symptom>

Reply: "pass" | "fail: <description>" | "modify: <bounded change>"
```

**Your final response for this turn must be the testing block above.** Do not replace it with a status recap. Do not keep working after it is surfaced.

After printing the testing block in the final response, send a best-effort notification. The notification is only an attention cue; the testing block is the source of truth.

Use a non-blocking choice notification so the user can focus the reply pane without leaving the agent process blocked on a response file:
```bash
# Persist enough state for a later user reply to survive context compaction.
ALPHA_ROOT=$(git worktree list --porcelain | awk '
  /^worktree / { path=$2 }
  /^branch refs\/heads\/alpha$/ { print path; exit }
')
VALIDATION_STATE="/tmp/plexi-validate-${PLEXI_PANE_ID:-unknown}.env"
{
  printf 'PR_NUMBER=%s\n' "$PR_NUMBER"
  printf 'ISSUE_NUMBER=%s\n' "$ISSUE_NUMBER"
  printf 'BRANCH=%s\n' "$BRANCH"
  printf 'WORKTREE=%s\n' "$ALPHA_ROOT/worktrees/$BRANCH"
} > "$VALIDATION_STATE"

# Flip pane status to needs-you so the PM surfaces this lane as awaiting the user
plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL} pane name "#<n> · needs-you"
# Route reply to PM pane if PM dispatched this skill, otherwise this pane
REPLY_PANE="${PM_PANE_ID:-$PLEXI_PANE_ID}"
plexi notify --no-wait \
  --title "PR #<n> quality checks done (attempt $((ATTEMPT_COUNT+1))/3)" \
  --body "<title>. Review the [TESTING] block, then reply pass/fail/modify." \
  --choice "talk:Talk to Claude:pane_focus:$REPLY_PANE"
```

**STOP. Wait for user reply.**

---

## Step 4 — Handle Response

### Pass

Append to issue Ship Log and set pipeline state in one call:
```bash
CURRENT_BODY=$(gh issue view $ISSUE_NUMBER --json body --jq '.body')
gh issue edit $ISSUE_NUMBER \
  --body "$(printf '%s\n**Validate attempt %s:** PASS\n**Test date:** %s' "$CURRENT_BODY" "$((ATTEMPT_COUNT+1))" "$(date +%Y-%m-%d)")" \
  --add-label "pipeline:merge" \
  --add-label "ready" \
  --remove-label "pipeline:validate" \
  --remove-label "in progress"
```

Output:
```
[VALIDATED] PR #<n> — <title>
Attempt: <N>/3
Pipeline: pipeline:merge + ready set — invoking /merge-pr inline
```

Invoke `/merge-pr <pr-number>` inline in the same pane.

### Modify

Valid only if pass criteria were **not** fully met. If criteria were met and user wants extras: "Pass criteria are met. Filing that as a separate issue." Create the issue, then treat as pass.

First run **Step 0b — Resume Guard After User Reply**. Do not inspect or edit production files until `cd "$WORKTREE"` has succeeded.

Fix the specific change on the feature branch, commit, push:
```bash
plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL} pane name "#<n> · fixing"
git add <files>
git commit -m "fix: <description>"
git push
```

Run `cargo build` from `WORKTREE`, then re-run `just pr-install $PR_NUMBER` **from inside `WORKTREE`**. Never run `just install` during validation. Re-surface the testing block (which flips status back to `needs-you`). Do not expand scope.

Append to Ship Log:
```markdown
**Validate attempt <N>:** MODIFY — <description of change>
```

### Soft Reject (fail with description)

"fail" without a description: ask for it before taking any action.

**Check attempt count:**
```bash
ATTEMPT_COUNT=$(gh issue view $ISSUE_NUMBER --json body --jq '.body' | grep -c "### Attempt")
```

**If ATTEMPT_COUNT < 3 (soft reject — push a fix):**

Read the PR build log first:
```bash
tail -100 ~/.plexi-pr-$PR_NUMBER/plexi.log
```
Report what the log shows.

First run **Step 0b — Resume Guard After User Reply**. Do not inspect or edit production files until `cd "$WORKTREE"` has succeeded.

Apply the targeted fix to the feature branch, commit, push:
```bash
plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL} pane name "#<n> · fixing"
git add <files>
git commit -m "fix: <description from failure>"
git push
```

Run `cargo build` from `WORKTREE`, then re-run `just pr-install $PR_NUMBER` **from inside `WORKTREE`**. Never run `just install` during validation.

Append to Ship Log:
```markdown
**Validate attempt <N>:** FAIL — <verbatim user description>
**Fix applied:** <what was pushed> (commit <hash>)
```

Re-surface the testing block (attempt N+1/3) in your response first, then fire the notification again. STOP.

**If ATTEMPT_COUNT >= 3 (escalate to hard reject):**

Send notification:
```bash
RESULT=$(plexi notify \
  --title "PR #<n> failed 3 times — hard reject?" \
  --body "<title>. 3 attempts exhausted. Close and rewrite issue?" \
  --choice "a:Talk to Claude:pane_focus:$PLEXI_PANE_ID" \
  --choice "b:Hard reject — close PR" \
  --choice "c:Keep open — I'll review")
```

If `b` or user confirms in chat → proceed to Hard Reject.
If `c` → STOP. Leave PR open. Remove attempt limit — user owns it from here.

---

## Hard Reject

1. **Mark the lane blocked and read the PR log:**
```bash
plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL} pane name "#<n> · blocked"
tail -100 ~/.plexi-pr-$PR_NUMBER/plexi.log
```

2. **Close the PR:**
```bash
gh pr close $PR_NUMBER --comment "Closing after 3 failed validate attempts. Full context in issue #$ISSUE_NUMBER body."
```

3. **Rewrite the issue body:**

Fetch the current body and all Ship Log entries. Rewrite the body to include:
- Original spec (preserved)
- `## Prior Attempts` section (required format):
  ```markdown
  ## Prior Attempts

  **Attempt N:** What was tried (branch, files changed, approach).
  **Why it failed:** Root cause or observable symptom (from validate log and user description).
  **What to try next:** Specific next investigation step.
  ```
  One block per attempt, using the Ship Log entries as source.
- Updated `## Ship Log` with hard-reject entry:
  ```markdown
  **Hard reject:** <YYYY-MM-DD> — PR #<pr-number> closed after <N> attempts.
  ```

```bash
gh issue edit $ISSUE_NUMBER --body "<rewritten body>"
```

4. **Re-label (remove all pipeline labels, reset to ready):**
```bash
gh issue edit $ISSUE_NUMBER \
  --remove-label "in progress" \
  --remove-label "pipeline:implement" \
  --remove-label "pipeline:open-pr" \
  --remove-label "pipeline:validate" \
  --remove-label "pipeline:merge" \
  --add-label "ready"
```

5. **Clean up:**
```bash
just channel-clean pr-$PR_NUMBER
wtp remove feature/<branch> --force
git push origin --delete feature/<branch>
```

6. **Output:**
```
[HARD REJECT] PR #<n> closed
Issue #<issue-number> rewritten with Prior Attempts section
Branch feature/<branch> deleted
Issue re-labeled ready for next attempt
```

---

## Diff-Review Testing Block (no install)

Still run Step 2b (automated quality checks) even when install is skipped. Then surface:

```
[TESTING] PR #<n> — <title> (diff review only)
PR: <pr-url>

Issue #<issue-number>: <ISSUE_TITLE>
What this ships: <ISSUE_WHAT — first non-header paragraph from issue body>

No binary install needed for this change.

Codex review (gpt-5.5 xhigh):
<AI_FINDINGS verbatim>

Pass criteria:
- <observable change visible in diff>

Fail criteria:
- <what would look wrong>

Reply: "pass" | "fail: <description>" | "modify: <change>"
```

Before firing the notification, flip pane status the same as the install path:
```bash
plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL} pane name "#<n> · needs-you"
```

---

## Rules

- Step 2b quality checks (Codex diff review) run unless the PR is exclusively cosmetic/style — assess the diff, then decide
- Cosmetic = colors, spacing, font sizes, help strings, markdown, config values, UI copy — anything where human visual verification is the only meaningful check
- AI_FINDINGS is always shown verbatim — never summarized or filtered
- Issue brief (ISSUE_TITLE + ISSUE_WHAT) must appear at the top of every testing block
- `fail` without description: ask for it before taking any action
- `modify` is only valid if pass criteria not yet met
- A user reply after a `[TESTING]` block stays inside `/validate-pr` by default, even after context compaction
- Before any validation fix, rehydrate the PR with Step 0b and move into `WORKTREE`; no edits from repo root
- `just install` is forbidden during validation; only `just pr-install $PR_NUMBER` from `WORKTREE`
- Never commit or release-bump `alpha`, `beta`, or `main` while handling `pass`, `fail`, or `modify`
- Attempt count comes from the Ship Log, not arguments
- Max 3 soft rejects. Hard reject requires explicit confirmation unless user already typed "fail" 3 times.
- Never close an issue on hard reject — only the PR closes. Issue stays open.
- PR build log must be read before any soft-reject diagnosis
- Test script (`test_pr<N>.py`) is deleted in merge-pr Phase 5 — never committed
- Never run `test_pr<N>.py` yourself — it blocks on `plexi notify`; use direct Bash for automated checks
- All subprocess calls in test script must use full absolute path to PR binary
- Never include `tail` or log-reading in user-facing testing block — read the log yourself, report findings directly
- Cosmetic issues spotted during testing go in separate issues — not blockers or modifies
