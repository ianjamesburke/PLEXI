#!/usr/bin/env bash
# Usage: scripts/pr-clean-merged.sh
# Removes app bundle, CLI binary, and profile directory for any PR build
# whose GitHub PR is no longer open. Reports orphaned worktrees. Requires gh CLI.
set -euo pipefail

REPO_ROOT="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"

found=0
for profile in "$HOME"/.plexi-pr-*/; do
    [[ -d "$profile" ]] || continue
    num=$(basename "$profile" | sed 's/\.plexi-pr-//')
    state=$(gh pr view "$num" --json state -q '.state' 2>/dev/null || echo "NOTFOUND")
    if [[ "$state" != "OPEN" ]]; then
        found=1
        echo "PR #$num ($state) — cleaning..."
        rm -rf "$profile"
        rm -rf "/Applications/Plexi PR${num}.app"
        rm -f "/usr/local/bin/plexi-pr-${num}"
        echo "  done"
    else
        echo "PR #$num (OPEN) — skipping"
    fi
done

if [[ $found -eq 0 ]]; then
    echo "Nothing to clean"
fi

# Clean orphaned bin symlinks (profile dir may already be gone)
for bin in /usr/local/bin/plexi-pr-*; do
    [[ -L "$bin" ]] || continue
    if [[ ! -e "$bin" ]]; then
        rm -f "$bin"
        echo "Removed dead symlink: $bin"
    fi
done

# Clean orphaned app bundles
for app in /Applications/Plexi\ PR*.app; do
    [[ -d "$app" ]] || continue
    num=$(echo "$app" | sed 's|.*/Plexi PR\([0-9]*\)\.app|\1|')
    state=$(gh pr view "$num" --json state -q '.state' 2>/dev/null || echo "NOTFOUND")
    if [[ "$state" != "OPEN" ]]; then
        rm -rf "$app"
        echo "Removed orphaned app: $app"
    fi
done

# Check for orphaned feature/fix worktrees with no open PR
echo ""
echo "Checking for orphaned worktrees..."
orphans=0
for wt_dir in "$REPO_ROOT"/worktrees/feature/* "$REPO_ROOT"/worktrees/fix/* "$REPO_ROOT"/worktrees/temp-*; do
    [[ -d "$wt_dir" ]] || continue
    branch=$(git -C "$wt_dir" branch --show-current 2>/dev/null) || continue
    [[ -n "$branch" ]] || continue
    open_count=$(gh pr list --head "$branch" --state open --json number -q 'length' 2>/dev/null || echo "0")
    if [[ "$open_count" == "0" ]]; then
        orphans=1
        echo "  ORPHAN: worktrees/$(basename "$(dirname "$wt_dir")")/$(basename "$wt_dir") (branch: $branch) — no open PR"
        echo "          remove with: wtp remove $branch"
    fi
done
if [[ $orphans -eq 0 ]]; then
    echo "No orphaned worktrees found"
fi
