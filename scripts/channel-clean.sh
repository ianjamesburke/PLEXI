#!/usr/bin/env bash
# Channel cleanup uses recorded ownership and retains profile data.
set -euo pipefail
exec bash "$(dirname "${BASH_SOURCE[0]}")/uninstall.sh" "${1:?channel name required}"
