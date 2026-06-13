#!/usr/bin/env bash
# Removes whole worktrees whose PR is MERGED — only when provably safe to delete.
#
# Destructive (deletes the worktree dir and its local branch), so the bar is
# high. A worktree is removed only when ALL of these hold:
#   1. Its checked-out branch has a MERGED PR (not just "no open PR" — that
#      would also match pre-PR dev work and abandoned/closed PRs).
#   2. Working tree is clean: `git status --porcelain` is empty (covers
#      uncommitted tracked files AND untracked files).
#   3. Zero unpushed commits: every local commit is on the remote. If the
#      upstream ref can't be resolved, the work is unverifiable → keep.
#   4. It lives under worktrees/feature/* or worktrees/fix/* (root, beta, main,
#      and temp-* are never considered).
#
# A branch that is MERGED but fails #2 or #3 (you added work after the merge) is
# KEPT and printed with a warning — never silently deleted. clean-stale-targets
# still reclaims its target/, so disk is freed without risking source.
#
# Detection is by the worktree's checked-out branch; removal is by worktree
# name — the two can differ. Squash-merged branches look "not merged" to git, so
# branch deletion uses --force-branch, justified by the verified MERGED state.
#
# Pass `dry-run` as the first arg to print verdicts without deleting anything.
# Runs automatically from merge-pr's cleanup step.
set -euo pipefail

DRY_RUN=0
[[ "${1:-}" == "dry-run" || "${1:-}" == "--dry-run" ]] && DRY_RUN=1

REPO_ROOT="$(git -C "$(dirname "${BASH_SOURCE[0]}")" rev-parse --show-toplevel)"
WORKTREES_DIR="$REPO_ROOT/worktrees"

if [[ ! -d "$WORKTREES_DIR" ]]; then
  echo "No worktrees directory found at $WORKTREES_DIR"
  exit 0
fi

if ! command -v gh >/dev/null 2>&1; then
  echo "warn: gh CLI not found — skipping merged-worktree reap (needs PR state)"
  exit 0
fi

# One network call: every PR's head branch + state.
pr_states="$(gh pr list --state all --limit 1000 --json headRefName,state \
  --jq '.[] | "\(.headRefName)\t\(.state)"' 2>/dev/null || true)"

branch_states() {
  awk -F'\t' -v b="$1" '$1 == b { print $2 }' <<<"$pr_states"
}

removed=0
kept=0

for wt_path in "$WORKTREES_DIR"/feature/* "$WORKTREES_DIR"/fix/*; do
  [[ -d "$wt_path" ]] || continue

  name="${wt_path#"$WORKTREES_DIR"/}"
  branch=$(git -C "$wt_path" rev-parse --abbrev-ref HEAD 2>/dev/null || echo "")
  if [[ -z "$branch" ]]; then
    echo "  keep  $name (could not determine branch)"
    ((kept++)) || true
    continue
  fi

  states="$(branch_states "$branch")"

  # Gate 1: must be MERGED, with nothing still open.
  if grep -qx "OPEN" <<<"$states"; then
    echo "  keep  $name  ($branch — open PR)"
    ((kept++)) || true
    continue
  fi
  if ! grep -qx "MERGED" <<<"$states"; then
    if [[ -n "$states" ]]; then
      echo "  keep  $name  ($branch — PR $(paste -sd, - <<<"$states"), not merged)"
    else
      echo "  keep  $name  ($branch — no PR yet, work in flight)"
    fi
    ((kept++)) || true
    continue
  fi

  # Gate 2: working tree must be clean.
  if [[ -n "$(git -C "$wt_path" status --porcelain 2>/dev/null)" ]]; then
    echo "  KEEP  $name  ($branch — MERGED but working tree is DIRTY; investigate)"
    ((kept++)) || true
    continue
  fi

  # Gate 3: no unpushed commits, and upstream must be resolvable.
  if ! git -C "$wt_path" rev-parse --abbrev-ref --symbolic-full-name '@{upstream}' >/dev/null 2>&1; then
    echo "  KEEP  $name  ($branch — MERGED but no upstream; cannot verify pushed)"
    ((kept++)) || true
    continue
  fi
  unpushed=$(git -C "$wt_path" rev-list '@{upstream}..HEAD' --count 2>/dev/null || echo "?")
  if [[ "$unpushed" != "0" ]]; then
    echo "  KEEP  $name  ($branch — MERGED but $unpushed unpushed commit(s); investigate)"
    ((kept++)) || true
    continue
  fi

  # All gates passed: finished, clean, fully pushed, merged. Safe to remove.
  if [[ "$DRY_RUN" -eq 1 ]]; then
    echo "  REAP  $name  ($branch — MERGED, clean, pushed) [dry-run]"
  else
    echo "  reap  $name  ($branch — MERGED, clean, pushed)"
    wtp remove "$name" --with-branch --force-branch
  fi
  ((removed++)) || true
done

echo ""
if [[ "$DRY_RUN" -eq 1 ]]; then
  echo "Dry-run: would remove $removed worktree(s), keep $kept."
else
  echo "Done: $removed worktree(s) removed, $kept kept."
fi
