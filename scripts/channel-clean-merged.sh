#!/usr/bin/env bash
# Usage: scripts/channel-clean-merged.sh
# Removes installation files and ephemeral ~/.plexi-pr-* profile data for any
# PR build whose GitHub PR is no longer open. Reports orphaned worktrees. Requires gh CLI.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel)"

command -v gh >/dev/null 2>&1 || { echo "error: gh CLI is required for merged channel cleanup"; exit 1; }

raw_channels=()
if installed="$(bash "$SCRIPT_DIR/distribution-tool.sh" list 2>/dev/null)"; then
    while IFS= read -r ch; do
        [[ -n "$ch" ]] && raw_channels+=("$ch")
    done < <(printf '%s' "$installed" | python3 -c 'import json,sys; print("\n".join(r.get("channel","") for r in json.load(sys.stdin) if r.get("channel","").startswith("pr-")))' 2>/dev/null || true)
fi

shopt -s nullglob
for dir in "${XDG_DATA_HOME:-$HOME/.local/share}"/plexi/pr-*/; do
    raw_channels+=("$(basename "$dir")")
done
for reg in "${XDG_DATA_HOME:-$HOME/.local/share}"/plexi/installations/pr-*.json; do
    raw_channels+=("$(basename "$reg" .json)")
done
for prof in "$HOME"/.plexi-pr-*/ "$REPO_ROOT"/.plexi-pr-*/; do
    raw_channels+=("$(basename "$prof" | sed 's/^\.plexi-//')")
done
for bin in "${PLEXI_BIN_DIR:-$HOME/.local/bin}"/plexi-pr-*; do
    raw_channels+=("$(basename "$bin" | sed 's/^plexi-//')")
done
for app in "$HOME"/Applications/Plexi\ PR*.app; do
    num="$(basename "$app" .app | sed 's/^Plexi PR//')"
    [[ "$num" =~ ^[0-9]+$ ]] && raw_channels+=("pr-$num")
done
shopt -u nullglob

channels=""
if [[ ${#raw_channels[@]} -gt 0 ]]; then
    channels="$(printf '%s\n' "${raw_channels[@]}" | sort -u)"
fi

while IFS= read -r channel; do
    [[ "$channel" =~ ^pr-[0-9]+$ ]] || continue
    num="${channel#pr-}"
    state=$(gh pr view "$num" --json state -q '.state' 2>/dev/null || echo "NOTFOUND")
    case "$state" in
        CLOSED|MERGED|NOTFOUND)
            echo "PR #$num ($state) — cleaning $channel..."
            bash "$SCRIPT_DIR/channel-clean.sh" "$channel"
            ;;
        OPEN) echo "PR #$num (OPEN) — skipping" ;;
        *) echo "Unknown state '$state' for PR #$num; retained." >&2; exit 1 ;;
    esac
done <<< "$channels"

# Worktree reaping is owned by scripts/reap-merged-worktrees.sh, which enforces
# all four safety gates (MERGED PR state, clean working tree, zero unpushed
# commits, and never touching temp-* or pre-PR work in flight).

