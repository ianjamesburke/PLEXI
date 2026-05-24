#!/usr/bin/env bash
# Alias for scripts/channel-clean-merged.sh
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec "$SCRIPT_DIR/channel-clean-merged.sh" "$@"
