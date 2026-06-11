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

truncate_detail() {
    local max="$1"
    local value="$2"
    if [ "${#value}" -gt "$max" ]; then
        printf '%s' "${value:0:$max}"
    else
        printf '%s' "$value"
    fi
}

basename_detail() {
    local value="$1"
    if [ -n "$value" ]; then
        basename "$value"
    fi
}

tool_detail() {
    local tool_name value base
    tool_name=$(jq -r '.tool_name // empty' <<< "$INPUT" 2>/dev/null || true)
    case "$tool_name" in
        Bash)
            value=$(jq -r '.tool_input.command // empty' <<< "$INPUT" 2>/dev/null || true)
            [ -n "$value" ] && printf 'Bash: %s' "$(truncate_detail 60 "$value")" || printf 'Bash'
            ;;
        Edit)
            value=$(jq -r '.tool_input.file_path // .tool_input.path // empty' <<< "$INPUT" 2>/dev/null || true)
            base=$(basename_detail "$value")
            [ -n "$base" ] && printf 'Edit: %s' "$base" || printf 'Edit'
            ;;
        Write)
            value=$(jq -r '.tool_input.file_path // .tool_input.path // empty' <<< "$INPUT" 2>/dev/null || true)
            base=$(basename_detail "$value")
            [ -n "$base" ] && printf 'Write: %s' "$base" || printf 'Write'
            ;;
        Read)
            value=$(jq -r '.tool_input.file_path // .tool_input.path // empty' <<< "$INPUT" 2>/dev/null || true)
            base=$(basename_detail "$value")
            [ -n "$base" ] && printf 'Read: %s' "$base" || printf 'Read'
            ;;
        WebSearch)
            value=$(jq -r '.tool_input.query // empty' <<< "$INPUT" 2>/dev/null || true)
            [ -n "$value" ] && printf 'WebSearch: %s' "$(truncate_detail 50 "$value")" || printf 'WebSearch'
            ;;
        WebFetch)
            value=$(jq -r '.tool_input.url // empty' <<< "$INPUT" 2>/dev/null || true)
            [ -n "$value" ] && printf 'WebFetch: %s' "$(truncate_detail 60 "$value")" || printf 'WebFetch'
            ;;
        Agent)
            value=$(jq -r '.tool_input.description // .tool_input.prompt // empty' <<< "$INPUT" 2>/dev/null || true)
            [ -n "$value" ] && printf 'Agent: %s' "$(truncate_detail 60 "$value")" || printf 'Agent'
            ;;
        "")
            ;;
        *)
            printf '%s' "$tool_name"
            ;;
    esac
}

DETAIL=""
case "$EVENT" in
    PreToolUse)                       STATE="working"; DETAIL=$(tool_detail) ;;
    SessionStart|UserPromptSubmit)    STATE="working" ;;
    PermissionRequest)                STATE="blocked" ;;
    PostToolUse|PostToolBatch|Stop|StopFailure|SessionEnd) STATE="idle" ;;
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
if [ -n "$DETAIL" ]; then
    ARGS+=("--detail" "$DETAIL")
fi

"$BINARY" "${ARGS[@]}" >/dev/null 2>&1 || true
exit 0
