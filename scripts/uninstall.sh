#!/usr/bin/env bash
# Removal is receipt-scoped and retains user data; exit 2 means not managed.
set -euo pipefail
channel="${1:-stable}"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if binary="$(bash "$script_dir/distribution-tool.sh" locate --channel "$channel")"; then
  exec env -u PLEXI_CHANNEL -u PLEXI_RUNNING -u PLEXI_SOCKET "$binary" uninstall --yes
else
  result=$?
  [[ "$result" != 2 ]] || echo "No managed installation for $channel; legacy files and user data retained."
  exit "$result"
fi
