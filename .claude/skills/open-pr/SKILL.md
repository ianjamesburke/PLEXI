---
name: open-pr
description: "Phase 2 of the PLEXI ship pipeline. Takes a pushed feature branch, creates a PR targeting alpha, runs AI review (CodeRabbit + Gemini), and appends results to the issue body. Input: branch name or auto-detect from CWD. Output: PR URL."
risk: medium
source: local
date_added: "2026-05-20"
---

# Open PR

Phase 2 of the ship pipeline. Input: a pushed feature branch. Output: PR URL ready for `/validate-pr`.

**Entry points:**
- `/open-pr` — auto-detect branch from CWD (must be inside a feature worktree)
- `/open-pr <branch-name>` — explicit branch (e.g. `feature/1234-something`)
- `/open-pr <pr-number>` — re-run AI review on an existing PR, skip PR creation

On completion, **append to the issue body's Ship Log**, set pipeline labels (see Pipeline Labels below), and output:

```
[PR OPENED] PR #<n> — <title>
PR: <url>
AI Review: <N> fixes applied | no changes needed
Pipeline: pipeline:validate + ready set — invoking /validate-pr inline
```

> **Labels are the live state.** Never read the Ship Log to determine pipeline stage — read the issue labels. Ship Log is audit trail only.

---

## Step 1 — Detect Branch and Issue

**If branch-name given:**
```bash
git ls-remote origin "refs/heads/<branch>"
```
Fail loudly if branch doesn't exist on origin.

**If auto-detecting from CWD:**
```bash
git branch --show-current
```
Must be a `feature/` or `fix/` branch. Fail if on `alpha`, `beta`, or `main`.

**Extract issue number from branch:**
- `feature/<number>-...` → issue number is the first segment after `feature/`
- Bundle: `feature/bundle-<n1>-<n2>-...` → multiple issue numbers

**Fetch issue(s) for PR body:**
```bash
# Single issue:
gh issue view <number> --json title,body,labels --jq '{title: .title, body: .body}'
# Bundle — fetch all N in parallel, redirect to temp files to prevent stdout interleaving:
gh issue view <n1> --json title,body,labels --jq '{title: .title, body: .body}' > /tmp/issue_n1.json &
gh issue view <n2> --json title,body,labels --jq '{title: .title, body: .body}' > /tmp/issue_n2.json &
wait
cat /tmp/issue_n1.json /tmp/issue_n2.json
rm /tmp/issue_n1.json /tmp/issue_n2.json
```

---

## Step 2 — Create PR

**Idempotency guard — existing PR for this branch (run FIRST, before anything else):**
```bash
EXISTING_PR=$(gh pr list --head <branch> --state open --json number,url --jq '.[0] // empty')
if [ -n "$EXISTING_PR" ]; then
  PR_NUMBER=$(echo "$EXISTING_PR" | jq -r '.number')
  PR_URL=$(echo "$EXISTING_PR" | jq -r '.url')
  echo "[open-pr] PR #$PR_NUMBER already exists for <branch> — skipping creation, advancing pipeline labels."
fi
```
If `EXISTING_PR` is non-empty, **do NOT run `gh pr create`** (it would error and abort the skill, stranding the issue at `pipeline:open-pr` forever). Skip directly to the **Pipeline Labels** step to advance the issue to `pipeline:validate`, then invoke `/validate-pr $PR_NUMBER`. This makes open-pr idempotent: re-running on an already-PR'd branch advances state instead of looping.

**Lightweight PR detection:**
```bash
ISSUE_LABELS=$(gh issue view <number> --json labels --jq '[.labels[].name] | join(",")')
LIGHTWEIGHT=false
if echo "$ISSUE_LABELS" | grep -q "bundle"; then
  LIGHTWEIGHT=true
fi
```

**PR body — lightweight path:** When `LIGHTWEIGHT=true`, add the CodeRabbit ignore directive and a minimal body.

For a **single issue**:
```bash
PR_URL=$(gh pr create \
  --base alpha \
  --head <branch> \
  --title "<issue-title> (#<number>)" \
  --body "$(cat <<'EOF'
<!-- coderabbitai:ignore -->
Closes #<number>

## Summary

<2-3 bullet points summarizing what changed and why>
EOF
)")
```

For a **bundle** (`feature/bundle-<n1>-<n2>-...`):
```bash
PR_URL=$(gh pr create \
  --base alpha \
  --head <branch> \
  --title "<short summary> (#<n1>, #<n2>)" \
  --body "$(cat <<'EOF'
<!-- coderabbitai:ignore -->
Closes #<n1>, Closes #<n2>

## Summary

**#<n1> — <issue-n1-title>**
- <bullet summarizing n1 change>

**#<n2> — <issue-n2-title>**
- <bullet summarizing n2 change>
EOF
)")
```

**PR body — standard path:** Full body with Done When and Notes. For bundles, repeat Closes and Done When for each issue:
```bash
PR_URL=$(gh pr create \
  --base alpha \
  --head <branch> \
  --title "<issue-title> (#<number>)" \
  --body "$(cat <<'EOF'
Closes #<number>

## Summary

<2-3 bullet points summarizing what changed and why>

## Done When

<paste Done When checklist from issue body>

## Notes

<any non-obvious implementation decisions or gotchas>
EOF
)")
# Bundle standard path — one Closes per issue, one Done When block per issue:
# Closes #<n1>, Closes #<n2>
# ## Done When — #<n1>
# <checklist>
# ## Done When — #<n2>
# <checklist>
PR_NUMBER=$(echo "$PR_URL" | grep -oE '[0-9]+$')
```

Update pipeline label on **each** issue in the bundle (or the single issue):
```bash
# Repeat for each issue number N:
gh issue edit <N> --add-label "in progress" --remove-label "pipeline:implement" --add-label "pipeline:open-pr" 2>/dev/null || true
```

Update project board to "In Review" for **each** issue in the bundle:
```bash
# Repeat for each issue number N:
_PROJ_ITEM=$(gh api graphql -f query='query($n:Int!){repository(owner:"ianjamesburke",name:"PLEXI"){issue(number:$n){projectItems(first:5){nodes{id project{id}}}}}}' -F n=<N> --jq '.data.repository.issue.projectItems.nodes[]|select(.project.id=="PVT_kwHOAkOgys4BXaQY")|.id')
[ -n "$_PROJ_ITEM" ] && gh api graphql -f query='mutation($i:ID!,$v:String!){updateProjectV2ItemFieldValue(input:{projectId:"PVT_kwHOAkOgys4BXaQY",itemId:$i,fieldId:"PVTSSF_lAHOAkOgys4BXaQYzhSnRw8",value:{singleSelectOptionId:$v}}){projectV2Item{id}}}' -f i="$_PROJ_ITEM" -f v="f1399a59" > /dev/null
```

---

## Step 3 — AI Review

**Skip gate — lightweight PRs bypass AI review entirely:**
```bash
if [ "$LIGHTWEIGHT" = "true" ]; then
  echo "[AI REVIEW] Skipped — bundle/lightweight PR. CodeRabbit suppressed via <!-- coderabbitai:ignore -->."
  # Skip to Ship Log Append
fi
```

**Skip gate — small diffs:**
```bash
STAT_LINE=$(gh pr diff "$PR_NUMBER" --stat | tail -1)
CHANGED_LINES=$(printf '%s\n' "$STAT_LINE" | grep -oE '[0-9]+ insertions?' | grep -oE '[0-9]+')
DELETED_LINES=$(printf '%s\n' "$STAT_LINE" | grep -oE '[0-9]+ deletions?' | grep -oE '[0-9]+')
TOTAL_CHANGED=$(( ${CHANGED_LINES:-0} + ${DELETED_LINES:-0} ))
if [ "$TOTAL_CHANGED" -le 10 ]; then
  echo "[AI REVIEW] Skipped — diff is $TOTAL_CHANGED lines (threshold: 10)."
  # Skip to Ship Log Append
fi
```

**1. CodeRabbit local review** (run from inside the feature worktree):
```bash
cr 2>&1
```
Fix `error`/`warning` severity findings. `info`/`suggestion` are advisory — apply if trivial. If fixes made: commit, push, re-run to confirm clean.

**2. Inline bot review check (no waiting):**

Check immediately for any Gemini / CodeRabbit bot comments already posted:
```bash
gh pr view $PR_NUMBER --json comments \
  --jq '.comments[] | select(.author.login | test("gemini|coderabbit"; "i")) | "[\(.author.login)] \(.body)"'
gh api repos/ianjamesburke/PLEXI/pulls/$PR_NUMBER/reviews \
  --jq '.[] | select(.user.login | test("gemini|coderabbit"; "i")) | "[\(.user.login)] \(.body)"'
gh api repos/ianjamesburke/PLEXI/pulls/$PR_NUMBER/comments \
  --jq '.[] | select(.user.login | test("gemini|coderabbit"; "i")) | "[\(.user.login)] \(.body)"'
```

If no output: note "No bot feedback at PR creation time" in the review block and move on. Do not wait or poll.

For each finding:
- Correctness/bug/type safety → fix, commit, push
- Style/naming/docs → apply if trivial, skip if out of scope
- False positive → note and ignore

If fixes applied from bot feedback, re-run `cr 2>&1` to confirm clean.

**Required: emit `[AI REVIEW]` block:**
```
[AI REVIEW] PR #<n>

CodeRabbit (local):
  - [error/warning/info] <description> → FIXED: <what changed> | SKIPPED: <reason> | N/A: <reason>

Gemini / bot:
  - [<severity>] <description> → FIXED: <what changed> | SKIPPED: <reason> | N/A: <reason>

Net: <N> fix(es) committed | no changes needed
```

---

## Ship Log Append

Append to the `## Ship Log` section in **each** issue's body (all N issues in bundle mode, the single issue otherwise):

```markdown
**PR:** #<pr-number> — <pr-url>
**AI Review:** <N> fixes | no changes (CodeRabbit: <finding count>, Gemini: <finding count>)
```

```bash
# Repeat for each issue number N:
CURRENT_BODY=$(gh issue view <N> --json body --jq '.body')
# Append the PR line to the most recent Ship Log entry
gh issue edit <N> --body "<updated body>"
```

---

## Pipeline Labels

After writing the Ship Log, set pipeline state on **every** issue in the bundle (or the single issue):

```bash
# Repeat for each issue number N:
gh issue edit <N> \
  --add-label "pipeline:validate" \
  --add-label "ready" \
  --remove-label "pipeline:open-pr" \
  --remove-label "in progress"
```

After setting labels on all issues, invoke `/validate-pr <pr-number>` inline in the same pane — do not spawn a new pane or wait for PM to dispatch.

---


## Rules

- Never push to alpha, beta, or main directly
- PR must always target `alpha`
- Bundle mode: close all issues in Phase 6 of merge-pr
- The final approver is always the user — AI reviews inform the fix pass, never gate the merge
- If `cr` unavailable: note "cr not available" in the review block
- No bot feedback at creation time: note "No bot feedback at PR creation time" and move on — never wait or poll
