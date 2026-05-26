#!/usr/bin/env bash
# Usage: scripts/pr-clean.sh <pr-number>
# Alias for: scripts/channel-clean.sh pr-<number>
set -euo pipefail
number="${1:?PR number required}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec "$SCRIPT_DIR/channel-clean.sh" "pr-${number}"
