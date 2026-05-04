#!/usr/bin/env bash
# Usage: scripts/pr-clean.sh <pr-number>
# Removes app bundle, CLI binary, and profile directory for a specific PR build.
set -euo pipefail

number="${1:?PR number required}"
app="/Applications/Plexi PR${number}.app"
bin="/usr/local/bin/plexi-pr-${number}"
profile="$HOME/.plexi-pr-${number}"
removed=0

if [[ -d "$app" ]]; then
    rm -rf "$app"
    echo "Removed $app"
    removed=1
fi
if [[ -f "$bin" ]]; then
    rm -f "$bin"
    echo "Removed $bin"
    removed=1
fi
if [[ -d "$profile" ]]; then
    rm -rf "$profile"
    echo "Removed $profile"
    removed=1
fi

if [[ $removed -eq 0 ]]; then
    echo "Nothing to clean for PR ${number}"
else
    echo "PR ${number} cleaned up"
fi
