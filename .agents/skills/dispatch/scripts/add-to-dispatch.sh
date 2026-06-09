#!/usr/bin/env bash
# Usage: add-to-dispatch.sh <existing_pane_id> <issue_number>
# Adds a new issue as a vertical split below an existing dispatch pane.
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=../../_lib/plexi-env.sh
source "$SCRIPT_DIR/../../_lib/plexi-env.sh"

if [[ $# -ne 2 ]]; then
  echo "Usage: add-to-dispatch.sh <existing_pane_id> <issue_number>" >&2
  exit 1
fi

EXISTING_PANE_ID=$1
ISSUE=$2

PANE_ID=$($PLEXI terminal \
  --layout split_v \
  --from-pane-id "$EXISTING_PANE_ID" \
  --cwd "$REPO_DIR" \
  --no-focus \
  "c '/implement-issue $ISSUE'")
$PLEXI pane name "$PANE_ID" "#${ISSUE}"
echo "Added: pane $PANE_ID → #$ISSUE (below pane $EXISTING_PANE_ID)"
