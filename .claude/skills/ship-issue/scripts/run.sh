#!/usr/bin/env bash
# Starts the ship pipeline for a single issue.
#
# The pipeline is self-orchestrating:
#   implement-issue → open-pr (inline) → hand-off → validate-pr → merge-pr (inline)
#
# This script just spawns implement-issue. Everything else chains automatically.
#
# Usage: run.sh <issue-number>

set -euo pipefail

ISSUE=$1
REPO_DIR=$(git rev-parse --show-toplevel)
PLEXI=plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL}

log() { echo "[ship-issue #$ISSUE] $*"; }

if [ -z "${PLEXI_PANE_ID:-}" ]; then
  echo "ERROR: PLEXI_PANE_ID not set — must run inside a Plexi pane." >&2
  exit 1
fi

log "Spawning /implement-issue $ISSUE..."
$PLEXI terminal "c '/implement-issue $ISSUE'" \
  --cwd "$REPO_DIR" \
  --no-focus

log "Pipeline is self-orchestrating:"
log "  implement-issue → open-pr (inline) → /hand-off → validate-pr → merge-pr (inline)"
