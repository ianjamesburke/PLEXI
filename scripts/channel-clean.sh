#!/usr/bin/env bash
# Channel cleanup is idempotent, receipt-scoped, and retains profile data for
# standing channels while removing ephemeral (pr-*, test-hash) profiles.
set -euo pipefail
channel="${1:?channel name required}"
flag="--delete-data"
case "$channel" in
  main|stable|alpha|beta|rc-*) flag="--keep-data" ;;
esac
if bash "$(dirname "${BASH_SOURCE[0]}")/uninstall.sh" "$channel" "$flag"; then
  exit 0
else
  result=$?
  [[ "$result" != 2 ]] || exit 0
  exit "$result"
fi
