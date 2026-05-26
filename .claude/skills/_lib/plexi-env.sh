#!/usr/bin/env bash
# Shared env setup for all ship-pipeline scripts.
# Source this at the top of any script that needs Plexi context.
#
# Exports:
#   PLEXI      — channel-aware binary name (e.g. plexi-alpha)
#   MY_PANE_ID — current pane ID (fails loudly if not in a Plexi pane)
#   REPO_DIR   — absolute path to the git repo root

PLEXI=plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL}
MY_PANE_ID=${PLEXI_PANE_ID:?Must run inside a Plexi pane — PLEXI_PANE_ID is unset}
REPO_DIR=$(git rev-parse --show-toplevel 2>/dev/null || echo "$PWD")

export PLEXI MY_PANE_ID REPO_DIR
