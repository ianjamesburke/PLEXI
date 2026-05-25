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

Mark issue as actively in this phase:
```bash
gh issue edit $ISSUE_NUMBER \
  --add-label "in progress" \
  --remove-label "pipeline:open-pr" \
  --add-label "pipeline:validate" 2>/dev/null || true
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

If install not needed → use diff-review testing block (see below). Skip install steps.

---

## Step 2 — Install

Run from inside the feature worktree:
```bash
REPO_ROOT=$(git rev-parse --show-toplevel)
cd "$REPO_ROOT/worktrees/$BRANCH"
just pr-install $PR_NUMBER
```

> **CWD check:** `just pr-install` runs `cargo bundle` from CWD. Running from alpha silently builds old code.

> **Compile check:** If no `Compiling plexi` line in output, the binary was cached. `touch src/<changed-file>.rs` then re-run.

Wait for completion. Read the log to confirm the build landed:
```bash
tail -20 ~/.plexi-pr-$PR_NUMBER/plexi.log
```

---

## Step 3 — Write and Surface Testing Block

**Spec gate:** Re-read the issue's Done When checklist before writing. Pass criteria map 1:1 to checklist items. No extra criteria.

**Check for existing POC app:** scan `apps/` and `apps/dev/` in the feature worktree. If one exists, use it.

**If more than one command, or copy-paste across panes is awkward:** write a `test_pr<N>.py` at the repo root instead of a markdown block.

```python
import subprocess, json, sys
CLI = "/usr/local/bin/plexi-pr-<N>"  # full path required
PASS = "\033[32mPASS\033[0m"; FAIL = "\033[31mFAIL\033[0m"
failures = []
def check(label, ok, detail=""):
    if ok: print(f"  {PASS}  {label}")
    else: print(f"  {FAIL}  {label}{': ' + detail if detail else ''}"); failures.append(label)
# ... checks ...
subprocess.run([CLI, "notify", "--title", "Test result", "--body", "PASS" if not failures else "FAIL"])
if failures: sys.exit(1)
```

**Surface testing block format:**
```
[TESTING] PR #<n> — <title> (attempt <attempt+1>/3)
PR: <pr-url>

Instructions:
1. <exact step>
2. <exact step>

Pass criteria:
- <concrete observable outcome>

Fail criteria:
- <concrete observable symptom>

Reply: "pass" | "fail: <description>" | "modify: <bounded change>"
```

**Notification:**
```bash
# Route reply to PM pane if PM dispatched this skill, otherwise this pane
REPLY_PANE="${PM_PANE_ID:-$PLEXI_PANE_ID}"
RESULT=$(plexi notify \
  --title "PR #<n> ready to test (attempt $((ATTEMPT_COUNT+1))/3)" \
  --body "<title>. Reply pass/fail/modify in pane." \
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

Append to issue Ship Log:
```markdown
**Validate attempt <N>:** PASS
**Test date:** <YYYY-MM-DD>
```

Set pipeline state:
```bash
gh issue edit $ISSUE_NUMBER \
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
git -C "$(git rev-parse --show-toplevel)/worktrees/$BRANCH" add <files>
git -C "$(git rev-parse --show-toplevel)/worktrees/$BRANCH" commit -m "fix: <description>"
git -C "$(git rev-parse --show-toplevel)/worktrees/$BRANCH" push
```

Re-run `just pr-install $PR_NUMBER` from the feature worktree. Re-surface the testing block. Do not expand scope.

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
git -C "$(git rev-parse --show-toplevel)/worktrees/$BRANCH" add <files>
git -C "$(git rev-parse --show-toplevel)/worktrees/$BRANCH" commit -m "fix: <description from failure>"
git -C "$(git rev-parse --show-toplevel)/worktrees/$BRANCH" push
```

Re-run `just pr-install $PR_NUMBER` from the feature worktree.

Append to Ship Log:
```markdown
**Validate attempt <N>:** FAIL — <verbatim user description>
**Fix applied:** <what was pushed> (commit <hash>)
```

Re-surface the testing block (attempt N+1/3). Fire the notification again. STOP.

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

1. **Read the PR log:**
```bash
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
just pr-clean $PR_NUMBER
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

```
[TESTING] PR #<n> — <title> (diff review only)

No binary install needed for this change.

Review the diff: <pr-url>

Pass criteria:
- <observable change visible in diff>

Fail criteria:
- <what would look wrong>

Reply: "pass" | "fail: <description>" | "modify: <change>"
```

---

## Rules

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
