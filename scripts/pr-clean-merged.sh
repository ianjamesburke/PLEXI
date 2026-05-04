#!/usr/bin/env bash
# Usage: scripts/pr-clean-merged.sh
# Removes app bundle, CLI binary, and profile directory for any PR build
# whose GitHub PR is no longer open. Requires gh CLI.
set -euo pipefail

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
