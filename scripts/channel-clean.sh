#!/usr/bin/env bash
# Channel cleanup is idempotent, receipt-scoped and retains profile data.
set -euo pipefail
if bash "$(dirname "${BASH_SOURCE[0]}")/uninstall.sh" "${1:?channel name required}"; then
  exit 0
else
  result=$?
  [[ "$result" != 2 ]] || exit 0
  exit "$result"
fi
