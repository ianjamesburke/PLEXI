#!/usr/bin/env bash
# Usage: add-to-dispatch.sh <existing_pane_id> <issue_number>
# Adds a new issue as a vertical split below an existing dispatch pane.
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "Usage: add-to-dispatch.sh <existing_pane_id> <issue_number>" >&2
  exit 1
fi

EXISTING_PANE_ID=$1
ISSUE=$2

PLEXI=plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL}

PANE_ID=$($PLEXI terminal --layout split_v --from-pane-id $EXISTING_PANE_ID --no-focus)
$PLEXI pane name $PANE_ID "#${ISSUE}"
$PLEXI pane send $PANE_ID 'c "/ship-issue '"$ISSUE"'"'$'\n'
echo "Added: pane $PANE_ID → #$ISSUE (below pane $EXISTING_PANE_ID)"
