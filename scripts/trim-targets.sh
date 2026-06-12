#!/usr/bin/env bash
# Runs `cargo clean --profile dev` in every worktree and the repo root
# to reclaim debug/incremental build artifacts without nuking dep caches.
set -euo pipefail

REPO_ROOT="$(git -C "$(dirname "${BASH_SOURCE[0]}")" rev-parse --show-toplevel)"
WORKTREES_DIR="$REPO_ROOT/worktrees"

trim() {
  local dir="$1"
  local label="$2"
  if [[ ! -d "$dir/target" ]]; then
    echo "  skip  $label (no target/)"
    return
  fi
  local before after
  before=$(du -sh "$dir/target" 2>/dev/null | cut -f1)
  # Remove debug and incremental dirs directly — faster than cargo clean
  # and avoids needing a Cargo.toml in scope.
  rm -rf "$dir/target/debug" "$dir/target/.rustc_info.json"
  after=$(du -sh "$dir/target" 2>/dev/null | cut -f1)
  echo "  trim  $label  ($before → $after)"
}

trim "$REPO_ROOT" "alpha (repo root)"

if [[ -d "$WORKTREES_DIR" ]]; then
  for wt_path in "$WORKTREES_DIR"/feature/* "$WORKTREES_DIR"/fix/* "$WORKTREES_DIR"/beta "$WORKTREES_DIR"/main; do
    [[ -d "$wt_path" ]] || continue
    name="${wt_path#"$WORKTREES_DIR/"}"
    trim "$wt_path" "$name"
  done
fi

echo ""
echo "Done. Run 'cargo build' in any worktree to rebuild from dep cache."
