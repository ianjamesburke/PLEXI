#!/usr/bin/env bash
# Claude Code agent state hook for PLEXI.
# Registered in ~/.claude/settings.json by `plexi agent hook install --claude-code`.
# Fires on lifecycle events and reports working/blocked/idle state to the host.

set -euo pipefail

# Exit silently if not inside a PLEXI pane.
if [ -z "${PLEXI_SOCKET:-}" ] || [ -z "${PLEXI_PANE_ID:-}" ]; then
    exit 0
fi

# Read JSON from stdin; extract hook_event_name.
INPUT=$(cat)
EVENT=$(jq -r '.hook_event_name // empty' <<< "$INPUT" 2>/dev/null || true)

case "$EVENT" in
    SessionStart|UserPromptSubmit)    STATE="working" ;;
    PermissionRequest)                STATE="blocked" ;;
    Stop|StopFailure|SessionEnd)      STATE="idle" ;;
    SubagentStop)                     exit 0 ;;  # skip — avoid false idle
    *)                                exit 0 ;;  # unknown event, skip
esac

SESSION_ID=$(jq -r '.session_id // empty' <<< "$INPUT" 2>/dev/null || true)

# Build plexi binary name from the socket path channel suffix.
BINARY="plexi"
if [[ "$PLEXI_SOCKET" =~ plexi-([^/]+)/notify\.sock ]]; then
    SUFFIX="${BASH_REMATCH[1]}"
    BINARY="plexi-${SUFFIX}"
fi

ARGS=("agent" "report" "--state" "$STATE" "--agent" "claude-code")
if [ -n "$SESSION_ID" ]; then
    ARGS+=("--session-id" "$SESSION_ID")
fi

"$BINARY" "${ARGS[@]}" >/dev/null 2>&1 || true
exit 0
