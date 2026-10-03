#!/usr/bin/env bash
# Removal is receipt-scoped and retains user data.
set -euo pipefail
channel="${1:-stable}"
case "$channel" in stable|main) binary=plexi ;; *) binary="plexi-$channel" ;; esac
if ! command -v "$binary" >/dev/null 2>&1; then
  echo "No $binary command found. Use the absolute installed executable with 'uninstall --yes'." >&2
  exit 1
fi
exec env -u PLEXI_CHANNEL -u PLEXI_RUNNING -u PLEXI_SOCKET "$binary" uninstall --yes
