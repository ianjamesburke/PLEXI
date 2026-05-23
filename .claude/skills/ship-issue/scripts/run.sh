#!/usr/bin/env bash
# Orchestrates the full ship pipeline for a single issue.
# Spawns each phase in a new Plexi pane, waits for exit, reads the Ship Log,
# then dispatches the next phase. Resumes mid-pipeline if called on an issue
# that's already in flight.
#
# Usage: run.sh <issue-number>

set -euo pipefail

ISSUE=$1
POLL_INTERVAL=60   # seconds between pane-alive checks
LOG_POLL_INTERVAL=300  # seconds between Ship Log checks when pane is still alive

# ── helpers ──────────────────────────────────────────────────────────────────

log() { echo "[ship-issue #$ISSUE] $*"; }

issue_body() {
  gh issue view "$ISSUE" --json body --jq '.body'
}

ship_log() {
  issue_body | awk '/^## Ship Log/,0'
}

last_marker() {
  # Returns the last [MARKER] line found in the Ship Log section
  ship_log | grep -oE '\[(IMPLEMENTED|PR OPENED|VALIDATED|HARD REJECT|COMPLETE)\]' | tail -1
}

pane_alive() {
  local pane_id=$1
  plexi pane list --json | jq -e ".[] | select(.id == $pane_id)" > /dev/null 2>&1
}

wait_for_pane_exit() {
  local pane_id=$1
  local phase=$2
  log "Waiting for $phase pane ($pane_id) to exit..."
  while pane_alive "$pane_id"; do
    sleep "$POLL_INTERVAL"
  done
  log "$phase pane exited."
}

spawn() {
  local cmd=$1
  local pane_id
  pane_id=$(plexi terminal -- bash -c "c '$cmd'; echo '[dispatch] pane done'; sleep 2" 2>/dev/null)
  echo "$pane_id"
}

notify_orch() {
  local title=$1
  local body=$2
  plexi notify \
    --title "$title" \
    --body "$body" \
    --choice "ok:Dismiss" \
    2>/dev/null || true
}

fail() {
  local reason=$1
  log "FAILED: $reason"
  notify_orch "ship-issue #$ISSUE failed" "$reason"
  exit 1
}

# ── determine resume point ────────────────────────────────────────────────────

MARKER=$(last_marker || true)
log "Current Ship Log marker: ${MARKER:-none}"

case "$MARKER" in
  "")
    START_PHASE="implement"
    ;;
  "[IMPLEMENTED]")
    START_PHASE="open-pr"
    ;;
  "[PR OPENED]")
    START_PHASE="validate"
    ;;
  "[VALIDATED]")
    START_PHASE="merge"
    ;;
  "[COMPLETE]")
    log "Issue #$ISSUE is already complete."
    notify_orch "ship-issue #$ISSUE" "Already complete — nothing to do."
    exit 0
    ;;
  "[HARD REJECT]")
    log "Issue #$ISSUE was hard-rejected. Review the issue body before re-dispatching."
    notify_orch "ship-issue #$ISSUE — hard reject" "Previous attempt hard-rejected. Issue body has Prior Attempts section. Review before re-dispatching."
    exit 1
    ;;
  *)
    fail "Unknown Ship Log marker: $MARKER"
    ;;
esac

log "Resuming at phase: $START_PHASE"

# ── phase: implement ──────────────────────────────────────────────────────────

if [[ "$START_PHASE" == "implement" ]]; then
  log "Spawning /implement-issue $ISSUE..."
  PANE=$(spawn "/implement-issue $ISSUE")
  wait_for_pane_exit "$PANE" "implement-issue"

  MARKER=$(last_marker || true)
  if [[ "$MARKER" != "[IMPLEMENTED]" ]]; then
    fail "implement-issue exited but Ship Log shows '$MARKER' — expected [IMPLEMENTED]. Check pane output."
  fi
  log "implement-issue completed."
fi

# ── phase: open-pr ────────────────────────────────────────────────────────────

if [[ "$START_PHASE" == "implement" || "$START_PHASE" == "open-pr" ]]; then
  # Extract branch from Ship Log
  BRANCH=$(ship_log | grep "^\*\*Branch:\*\*" | tail -1 | awk '{print $2}')
  if [[ -z "$BRANCH" ]]; then
    fail "Could not extract branch name from Ship Log. Check issue #$ISSUE body."
  fi
  log "Spawning /open-pr $BRANCH..."
  PANE=$(spawn "/open-pr $BRANCH")
  wait_for_pane_exit "$PANE" "open-pr"

  MARKER=$(last_marker || true)
  if [[ "$MARKER" != "[PR OPENED]" ]]; then
    fail "open-pr exited but Ship Log shows '$MARKER' — expected [PR OPENED]."
  fi
  log "open-pr completed."
fi

# ── phase: validate ───────────────────────────────────────────────────────────

if [[ "$START_PHASE" == "implement" || "$START_PHASE" == "open-pr" || "$START_PHASE" == "validate" ]]; then
  # Extract PR number from Ship Log
  PR_NUMBER=$(ship_log | grep -oE 'PR #([0-9]+)' | tail -1 | grep -oE '[0-9]+')
  if [[ -z "$PR_NUMBER" ]]; then
    fail "Could not extract PR number from Ship Log. Check issue #$ISSUE body."
  fi
  log "Spawning /validate-pr $PR_NUMBER..."
  PANE=$(spawn "/validate-pr $PR_NUMBER")

  # validate-pr requires human interaction — we wait for pane exit but also
  # poll the Ship Log to surface status updates to the orchestrator output.
  log "validate-pr requires human testing. Polling every ${LOG_POLL_INTERVAL}s for updates..."
  while pane_alive "$PANE"; do
    CURRENT=$(last_marker || true)
    log "  Ship Log: ${CURRENT:-in progress}"
    sleep "$LOG_POLL_INTERVAL"
  done

  MARKER=$(last_marker || true)
  if [[ "$MARKER" == "[HARD REJECT]" ]]; then
    log "validate-pr hard-rejected PR #$PR_NUMBER. Issue #$ISSUE updated. Stopping."
    notify_orch "ship-issue #$ISSUE — hard reject" "PR #$PR_NUMBER hard-rejected after 3 attempts. Issue body updated with Prior Attempts."
    exit 1
  fi
  if [[ "$MARKER" != "[VALIDATED]" ]]; then
    fail "validate-pr exited but Ship Log shows '$MARKER' — expected [VALIDATED] or [HARD REJECT]."
  fi
  log "validate-pr: PR approved."
fi

# ── phase: merge ──────────────────────────────────────────────────────────────

PR_NUMBER=$(ship_log | grep -oE 'PR #([0-9]+)' | tail -1 | grep -oE '[0-9]+')
if [[ -z "$PR_NUMBER" ]]; then
  fail "Could not extract PR number from Ship Log before merge."
fi
log "Spawning /merge-pr $PR_NUMBER..."
PANE=$(spawn "/merge-pr $PR_NUMBER")
wait_for_pane_exit "$PANE" "merge-pr"

MARKER=$(last_marker || true)
if [[ "$MARKER" != "[COMPLETE]" ]]; then
  fail "merge-pr exited but Ship Log shows '$MARKER' — expected [COMPLETE]."
fi

VERSION=$(ship_log | grep -oE 'v[0-9]+\.[0-9]+\.[0-9]+' | tail -1)
log "Done. Issue #$ISSUE shipped as $VERSION."
notify_orch "Shipped #$ISSUE" "Issue #$ISSUE closed — $VERSION"
