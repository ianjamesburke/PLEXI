#!/usr/bin/env bash
# List all installed Plexi channels with their tier, binary path, and profile dir.
set -euo pipefail

_tier() {
  local name="$1"
  case "$name" in
    main|alpha|beta) echo "production" ;;
    pr-*)            echo "ephemeral" ;;
    *)               echo "development" ;;
  esac
}

printf '%-12s %-12s %-36s %s\n' "Channel" "Tier" "Binary" "Profile"
printf '%-12s %-12s %-36s %s\n' "-------" "----" "------" "-------"

found=0
for bin in /usr/local/bin/plexi /usr/local/bin/plexi-*; do
    [[ -e "$bin" || -L "$bin" ]] || continue
    name="$(basename "$bin")"
    if [[ "$name" == "plexi" ]]; then
        channel="main"
        profile="$HOME/.plexi"
    else
        channel="${name#plexi-}"
        profile="$HOME/.plexi-${channel}"
    fi
    tier="$(_tier "$channel")"
    printf '%-12s %-12s %-36s %s\n' "$channel" "$tier" "$bin" "$profile/"
    found=1
done

if [[ $found -eq 0 ]]; then
    echo "No Plexi channels installed."
fi
