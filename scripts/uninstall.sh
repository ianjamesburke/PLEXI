#!/usr/bin/env bash
# Removal is receipt-scoped; retains user data for standing channels and deletes
# ephemeral ~/.plexi-pr-* profile dirs for pr-<N> channels. Exit 2 means not managed.
set -euo pipefail
channel="${1:-stable}"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
extra_flags=()
if [[ "$channel" =~ ^pr-[0-9]+$ ]]; then
  extra_flags+=(--delete-data)
fi
if binary="$(bash "$script_dir/distribution-tool.sh" locate --channel "$channel")"; then
  exec env -u PLEXI_CHANNEL -u PLEXI_RUNNING -u PLEXI_SOCKET "$binary" uninstall --yes "${extra_flags[@]}"
else
  result=$?
  if [[ "$result" == 2 ]]; then
    if [[ "$channel" =~ ^pr-[0-9]+$ && -d "$HOME/.plexi-$channel" ]]; then
      rm -rf "$HOME/.plexi-$channel"
      echo "Removed leftover ephemeral profile ~/.plexi-$channel."
    else
      echo "No managed installation for $channel; legacy files and user data retained."
    fi
  fi
  exit "$result"
fi
