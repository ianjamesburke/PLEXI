#!/usr/bin/env bash
# Usage: scripts/clear-apps.sh <channel>
# Wipes a channel's installed apps directory. Re-run 'just install' afterwards
# to re-sync from examples/. Useful when an app is renamed or removed.
set -euo pipefail

channel="${1:?channel required — one of: alpha | beta | stable}"

case "$channel" in
    stable) dir="$HOME/.plexi/apps" ;;
    *)      dir="$HOME/.plexi-${channel}/apps" ;;
esac

if [[ ! -d "$dir" ]]; then
    echo "nothing to clear: $dir does not exist"
    exit 0
fi

count=$(find "$dir" -mindepth 1 -maxdepth 1 -type d | wc -l | tr -d ' ')
rm -rf "$dir"/*
echo "Cleared $count app directories from $dir"
echo "Re-run 'just install' from the matching worktree to re-sync from examples/"
