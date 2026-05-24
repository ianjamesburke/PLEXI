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
Pipeline: pipeline:validate + ready set — PM will dispatch /validate-pr on next run
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

**Fetch issue for PR body:**
```bash
gh issue view <number> --json title,body,labels --jq '{title: .title, body: .body}'
```

---

## Step 2 — Create PR

**Lightweight PR detection:**
```bash
ISSUE_LABELS=$(gh issue view <number> --json labels --jq '[.labels[].name] | join(",")')
LIGHTWEIGHT=false
if echo "$ISSUE_LABELS" | grep -q "bundle"; then
  LIGHTWEIGHT=true
fi
```

**PR body — lightweight path:** When `LIGHTWEIGHT=true`, add the CodeRabbit ignore directive and a minimal body:
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

**PR body — standard path:** Full body with Done When and Notes:
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
PR_NUMBER=$(echo "$PR_URL" | grep -oE '[0-9]+$')
```

Update pipeline label:
```bash
gh issue edit <number> --add-label "in progress" --remove-label "pipeline:implement" --add-label "pipeline:open-pr" 2>/dev/null || true
```

Update project board to "In Review":
```bash
_PROJ_ITEM=$(gh api graphql -f query='query($n:Int!){repository(owner:"ianjamesburke",name:"PLEXI"){issue(number:$n){projectItems(first:5){nodes{id project{id}}}}}}' -F n=<number> --jq '.data.repository.issue.projectItems.nodes[]|select(.project.id=="PVT_kwHOAkOgys4BXaQY")|.id')
[ -n "$_PROJ_ITEM" ] && gh api graphql -f query='mutation($i:ID!,$v:String!){updateProjectV2ItemFieldValue(input:{projectId:"PVT_kwHOAkOgys4BXaQY",itemId:$i,fieldId:"PVTSSF_lAHOAkOgys4BXaQYzhSnRw8",value:{singleSelectOptionId:$v}}){projectV2Item{id}}}' -f i="$_PROJ_ITEM" -f v="f1399a59" > /dev/null
```

**Bundle mode:** PR title `<summary> (#<n1>, #<n2>[, ...])`. Body lists each issue's Done When separately.

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

**2. Poll for Gemini / CodeRabbit bot:**

Wait 5 minutes, then check every 60s up to 10 minutes (6 checks):
```bash
echo "Waiting 5 minutes for AI reviewers..."
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
    gh pr view $PR_NUMBER --json comments \
      --jq '.comments[] | select(.author.login | test("gemini|coderabbit"; "i")) | "[\(.author.login)] \(.body)"'
    gh api repos/ianjamesburke/PLEXI/pulls/$PR_NUMBER/reviews \
      --jq '.[] | select(.user.login | test("gemini|coderabbit"; "i")) | "[\(.user.login)] \(.body)"'
    gh api repos/ianjamesburke/PLEXI/pulls/$PR_NUMBER/comments \
      --jq '.[] | select(.user.login | test("gemini|coderabbit"; "i")) | "[\(.user.login)] \(.body)"'
    break
  fi
  [ "$i" -lt 6 ] && echo "Check $i/6 — no feedback yet, waiting 60s..." && sleep 60
done
```

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

Append to the issue's `## Ship Log` section:

```markdown
**PR:** #<pr-number> — <pr-url>
**AI Review:** <N> fixes | no changes (CodeRabbit: <finding count>, Gemini: <finding count>)
```

```bash
CURRENT_BODY=$(gh issue view <number> --json body --jq '.body')
# Append the PR line to the most recent Ship Log entry
gh issue edit <number> --body "<updated body>"
```

---

## Pipeline Labels

After writing the Ship Log, set pipeline state:

```bash
gh issue edit <number> \
  --add-label "pipeline:validate" \
  --add-label "ready" \
  --remove-label "pipeline:open-pr" \
  --remove-label "in progress"
```

This is the only handoff mechanism. Never spawn a new pane or output "Next: /validate-pr" as an instruction — PM reads the label and dispatches.

---


## Rules

- Never push to alpha, beta, or main directly
- PR must always target `alpha`
- Bundle mode: close all issues in Phase 6 of merge-pr
- The final approver is always the user — AI reviews inform the fix pass, never gate the merge
- If `cr` unavailable: note "cr not available" in the review block
- No bot feedback after 10 minutes: note "No feedback received (10 min timeout)"
