#!/usr/bin/env bash
# bs-merge.sh — merge a validated PR and fully retire its lane (bs- pipeline).
#
# Usage: bs-merge.sh <pr-number> [merge-pr flags, e.g. no-issue]
#
# Wraps `just merge-pr` (rebase → squash → sync → cleanup → close), then
# uninstalls the plexi-pr-<N> build, confirms the feature worktree was reaped
# (hard error if not), and publishes a final slot report.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [[ $# -lt 1 ]]; then
  echo "bs-merge: missing PR number. Usage: just bs-merge <pr> [flags]" >&2
  exit 1
fi
pr="$1"; shift

alpha_root="$(git worktree list --porcelain | awk '
  /^worktree / { path=substr($0, 10) }
  /^branch refs\/heads\/alpha$/ { print path; exit }
')"
if [[ -z "$alpha_root" ]]; then
  echo "bs-merge: could not resolve the alpha worktree from 'git worktree list'." >&2
  exit 1
fi

branch="$(gh pr view "$pr" --json headRefName --jq '.headRefName' 2>/dev/null || true)"
if [[ -z "$branch" ]]; then
  echo "bs-merge: could not resolve PR #$pr via gh (gh pr view failed)." >&2
  exit 1
fi

bash "$SCRIPT_DIR/bs-slot.sh" merge working "pr=$pr" "detail=merging $branch"
rc=0
(cd "$alpha_root" && just merge-pr "$pr" "$@") || rc=$?
if [[ "$rc" -ne 0 ]]; then
  bash "$SCRIPT_DIR/bs-slot.sh" merge failed "pr=$pr" "detail=merge-pr exited $rc"
  echo "bs-merge: 'just merge-pr $pr' failed with exit $rc — recover with the merge-* sub-steps (see justfile), then re-run bs-merge only if the merge itself did not land." >&2
  exit "$rc"
fi

# Retire the PR build. channel-clean is safe when nothing is installed.
(cd "$alpha_root" && just channel-clean "pr-$pr")

# merge-pr's cleanup owns worktree reaping — confirm it actually happened.
worktree="$alpha_root/worktrees/$branch"
git -C "$alpha_root" worktree prune
if [[ -d "$worktree" ]]; then
  bash "$SCRIPT_DIR/bs-slot.sh" merge failed "pr=$pr" "detail=worktree not reaped: $worktree"
  echo "bs-merge: PR #$pr merged but its worktree survived cleanup: $worktree — inspect it (uncommitted files?), then remove with 'wtp remove -f $branch'." >&2
  exit 1
fi

bash "$SCRIPT_DIR/bs-slot.sh" merge done "pr=$pr" "verdict=PASS" "detail=merged, pr-$pr uninstalled, worktree reaped"
echo "bs-merge: PR #$pr merged; plexi-pr-$pr uninstalled; worktree reaped."
