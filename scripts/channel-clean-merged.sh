#!/usr/bin/env bash
# Usage: scripts/channel-clean-merged.sh
# Removes recorded installation files and retains user data for any PR build
# whose GitHub PR is no longer open. Reports orphaned worktrees. Requires gh CLI.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel)"

command -v gh >/dev/null 2>&1 || { echo "error: gh CLI is required for merged channel cleanup"; exit 1; }

installed="$(bash "$SCRIPT_DIR/distribution-tool.sh" list)"
channels="$(printf '%s' "$installed" | uv run --no-project --python 3.11 python -c 'import json,sys; print("\n".join(r["channel"] for r in json.load(sys.stdin) if r["channel"].startswith("pr-")))')"
while IFS= read -r channel; do
    [[ "$channel" =~ ^pr-[0-9]+$ ]] || continue
    num="${channel#pr-}"
    state=$(gh pr view "$num" --json state -q '.state')
    case "$state" in
        CLOSED|MERGED) bash "$SCRIPT_DIR/channel-clean.sh" "$channel" ;;
        OPEN) echo "PR #$num (OPEN) — skipping" ;;
        *) echo "Unknown state for PR #$num; retained." >&2; exit 1 ;;
    esac
done <<< "$channels"

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
