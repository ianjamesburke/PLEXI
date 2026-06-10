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
tail -20 ~/.plexi-pr-$PR_NUMBER/plexi.log
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
AI_FINDINGS=$(cd "$ALPHA_ROOT/worktrees/$BRANCH" && \
  codex review --base alpha 2>&1)
[ -z "$AI_FINDINGS" ] && AI_FINDINGS="Codex review unavailable — skipping automated review."
```

`AI_FINDINGS` is surfaced verbatim in the testing block — do not summarize or filter it.

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

**Print the testing block above in your response first. Only after the testing block text is in your response, run the notification:**
```bash
# Flip pane status to needs-you so the PM surfaces this lane as awaiting the user
plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL} pane name "#<n> · needs-you"
# Route reply to PM pane if PM dispatched this skill, otherwise this pane
REPLY_PANE="${PM_PANE_ID:-$PLEXI_PANE_ID}"
RESULT=$(plexi notify \
  --title "PR #<n> quality checks done (attempt $((ATTEMPT_COUNT+1))/3)" \
  --body "<title>. Review Codex findings above, then reply pass/fail/modify." \
  --choice "a:Talk to Claude:pane_focus:$REPLY_PANE" \
  --choice "b:Open PR build" \
  --choice "c:Open PR")
case "$RESULT" in
  b) open -a "Plexi PR$PR_NUMBER" ;;
  c) open "<pr-url>" ;;
esac
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

Fix the specific change on the feature branch, commit, push:
```bash
plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL} pane name "#<n> · fixing"
git -C "$(git rev-parse --show-toplevel)/worktrees/$BRANCH" add <files>
git -C "$(git rev-parse --show-toplevel)/worktrees/$BRANCH" commit -m "fix: <description>"
git -C "$(git rev-parse --show-toplevel)/worktrees/$BRANCH" push
```

Re-run `just pr-install $PR_NUMBER` **from inside the feature worktree** (`cd worktrees/$BRANCH && just pr-install $PR_NUMBER`) — not from the repo root. Re-surface the testing block (which flips status back to `needs-you`). Do not expand scope.

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

Apply the targeted fix to the feature branch, commit, push:
```bash
plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL} pane name "#<n> · fixing"
git -C "$(git rev-parse --show-toplevel)/worktrees/$BRANCH" add <files>
git -C "$(git rev-parse --show-toplevel)/worktrees/$BRANCH" commit -m "fix: <description from failure>"
git -C "$(git rev-parse --show-toplevel)/worktrees/$BRANCH" push
```

Re-run `just pr-install $PR_NUMBER` **from inside the feature worktree** (`cd worktrees/$BRANCH && just pr-install $PR_NUMBER`) — not from the repo root.

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

Before firing the notification and waiting, flip pane status the same as the install path:
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
- Attempt count comes from the Ship Log, not arguments
- Max 3 soft rejects. Hard reject requires explicit confirmation unless user already typed "fail" 3 times.
- Never close an issue on hard reject — only the PR closes. Issue stays open.
- PR build log must be read before any soft-reject diagnosis
- Test script (`test_pr<N>.py`) is deleted in merge-pr Phase 5 — never committed
- Never run `test_pr<N>.py` yourself — it blocks on `plexi notify`; use direct Bash for automated checks
- All subprocess calls in test script must use full absolute path to PR binary
- Never include `tail` or log-reading in user-facing testing block — read the log yourself, report findings directly
- Cosmetic issues spotted during testing go in separate issues — not blockers or modifies
