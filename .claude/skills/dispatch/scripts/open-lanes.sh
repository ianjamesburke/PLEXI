#!/usr/bin/env bash
# Usage: open-lanes.sh <issue1> [issue2] [issue3] [issue4]
# Opens dispatch lane panes relative to the orchestrator pane.
# Lane 1: split_h right of self. Lane 2+: split_v below previous.
# All panes opened with --no-focus so orchestrator keeps focus.
set -euo pipefail

if [[ $# -eq 0 ]]; then
  echo "Usage: open-lanes.sh <issue1> [issue2] [issue3] [issue4]" >&2
  exit 1
fi

PLEXI=plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL}
MY_PANE_ID=${PLEXI_PANE_ID:?PLEXI_PANE_ID not set — must run inside a Plexi pane}

PREV_ID=$MY_PANE_ID
LAYOUT=split_h

for ISSUE in "$@"; do
  PANE_ID=$($PLEXI terminal --layout $LAYOUT --from-pane-id $PREV_ID --no-focus)
  $PLEXI pane name $PANE_ID "#${ISSUE}"
  $PLEXI pane send $PANE_ID 'c "/ship-issue '"$ISSUE"'"'$'\n'
  echo "Lane opened: pane $PANE_ID → #$ISSUE"
  PREV_ID=$PANE_ID
  LAYOUT=split_v
done
