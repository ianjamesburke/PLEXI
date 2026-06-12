#!/usr/bin/env bash
# Removes target/ from any worktree whose branch is already merged to alpha.
# Safe to run any time — never touches active branches.
set -euo pipefail

REPO_ROOT="$(git -C "$(dirname "${BASH_SOURCE[0]}")" rev-parse --show-toplevel)"
WORKTREES_DIR="$REPO_ROOT/worktrees"

if [[ ! -d "$WORKTREES_DIR" ]]; then
  echo "No worktrees directory found at $WORKTREES_DIR"
  exit 0
fi

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

  if git -C "$REPO_ROOT" branch --merged alpha 2>/dev/null | grep -qF "$branch"; then
    size=$(du -sh "$target_dir" 2>/dev/null | cut -f1)
    echo "  clean [$size] $target_dir  ($branch — merged)"
    rm -rf "$target_dir"
    ((removed++)) || true
  else
    echo "  keep  $wt_path  ($branch — active)"
    ((skipped++)) || true
  fi
done

echo ""
echo "Done: $removed target dir(s) removed, $skipped skipped."
