---
name: open-pr
description: "Phase 2 of the PLEXI ship pipeline. Takes a pushed feature branch, creates a PR targeting alpha, updates pipeline labels, and invokes /validate-pr inline. Input: branch name or auto-detect from CWD. Output: PR URL."
risk: medium
source: local
date_added: "2026-05-20"
---

# Open PR

Phase 2 of the ship pipeline. Input: pushed feature branch. Output: PR open, pipeline advanced to validate.

**Entry points:**
- `/open-pr` — auto-detect branch from CWD (must be inside a feature worktree)
- `/open-pr <branch-name>` — explicit branch (e.g. `feature/1234-something`)
- `/open-pr <pr-number>` — existing PR, skip creation and advance labels only

On completion output:
```
[PR OPENED] PR #<n> — <title>
PR: <url>
Pipeline: pipeline:validate + ready set — invoking /validate-pr inline
```

> **Labels are the live state.** Ship Log is audit trail only.

> **Pane status title.** Update so the PM reads state from `plexi pane list`:
> ```bash
> plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL} pane name "#<n> · <state>"
> ```
> States this skill sets: `pr-open`.
>
> **Pane slots.** Source `.agents/skills/_lib/pipeline-slots.sh` and publish `pipeline_slots_set open-pr <issue> <pr> <status> "" ""` at phase boundaries.

---

## Step 1 — Detect Branch and Issue

**Auto-detect from CWD:**
```bash
git branch --show-current
```
Must be a `feature/` or `fix/` branch. Fail loudly if on `alpha`, `beta`, or `main`.

**Explicit branch:** verify it exists on origin:
```bash
git ls-remote origin "refs/heads/<branch>"
```

**Extract issue number(s) from branch:**
- `feature/<number>-...` → single issue
- `feature/bundle-<n1>-<n2>-...` → multiple issues

**Fetch issue title(s) for PR body:**
```bash
gh issue view <number> --json title --jq '.title'
```

---

## Step 2 — Create PR

**Idempotency guard — check for existing PR first:**
```bash
EXISTING_PR=$(gh pr list --head <branch> --state open --json number,url --jq '.[0] // empty')
```
If non-empty: skip `gh pr create`, use the existing PR number, advance pipeline labels, invoke `/validate-pr`.

**Create PR targeting alpha:**

Single issue:
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
EOF
)")
PR_NUMBER=$(echo "$PR_URL" | grep -oE '[0-9]+$')
```

Bundle (`feature/bundle-<n1>-<n2>-...`):
```bash
PR_URL=$(gh pr create \
  --base alpha \
  --head <branch> \
  --title "<short summary> (#<n1>, #<n2>)" \
  --body "$(cat <<'EOF'
Closes #<n1>, Closes #<n2>

## Summary

**#<n1> — <issue-n1-title>**
- <bullet summarizing n1 change>

**#<n2> — <issue-n2-title>**
- <bullet summarizing n2 change>

## Done When — #<n1>
<checklist>

## Done When — #<n2>
<checklist>
EOF
)")
PR_NUMBER=$(echo "$PR_URL" | grep -oE '[0-9]+$')
```

Update pane status:
```bash
plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL} pane name "#<n> · pr-open"
pipeline_slots_set open-pr <n> "$PR_NUMBER" pr-open "" ""
```

---

## Step 3 — Ship Log + Pipeline Labels

Append to `## Ship Log` in each issue's body:
```markdown
**PR:** #<pr-number> — <pr-url>
```

```bash
# Repeat for each issue number N:
CURRENT_BODY=$(gh issue view <N> --json body --jq '.body')
gh issue edit <N> --body "$(printf '%s\n**PR:** #%s — %s\n' "$CURRENT_BODY" "$PR_NUMBER" "$PR_URL")"
```

Set pipeline state on every issue:
```bash
# Repeat for each issue number N:
gh issue edit <N> \
  --add-label "pipeline:validate" \
  --add-label "ready" \
  --remove-label "pipeline:open-pr" \
  --remove-label "in progress"
```

Invoke `/validate-pr <pr-number>` inline in the same pane.

---

## Rules

- PR always targets `alpha` — never `beta` or `main`
- Never push to `alpha`, `beta`, or `main` directly
- Idempotency: re-running on an already-open PR advances labels, never errors
- All quality review happens in `/validate-pr` — open-pr does not run any review
