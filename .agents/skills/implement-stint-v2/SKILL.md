---
name: implement-stint-v2
description: Implement one or more stints in an isolated worktree off alpha, run the tests for what changed, and open a PR only if a person needs to try it.
---

# Implement Stint

1. `stint claim <id>`, then `wtp add -b feature/stint-<id>-<slug>` off alpha. Check the new HEAD equals `git rev-parse origin/alpha`.
2. Implement. Follow [TESTING.md](../../../src/testing/TESTING.md): run the tests that cover the change, add one if none does, and look at the screenshot for visible UI changes (see the `testing` skill).
3. Commit.
4. Needs a person to try it: push, `gh pr create --base alpha` with `Stint: <id>` in the body (about 5 lines), run `just pr-install <PR>`, and give the PR URL plus one line on what to check.
5. On approval: `just merge-pr <PR>` (see the `merge-pr` skill).

A change nobody needs to try can be committed and pushed to alpha directly when the user is watching, then `stint done <id>`.

Pre-existing defects outside your diff become stints, not fixes. Never `git stash`; use a `WIP:` commit.
