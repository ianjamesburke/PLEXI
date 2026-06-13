#!/usr/bin/env bash
# Removes target/ from any worktree whose branch's PR is merged or closed.
#
# Squash-merge safe: detection is by GitHub PR state, NOT by `git branch
# --merged alpha`. A squash-merge replays the feature branch as a single new
# commit on alpha, so the feature branch is never an ancestor of alpha and
# `--merged` reports it as active forever. PLEXI squash-merges everything, so
# the old `--merged` check made this sweep a silent no-op and let merged
# worktrees pile up 5-13 GB of target/ each.
#
# Non-destructive: removes only target/ build artifacts, never the worktree
# itself and never any unpushed commits. A reaped target rebuilds from the
# sccache cache on next build. Worktrees with an OPEN PR, or with NO PR at all
# (pre-PR work in flight), are kept.
#
# Runs automatically from merge-pr's cleanup step, and is safe to run any time.
set -euo pipefail

REPO_ROOT="$(git -C "$(dirname "${BASH_SOURCE[0]}")" rev-parse --show-toplevel)"
WORKTREES_DIR="$REPO_ROOT/worktrees"

if [[ ! -d "$WORKTREES_DIR" ]]; then
  echo "No worktrees directory found at $WORKTREES_DIR"
  exit 0
fi

if ! command -v gh >/dev/null 2>&1; then
  echo "warn: gh CLI not found — skipping stale-target sweep (squash-safe detection needs it)"
  exit 0
fi

# One network call: every PR's head branch + state. Build a local lookup so the
# per-worktree loop does zero further network round-trips.
pr_states="$(gh pr list --state all --limit 1000 --json headRefName,state \
  --jq '.[] | "\(.headRefName)\t\(.state)"' 2>/dev/null || true)"

# All PR states recorded for a branch, newline-separated (empty = branch never
# had a PR).
branch_states() {
  awk -F'\t' -v b="$1" '$1 == b { print $2 }' <<<"$pr_states"
}

removed=0
skipped=0

for wt_path in "$WORKTREES_DIR"/feature/* "$WORKTREES_DIR"/fix/*; do
  [[ -d "$wt_path" ]] || continue
  target_dir="$wt_path/target"
  [[ -d "$target_dir" ]] || continue

  branch=$(git -C "$wt_path" rev-parse --abbrev-ref HEAD 2>/dev/null || echo "")
  if [[ -z "$branch" ]]; then
    echo "  skip  $wt_path (could not determine branch)"
    ((skipped++)) || true
    continue
  fi

  states="$(branch_states "$branch")"
  if grep -qx "OPEN" <<<"$states"; then
    echo "  keep  $wt_path  ($branch — open PR)"
    ((skipped++)) || true
  elif [[ -n "$states" ]]; then
    # Had a PR, none open → merged or closed. Reap.
    size=$(du -sh "$target_dir" 2>/dev/null | cut -f1)
    state_label=$(paste -sd, - <<<"$states")
    echo "  clean [$size] $target_dir  ($branch — PR $state_label)"
    rm -rf "$target_dir"
    ((removed++)) || true
  else
    echo "  keep  $wt_path  ($branch — no PR yet, work in flight)"
    ((skipped++)) || true
  fi
done

echo ""
echo "Done: $removed target dir(s) removed, $skipped skipped."
