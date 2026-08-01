#!/usr/bin/env bash
# bs-reset.sh — the start-over button for a stint lane (bs- pipeline).
#
# Usage: bs-reset.sh <stint-id>
#
# Force-wipes the stint's worktree and local branch, uninstalls any
# plexi-pr-<N> build whose PR heads that branch, unclaims the stint, then
# re-runs bs-start. Idempotent: missing pieces are skipped with a note, never
# an error. Refuses only when the stint is already done/archived.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [[ $# -ne 1 ]]; then
  echo "bs-reset: missing stint id. Usage: just bs-reset <stint-id>" >&2
  exit 1
fi
stint_id="$1"

alpha_root="$(git worktree list --porcelain | awk '
  /^worktree / { path=substr($0, 10) }
  /^branch refs\/heads\/alpha$/ { print path; exit }
')"
if [[ -z "$alpha_root" ]]; then
  echo "bs-reset: could not resolve the alpha worktree from 'git worktree list'." >&2
  exit 1
fi

# Refuse to reset finished work.
shopt -s nullglob
task_files=("$alpha_root/.stint/tasks/${stint_id}"-*.md)
shopt -u nullglob
if [[ ${#task_files[@]} -eq 0 ]]; then
  echo "bs-reset: no task file matches .stint/tasks/${stint_id}-*.md in $alpha_root — check the id with 'stint list'." >&2
  exit 1
fi
status="$(sed -n 's/^status: *//p' "${task_files[0]}" | head -1)"
if [[ "$status" == "done" || "$status" == "archived" ]]; then
  echo "bs-reset: stint $stint_id is '$status' — refusing to reset finished work." >&2
  exit 1
fi

# Find this stint's lane branch(es). The id can sit anywhere in a multi-id
# batch branch (feature/stint-<id>[-<id>...]-<slug>), so match by segment.
branches="$(git -C "$alpha_root" for-each-ref --format='%(refname:short)' 'refs/heads/feature/stint-*' \
  | grep -E "^feature/stint-([0-9]+-)*${stint_id}-" || true)"
if [[ -z "$branches" ]]; then
  echo "bs-reset: no local branch carries stint ${stint_id} (feature/stint-*${stint_id}-*) — nothing to wipe."
fi

for branch in $branches; do
  # Uninstall any PR build whose open PR heads this branch.
  pr="$(gh pr list --head "$branch" --state open --json number --jq '.[0].number // empty' 2>/dev/null || true)"
  if [[ -n "$pr" ]]; then
    echo "bs-reset: uninstalling PR build plexi-pr-$pr (open PR #$pr heads $branch)"
    (cd "$alpha_root" && just channel-clean "pr-$pr")
  fi

  worktree="$alpha_root/worktrees/$branch"
  if [[ -d "$worktree" ]]; then
    echo "bs-reset: force-removing worktree $worktree and branch $branch"
    (cd "$alpha_root" && wtp remove -f --with-branch "$branch")
  else
    echo "bs-reset: no worktree at $worktree — deleting branch $branch"
    git -C "$alpha_root" worktree prune
    git -C "$alpha_root" branch -D "$branch"
  fi
done

# Unclaim so bs-start's claim starts a fresh timing record.
if [[ "$status" == "in-progress" ]]; then
  (cd "$alpha_root" && stint unclaim "$stint_id")
fi

bash "$SCRIPT_DIR/bs-slot.sh" reset done "stint=$stint_id" "detail=lane wiped, restarting"
exec bash "$SCRIPT_DIR/bs-start.sh" "$stint_id"
