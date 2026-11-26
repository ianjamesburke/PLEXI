---
name: merge-pr
description: Squash-merge an approved PLEXI PR to alpha and clean up. Input: PR number.
---

# Merge PR

Run from the alpha checkout, only after the owner approved the PR.

```
just merge-pr <PR>            # closes the stints named in the PR body and any linked issue
just merge-pr <PR> no-issue   # standalone PR, nothing to close
```

It confirms the PR is open and targets alpha, squash-merges it with `gh pr merge --squash` (never `--delete-branch`), fast-forwards local alpha while keeping local edits, removes the `pr-<PR>` channel and the PR's worktree if clean, then closes stints and issue.

Nothing is rebuilt or re-tested after approval, and a PR behind alpha merges when GitHub can squash it.

- Conflict: it stops with a message. Resolve on the PR branch and rerun.
- Local alpha refuses the fast-forward (overlapping edits): the PR is already merged. Fix local alpha, rerun; it resumes.
- Stint ids come from `Stint: 0123` lines in the PR body; `Stint-partial: 0123` releases a stint instead of closing it.

Report the merge commit on alpha.
