---
name: merge-pr
description: "Phase 4 of the PLEXI ship pipeline. Takes an approved PR number, squash-merges to alpha, bumps the version, installs, closes the issue, and cleans up. Input: PR number. Output: installed alpha build at new version."
risk: medium
source: local
date_added: "2026-05-20"
---

# Merge PR

Phase 4 of the ship pipeline. Input: approved PR number. Output: clean alpha at new version.

**Entry:** `/merge-pr <pr-number>`

On completion, appends final entry to the issue's Ship Log and outputs:

```
[COMPLETE]
- Merged: PR #<n> — <title>
- Closed: Issue #<n> — <title>
- Version: v<x.y.z>
```

---

## Step 0 — Read Context

```bash
gh pr view <pr-number> --json title,headRefName,number,baseRefName,state,mergeStateStatus
```

Extract:
- `BRANCH` = `headRefName`
- `ISSUE_NUMBER` = parse from branch name (or from PR body `Closes #<n>`)
- `PR_STATE` = check if already `MERGED`

**If already MERGED:** skip to Step 5 (close issue). Another session merged it.

**CWD for all steps: the repo root. Set it now:**
```bash
cd /Users/ianburke/Documents/GitHub/PLEXI
```

---

## Step 1 — Discard Artifacts, Stash Local Edits

```bash
git restore Cargo.toml
git -C worktrees/$BRANCH restore Cargo.toml
DIRTY=$(git status --porcelain | grep -v "^??" | grep -v "Cargo.toml")
if [ -n "$DIRTY" ]; then
  git stash push -m "session edits — restore after merge"
  STASHED=1
fi
```

---

## Step 2 — Rebase and Push

```bash
git fetch origin
git -C worktrees/$BRANCH rebase origin/alpha
# On conflict: git -C worktrees/$BRANCH add <files> && GIT_EDITOR=true git -C worktrees/$BRANCH rebase --continue
git -C worktrees/$BRANCH push --force-with-lease origin HEAD
```

> If anything non-obvious happened this PR: add one entry to `GOTCHAS.md` in the feature worktree and commit it there before pushing. It lands in the squash commit. Do not write to alpha's GOTCHAS.md directly.

---

## Step 3 — Squash Merge

```bash
gh pr merge $PR_NUMBER --squash
```

If branch protection blocks: `gh pr merge $PR_NUMBER --squash --admin`

Wait ~10 seconds for GitHub's merge state to update before proceeding.

---

## Step 4 — Sync Alpha

```bash
git fetch origin
git reset --hard origin/alpha
```

Restore stash if created:
```bash
if [ "${STASHED:-0}" = 1 ]; then
  git stash pop
  PR_FILES=$(git show origin/alpha --name-only --format="" | grep .)
  while IFS= read -r f; do
    if ! printf '%s\n' "$PR_FILES" | grep -qxF "$f"; then
      printf 'WARNING: %s from stash is unrelated to PR — discarding\n' "$f"
      git restore "$f"
    fi
  done < <(git diff --name-only)
  REMAINING=$(git diff --name-only)
  if [ -n "$REMAINING" ]; then
    git add -u && git commit -m "chore: restore session edits carried through merge"
  fi
fi
```

If `stash pop` conflicts: take squash version (`git checkout --theirs <file>`).

---

## Step 5 — Cleanup

```bash
rm -f test_pr$PR_NUMBER.py
just pr-clean $PR_NUMBER        # skip if no pr-install was run (diff-review path)
wtp remove $BRANCH --force
git push origin --delete $BRANCH
```

---

## Step 6 — Bump, Install, Push

```bash
just bump && just install
git push
```

Read the new version:
```bash
VERSION=$(grep '^version' Cargo.toml | head -1 | grep -oE '[0-9]+\.[0-9]+\.[0-9]+')
```

---

## Step 7 — Close Issue and Update Project Board

```bash
gh issue close $ISSUE_NUMBER --comment "Closed by PR #$PR_NUMBER — verified on alpha v$VERSION"
_PROJ_ITEM=$(gh api graphql -f query='query($n:Int!){repository(owner:"ianjamesburke",name:"PLEXI"){issue(number:$n){projectItems(first:5){nodes{id project{id}}}}}}' -F n=$ISSUE_NUMBER --jq '.data.repository.issue.projectItems.nodes[]|select(.project.id=="PVT_kwHOAkOgys4BXaQY")|.id')
[ -n "$_PROJ_ITEM" ] && gh api graphql -f query='mutation($i:ID!,$v:String!){updateProjectV2ItemFieldValue(input:{projectId:"PVT_kwHOAkOgys4BXaQY",itemId:$i,fieldId:"PVTSSF_lAHOAkOgys4BXaQYzhSnRw8",value:{singleSelectOptionId:$v}}){projectV2Item{id}}}' -f i="$_PROJ_ITEM" -f v="98236657" > /dev/null
```

**Bundle mode:** close all issues:
```bash
for N in <n1> <n2> ...; do
  gh issue close $N --comment "Closed by PR #$PR_NUMBER — verified on alpha v$VERSION"
done
```

---

## Step 8 — Append Final Ship Log Entry

```bash
CURRENT_BODY=$(gh issue view $ISSUE_NUMBER --json body --jq '.body')
# Append to Ship Log:
# **Merged:** PR #<pr-number> → alpha v<version> (<YYYY-MM-DD>)
gh issue edit $ISSUE_NUMBER --body "<updated body>"
```

---

## Step 9 — Unblock Downstream Issues

```bash
gh issue-ext blocking list $ISSUE_NUMBER --json number,title,state 2>/dev/null \
  | jq '.[] | select(.state == "OPEN") | {number, title}'
```

For each match, check if all its blockers are now closed:
```bash
gh issue-ext blocking list <blocking-issue-number> --json number,state 2>/dev/null \
  | jq 'all(.state == "CLOSED")'
```

If true: `gh issue edit <n> --remove-label "blocked" --add-label "ready"`

Report any issues unblocked.

---

## Step 10 — Notify and Close Pane

```bash
git status  # must be clean
```

**Clean exit (no deferred threads):**
```bash
(RESULT=$(plexi notify \
  --title "Shipped #$ISSUE_NUMBER" \
  --body "<title> — v$VERSION" \
  --choice "ok:Dismiss" \
  --choice "open:Open PR")
 [ "$RESULT" = "open" ] && open "<pr-url>") &
plexi pane close
```

**Soft exit (deferred threads / improvements proposed):**
```bash
plexi notify \
  --title "Shipped #$ISSUE_NUMBER — review needed" \
  --body "<title> — v$VERSION. <N> improvement(s) proposed." \
  --choice "a:Talk to Claude:pane_focus:$PLEXI_PANE_ID" \
  --choice "b:Approve all" \
  --choice "c:Skip all"
```

---

## Rules

- CWD must be repo root for all commands in this skill
- `just pr-clean`, `just bump`, `just install` run from repo root
- `just pr-install` runs from the feature worktree (only if re-needed)
- Never pass `--delete-branch` to `gh pr merge` — git refuses to delete a branch checked out by a worktree
- Never commit directly to alpha, beta, or main
- Alpha must be clean when this skill exits
- On unrecoverable failure: comment on issue, remove `in progress` label, add `ready`, exit
- `git status` clean check is the final gate before notify
