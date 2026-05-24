#!/usr/bin/env bash
# Usage: scripts/channel-clean.sh <channel-name>
# Removes the app bundle, CLI binary, and profile directory for any channel.
# Examples: channel-clean alpha, channel-clean gpui, channel-clean pr-123
set -euo pipefail

channel="${1:?channel name required}"

if [[ "$channel" == "main" ]]; then
  echo "This will remove the main Plexi channel (Plexi.app, /usr/local/bin/plexi, ~/.plexi/)."
  read -r -p "Proceed? [y/N] " confirm
  [[ "$confirm" =~ ^[Yy]$ ]] || { echo "Aborted."; exit 0; }
  suffix=""
  cap=""
else
  suffix="-$channel"
  cap=" $(echo "$channel" | awk '{print toupper(substr($0,1,1)) substr($0,2)}')"
  # Handle pr-N: capitalize as "PR<N>"
  if [[ "$channel" =~ ^pr-([0-9]+)$ ]]; then
    cap=" PR${BASH_REMATCH[1]}"
  fi
fi

app="/Applications/Plexi${cap}.app"
bin="/usr/local/bin/plexi${suffix}"
profile="$HOME/.plexi${suffix}"
removed=0

if [[ -d "$app" ]]; then
    rm -rf "$app"
    echo "Removed $app"
    removed=1
fi
if [[ -e "$bin" || -L "$bin" ]]; then
    rm -f "$bin"
    echo "Removed $bin"
    removed=1
fi
if [[ -d "$profile" ]]; then
    rm -rf "$profile"
    echo "Removed $profile"
    removed=1
fi

# For main channel, also remove the bare plexi symlink if it exists
if [[ "$channel" == "main" ]]; then
    if [[ -e /usr/local/bin/plexi || -L /usr/local/bin/plexi ]]; then
        rm -f /usr/local/bin/plexi
        echo "Removed /usr/local/bin/plexi (bare symlink)"
        removed=1
    fi
fi

if [[ $removed -eq 0 ]]; then
    echo "Nothing to clean for channel ${channel}"
else
    echo "Channel ${channel} cleaned"
fi
