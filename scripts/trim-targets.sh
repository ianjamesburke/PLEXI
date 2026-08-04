#!/usr/bin/env bash
# Runs `cargo clean --profile dev` in every worktree and the repo root
# to reclaim debug/incremental build artifacts without nuking dep caches.
set -euo pipefail

REPO_ROOT="$(git -C "$(dirname "${BASH_SOURCE[0]}")" rev-parse --show-toplevel)"
WORKTREES_DIR="$REPO_ROOT/worktrees"

# `dry-run` reports what would be reclaimed and deletes nothing. This script
# discovers its own targets from git rather than taking them as arguments, so a
# preview is the only way to confirm the set is right before it is destroyed.
MODE="${1:-}"
if [[ -n "$MODE" && "$MODE" != "dry-run" ]]; then
  echo "usage: trim-targets.sh [dry-run]" >&2
  exit 2
fi

trim() {
  local dir="$1"
  local label="$2"
  if [[ ! -d "$dir/target/debug" ]]; then
    echo "  skip  $label (nothing to reclaim)"
    return
  fi
  local before
  before=$(du -sh "$dir/target" 2>/dev/null | cut -f1)
  if [[ "$MODE" == "dry-run" ]]; then
    local reclaim
    reclaim=$(du -sh "$dir/target/debug" 2>/dev/null | cut -f1)
    echo "  would  $label  (target $before, reclaims $reclaim)"
    return
  fi
  # Remove debug and incremental dirs directly — faster than cargo clean
  # and avoids needing a Cargo.toml in scope.
  rm -rf "$dir/target/debug" "$dir/target/.rustc_info.json"
  local after
  after=$(du -sh "$dir/target" 2>/dev/null | cut -f1)
  echo "  trim  $label  ($before → $after)"
}

# Enumerate worktrees from git, not from a glob of expected path shapes. The
# previous version globbed `worktrees/feature/*`, `worktrees/fix/*`, `beta`, and
# `main`, so it silently skipped every worktree that did not match — including
# stint-numbered branches (`worktrees/0724-impl-host-scope-origin`), ad-hoc ones
# (`worktrees/skill-clean-up`, `worktrees/pr-install`), and anything checked out
# under /private/tmp. Those held the majority of reclaimable space, which meant
# the tool under-reclaimed hardest exactly when disk was lowest. Asking git
# removes the naming convention from the contract entirely.
while IFS= read -r wt_path; do
  [[ -n "$wt_path" ]] || continue
  if [[ "$wt_path" == "$REPO_ROOT" ]]; then
    trim "$wt_path" "alpha (repo root)"
  else
    trim "$wt_path" "${wt_path#"$WORKTREES_DIR/"}"
  fi
done < <(git -C "$REPO_ROOT" worktree list --porcelain | awk '/^worktree /{print substr($0, 10)}')

echo ""
if [[ "$MODE" == "dry-run" ]]; then
  echo "Dry run — nothing deleted. Re-run without 'dry-run' to reclaim."
else
  echo "Done. Run 'cargo build' in any worktree to rebuild from dep cache."
fi
