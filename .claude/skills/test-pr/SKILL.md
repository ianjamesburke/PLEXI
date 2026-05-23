---
name: test-pr
description: "Standalone PR testing skill. Installs a PR build, reads the issue spec, writes test instructions, and returns pass/fail. Works independently — does not require being in a ship pipeline session. Input: PR number or issue number."
risk: low
source: local
date_added: "2026-05-20"
---

# Test PR

Standalone testing skill. Takes any PR and produces a structured test result.

**Entry:**
- `/test-pr <pr-number>` — test by PR number
- `/test-pr issue/<n>` — find the open PR for issue N and test it

**Output:**
```
[TEST RESULT] PR #<n>
Outcome: PASS | FAIL | INCONCLUSIVE
Attempt: <N>/3 (from Ship Log)
Next: /merge-pr <n> | /validate-pr <n> (for retry loop) | user decision needed
```

This skill is called by `/validate-pr` internally. It can also be called directly when:
- You have a PR open and want to test it outside the pipeline
- You want to re-test after a manual fix
- You're resuming a ship session cold and need to verify current state

---

## Step 1 — Resolve PR

**If given PR number directly:**
```bash
gh pr view <pr-number> --json title,headRefName,number,state,body
```

**If given issue number:**
```bash
gh pr list --search "head:feature/<issue-number>" --json number,title,state,url | head -1
```
Fail loudly if no open PR found.

Extract:
- `BRANCH` = `headRefName`
- `ISSUE_NUMBER` = parse from branch or PR body `Closes #<n>`
- `PR_NUMBER`

---

## Step 2 — Read Issue Spec

```bash
gh issue view $ISSUE_NUMBER --json title,body --jq '{title: .title, body: .body}'
```

Extract Done When checklist. Read current Ship Log to determine attempt count.

```bash
ATTEMPT_COUNT=$(gh issue view $ISSUE_NUMBER --json body --jq '.body' | grep -c "### Attempt")
```

---

## Step 3 — Install Gate

```bash
gh pr diff $PR_NUMBER --name-only
```

Skip install if all changed files are non-Rust, test-only, or diff-verifiable. Otherwise install.

**Install (run from feature worktree):**
```bash
REPO_ROOT=$(git rev-parse --show-toplevel)
cd "$REPO_ROOT/worktrees/$BRANCH"
just pr-install $PR_NUMBER
```

Verify compile happened (look for `Compiling plexi` in output). If not: `touch src/<changed-file>.rs && just pr-install $PR_NUMBER`.

Check the log:
```bash
tail -20 ~/.plexi-pr-$PR_NUMBER/plexi.log
```

Report any errors or warnings before proceeding.

---

## Step 4 — Write Testing Block

**Spec gate:** Pass criteria map 1:1 to Done When checklist. No extra criteria.

Check for existing POC app in `apps/core/`, `apps/examples/`, or `dev-examples/`. Use it if present.

**For multi-step or multi-pane tests:** write `test_pr<N>.py` at repo root.

```python
import subprocess, sys
CLI = "/usr/local/bin/plexi-pr-<N>"
PASS = "\033[32mPASS\033[0m"; FAIL = "\033[31mFAIL\033[0m"
failures = []
def check(label, ok, detail=""):
    if ok: print(f"  {PASS}  {label}")
    else: print(f"  {FAIL}  {label}{': ' + detail if detail else ''}"); failures.append(label)
# ... checks ...
subprocess.run([CLI, "notify", "--title", "Test done", "--body", "PASS" if not failures else f"FAIL: {failures}"])
if failures: sys.exit(1)
```

**CLI-only checks (no running host needed):** run them yourself and report the result. Don't surface a testing block for checks you can run.

**Surface testing block:**
```
[TESTING] PR #<n> — <title> (attempt <attempt+1>/3)
PR: <pr-url>
Binary: plexi-pr-<n>

Instructions:
1. <exact step>
2. <exact step>

Pass criteria (from Done When):
- <concrete observable outcome>

Fail criteria:
- <concrete observable symptom>

Reply: "pass" | "fail: <description>" | "modify: <bounded change within scope>"
```

**Notification:**
```bash
RESULT=$(plexi notify \
  --title "PR #<n> ready to test" \
  --body "<title>. Attempt $((ATTEMPT_COUNT+1))/3." \
  --choice "a:Talk to Claude:pane_focus:$PLEXI_PANE_ID" \
  --choice "b:Open PR build" \
  --choice "c:Open PR")
case "$RESULT" in
  b) open -a "Plexi PR$PR_NUMBER" ;;
  c) open "<pr-url>" ;;
esac
```

**STOP. Wait for reply.**

---

## Step 5 — Return Result

This skill does not handle retry loops or hard reject. It returns the result and lets the caller decide.

**Pass:**
Append to issue Ship Log:
```markdown
**Test attempt <N>:** PASS — <YYYY-MM-DD>
```
Output: `[TEST RESULT] PR #<n> — PASS`

**Fail:**
Append to issue Ship Log:
```markdown
**Test attempt <N>:** FAIL — <verbatim user description> (<YYYY-MM-DD>)
```
Output: `[TEST RESULT] PR #<n> — FAIL: <description>`

**Modify:**
Append to issue Ship Log:
```markdown
**Test attempt <N>:** MODIFY — <description>
```
Output: `[TEST RESULT] PR #<n> — MODIFY: <description>`

The caller (`/validate-pr`) handles what happens next based on the attempt count.

---

## Rules

- Never merge, close, or push in this skill — read and report only
- Always read the PR log before reporting a fail finding
- Never include log-reading commands in the user-facing testing block
- `test_pr<N>.py` must use full absolute path to the PR binary
- One command per code block in the testing instructions — never inline
- Never run `test_pr<N>.py` yourself — it blocks on `plexi notify`
- CLI-verifiable output: run it yourself, report the result, skip the testing block
