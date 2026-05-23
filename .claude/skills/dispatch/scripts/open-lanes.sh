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

REPO_DIR=$(git rev-parse --show-toplevel 2>/dev/null || echo "$PWD")

if ! git -C "$REPO_DIR" diff --quiet || ! git -C "$REPO_DIR" diff --cached --quiet; then
  echo "ERROR: alpha has uncommitted changes. Commit or stash before dispatching." >&2
  git -C "$REPO_DIR" status --short >&2
  exit 1
fi

PLEXI=plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL}
MY_PANE_ID=${PLEXI_PANE_ID:?PLEXI_PANE_ID not set — must run inside a Plexi pane}

PREV_ID=$MY_PANE_ID
LAYOUT=split_h

for ISSUE in "$@"; do
  PANE_ID=$($PLEXI terminal "c '/implement-issue $ISSUE'" \
    --layout $LAYOUT \
    --from-pane-id $PREV_ID \
    --cwd "$REPO_DIR" \
    --no-focus)
  $PLEXI pane name $PANE_ID "#${ISSUE}"
  echo "Lane opened: pane $PANE_ID → #$ISSUE"
  PREV_ID=$PANE_ID
  LAYOUT=split_v
done
