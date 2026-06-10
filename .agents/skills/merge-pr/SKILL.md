---
name: merge-pr
description: "Phase 4 of the PLEXI ship pipeline. Takes an approved PR number, squash-merges to alpha, bumps the version, closes the issue, and cleans up. Input: PR number. Output: merged alpha at new version."
risk: medium
source: local
date_added: "2026-05-20"
---

# Merge PR

Phase 4 of the ship pipeline. Input: approved PR number. Output: clean alpha at new version.

> **Labels are the live state.** On success, all `pipeline:*` labels are removed when the issue closes. On failure, remove all `pipeline:*` labels and `in progress`, add `ready`.

**Entry:** `/merge-pr <pr-number>`

On completion, appends final entry to the issue's Ship Log, fires a notify, closes the pane, and outputs `[COMPLETE]` as the last line before close — never before.

> **Pane status title.** Runs in the same dispatched pane (named `#<n>`, or `#<n1>+<n2>` for a bundle). Update the title so the PM reads state from `plexi pane list` instead of capturing content:
> ```bash
> plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL} pane name "#<n> · <state>"
> ```
> **The status word must never contain a digit** (the PM maps panes to issues via `grep -oE '[0-9]+'`). States this skill sets: `merging`, then `done` just before the pane self-closes.

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

**CWD for all steps: the repo root. Set it now, and flip pane status to `merging`:**
```bash
cd "$(git rev-parse --show-toplevel)"
plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL} pane name "#<n> · merging"
```

> Labels are already correct when invoked inline from validate-pr. No label edit needed here.

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

## Step 2 — Rebase and Push (conditional)

```bash
git fetch origin
BEHIND=$(git -C worktrees/$BRANCH rev-list HEAD..origin/alpha --count 2>/dev/null || echo 0)
if [ "$BEHIND" -gt 0 ]; then
  git -C worktrees/$BRANCH rebase origin/alpha
  # On conflict: git -C worktrees/$BRANCH add <files> && GIT_EDITOR=true git -C worktrees/$BRANCH rebase --continue
  git -C worktrees/$BRANCH push --force-with-lease origin HEAD
fi
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
just channel-clean pr-$PR_NUMBER        # skip if no pr-install was run (diff-review path)
wtp remove $BRANCH --force --with-branch
git push origin --delete $BRANCH 2>/dev/null || true  # remote may already be gone
```

---

## Step 6 — Bump and Push

```bash
just bump
git push
```

> `just install` is not run here — the user handles install after merge.

Read the new version:
```bash
VERSION=$(grep '^version' Cargo.toml | head -1 | grep -oE '[0-9]+\.[0-9]+\.[0-9]+')
```

---

## Step 7 — Remove Pipeline Labels And Close Issue

```bash
# Remove pipeline labels before closing (closing alone doesn't remove labels)
gh issue edit $ISSUE_NUMBER \
  --remove-label "pipeline:merge" \
  --remove-label "pipeline:validate" \
  --remove-label "pipeline:open-pr" \
  --remove-label "pipeline:implement" \
  --remove-label "in progress" 2>/dev/null || true
gh issue close $ISSUE_NUMBER --comment "Closed by PR #$PR_NUMBER — verified on alpha v$VERSION"
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

## Step 10 — Complete and Close Pane

`[COMPLETE]` is output here, as part of the close sequence — never earlier. Run this block in full before stopping.

```bash
git status  # must be clean
```

Print the completion marker:
```
[COMPLETE]
- Merged: PR #<n> — <title>
- Closed: Issue #<n> — <title>
- Version: v<x.y.z>
```

Then immediately close — no stopping after the marker:

**Clean exit (no deferred threads):**
```bash
plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL} pane name "#<n> · done"
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
# Use needs-you, NOT done — the pane stays alive for the user, and the PM must
# not free the slot / close it while interaction is pending.
plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL} pane name "#<n> · needs-you"
plexi notify \
  --title "Shipped #$ISSUE_NUMBER — review needed" \
  --body "<title> — v$VERSION. <N> improvement(s) proposed." \
  --choice "a:Talk to Claude:pane_focus:$PLEXI_PANE_ID" \
  --choice "b:Approve all" \
  --choice "c:Skip all"
# pane stays alive for user interaction — do NOT call plexi pane close here
```

---

## Rules

- CWD must be repo root for all commands in this skill
- `just channel-clean pr-<N>`, `just bump` run from repo root — `just install` is the user's responsibility post-merge
- `just pr-install` runs from the feature worktree (only if re-needed)
- Never pass `--delete-branch` to `gh pr merge` — git refuses to delete a branch checked out by a worktree
- Never commit directly to alpha, beta, or main
- Alpha must be clean when this skill exits
- On unrecoverable failure: set `plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL} pane name "#<n> · blocked"`, comment on issue, remove `in progress` and all `pipeline:*` labels, add `ready`, exit
- `git status` clean check is the final gate before notify
