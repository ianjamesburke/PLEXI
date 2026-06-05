#!/usr/bin/env bash
# Usage: scripts/channel-clean-merged.sh
# Removes app bundle, CLI binary, and profile directory for any PR build
# whose GitHub PR is no longer open. Reports orphaned worktrees. Requires gh CLI.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel)"

command -v gh >/dev/null 2>&1 || { echo "error: gh CLI is required for merged channel cleanup"; exit 1; }

found=0
for profile in "$HOME"/.plexi-pr-*/; do
    [[ -d "$profile" ]] || continue
    num=$(basename "$profile" | sed 's/\.plexi-pr-//')
    if [[ ! "$num" =~ ^[0-9]+$ ]]; then
        echo "Skipping invalid PR directory: $profile"
        continue
    fi
    state=$(gh pr view "$num" --json state -q '.state' 2>/dev/null || echo "NOTFOUND")
    if [[ "$state" != "OPEN" ]]; then
        found=1
        echo "PR #$num ($state) — cleaning..."
        "$SCRIPT_DIR/channel-clean.sh" "pr-${num}"
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
        "$SCRIPT_DIR/channel-clean.sh" "pr-${num}"
        echo "Removed orphaned app: $app"
    fi
done

# Remove orphaned feature/fix worktrees with no open PR
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
        echo "  Removing orphaned worktree: $wt_dir (branch: $branch)"
        git -C "$REPO_ROOT" worktree remove --force "$wt_dir" 2>/dev/null && echo "    removed worktree" || echo "    worktree removal failed, skipping"
        if git -C "$REPO_ROOT" branch --list "$branch" | grep -q .; then
            git -C "$REPO_ROOT" branch -D "$branch" && echo "    deleted local branch: $branch" || echo "    local branch delete failed"
        fi
        if git -C "$REPO_ROOT" ls-remote --heads origin "$branch" | grep -q .; then
            git -C "$REPO_ROOT" push origin --delete "$branch" && echo "    deleted remote branch: $branch" || echo "    remote branch delete failed (may already be gone)"
        fi
    fi
done
if [[ $orphans -eq 0 ]]; then
    echo "No orphaned worktrees found"
fi
