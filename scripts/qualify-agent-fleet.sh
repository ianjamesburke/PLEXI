#!/usr/bin/env bash
# scripts/qualify-agent-fleet.sh
# Qualification script for agent fleet human names, age bucketing, failure reasons,
# dismissal, and history preservation (stint 0965, #2873 / 1df1cddb).
#
# NOTE: This script identifies the exact fleet and history commands and surfaces
# to inspect. It does not claim execution until it is run.
#
# Usage:
#   scripts/qualify-agent-fleet.sh [--help] [--live] [CHANNEL_OR_BIN]
#
# Commands & surfaces inspected by this qualification:
# 1. CLI Fleet Roster:
#    - `plexi agent fleet` (active tasks, counts, roles, harnesses, ages, next hints, pane IDs)
#    - `plexi agent fleet --json` (structured AgentsView with `tasks` and `older`)
#    - `plexi agent fleet --filter needs-attention|working|done|all`
# 2. Attention Selection:
#    - `plexi agent next` (human-facing selection "<role> (<harness>) · <task_title>  #<id>")
# 3. Dismissal:
#    - `plexi agent fleet dismiss <pane-id>`
#    - `plexi agent fleet dismiss --all-failed`
# 4. Preserved History:
#    - `plexi agent history` (retains completed and dismissed runs on disk)
# 5. UI Surfaces (Cmd+Shift+A or app "agents"):
#    - Agents pane task rollups, status badges, failure details, next action buttons
#    - Older failure fold and dismissal actions
#    - Command Palette (Cmd+P) human-facing labels (no raw task/pane ID headers)
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  cat <<'EOF'
Plexi Agent Fleet Qualification Script (Stint 0965)

Identifies and qualifies shipped agent fleet features from #2873 (1df1cddb):
- Human name resolution (role -> pane_name -> harness; strips raw "Task #<id>" prefixes)
- Age bucketing (0s ago, 1m ago, 1h ago, 1d ago; >12h or non-live failures grouped into `older`)
- Failure reason mapping (schema validation, permission denied, harness not found, timeout)
- Dismissal (`plexi agent fleet dismiss <id>` / `--all-failed` removes from fleet view)
- History preservation (dismissed and completed runs remain intact in `plexi agent history`)
- Real Pi agent lifecycle transitions: working -> waiting (Needs you) -> done -> failed

Surfaces:
  CLI:
    plexi agent fleet
    plexi agent fleet --json
    plexi agent fleet --filter <all|needs-attention|working|done>
    plexi agent next
    plexi agent fleet dismiss <pane-id>
    plexi agent fleet dismiss --all-failed
    plexi agent history
  Host UI:
    Agents pane (`plexi app open agents`)
    Command Palette (Cmd+P) agent entries
    Needs-you attention bar and pane border escalation

Run modes:
  ./scripts/qualify-agent-fleet.sh          # Print qualification contract & inspect target binary/channel
  ./scripts/qualify-agent-fleet.sh --live   # Drive Pi reports through working, waiting, done, and failed
EOF
  exit 0
fi

LIVE_MODE=0
TARGET="${1:-}"
if [[ "$TARGET" == "--live" ]]; then
  LIVE_MODE=1
  TARGET="${2:-}"
fi

# The live shim must be copied from a native host executable. Installed channel
# commands are shell launchers whose basename hardcodes their channel, so copying
# one would silently target that installed profile rather than LIVE_CHAN.
is_native_plexi_binary() {
  local kind
  kind="$(file -bL "$1" 2>/dev/null || true)"
  [[ "$kind" == *"Mach-O"* || "$kind" == *"ELF"* ]]
}

resolve_channel_target() {
  local requested="$1"
  if [[ -x "$requested" ]]; then
    printf '%s\n' "$requested"
  elif [[ "$requested" != */* ]] && command -v "plexi-$requested" >/dev/null 2>&1; then
    command -v "plexi-$requested"
  elif [[ "$requested" != */* ]] && command -v "$requested" >/dev/null 2>&1; then
    command -v "$requested"
  else
    return 1
  fi
}

resolve_real_plexi_binary() {
  local candidate="$1" wrapper_name channel resolved
  if is_native_plexi_binary "$candidate"; then
    printf '%s\n' "$candidate"
    return 0
  fi

  wrapper_name="$(basename "$candidate")"
  if [[ "$wrapper_name" == plexi-* ]]; then
    channel="${wrapper_name#plexi-}"
  elif [[ "$wrapper_name" == "plexi" ]]; then
    channel="stable"
  else
    echo "error: $candidate is not a native Plexi executable" >&2
    return 1
  fi
  resolved="$(bash "$ROOT_DIR/scripts/distribution-tool.sh" locate --channel "$channel" 2>/dev/null | tail -n 1)"
  if [[ -z "$resolved" || ! -x "$resolved" ]] || ! is_native_plexi_binary "$resolved"; then
    echo "error: launcher $candidate did not resolve a native Plexi executable" >&2
    return 1
  fi
  printf '%s\n' "$resolved"
}

if [[ -n "$TARGET" ]]; then
  CANDIDATE="$(resolve_channel_target "$TARGET")" || {
    echo "error: $TARGET is neither an executable Plexi path nor an installed channel name" >&2
    exit 1
  }
elif [[ -n "${PLEXI_BIN:-}" ]]; then
  CANDIDATE="$(resolve_channel_target "$PLEXI_BIN")" || {
    echo "error: PLEXI_BIN=$PLEXI_BIN is not executable" >&2
    exit 1
  }
elif [[ -x "$ROOT_DIR/target/release/plexi" ]]; then
  CANDIDATE="$ROOT_DIR/target/release/plexi"
elif [[ -x "$ROOT_DIR/target/debug/plexi" ]]; then
  CANDIDATE="$ROOT_DIR/target/debug/plexi"
elif command -v plexi >/dev/null 2>&1; then
  CANDIDATE="$(command -v plexi)"
else
  echo "error: plexi binary not found. Build with 'just build' or specify binary path." >&2
  exit 1
fi
PLEXI_BIN="$(resolve_real_plexi_binary "$CANDIDATE")"

echo "======================================================================"
echo " Plexi Agent Fleet Qualification (Stint 0965 / #2873: 1df1cddb)"
echo "======================================================================"
echo "Binary: $PLEXI_BIN"
echo "Date:   $(date -u +"%Y-%m-%dT%H:%M:%SZ")"
echo ""

echo "--- [1/5] Fleet Command Surface ---"
echo "1.1 plexi agent fleet --help:"
"$PLEXI_BIN" agent fleet --help || true
echo ""
echo "1.2 plexi agent fleet dismiss --help:"
"$PLEXI_BIN" agent fleet dismiss --help || true
echo ""
echo "1.3 plexi agent next --help:"
"$PLEXI_BIN" agent next --help || true
echo ""

echo "--- [2/5] Shipped Implementation References ---"
echo "Commit 1df1cddb (PR #2873) delivered:"
echo "- src/agent/fleet.rs: human names (clean_human_title, resolve_agent_label),"
echo "  age bucketing (format_age, DEFAULT_STALE_FAILURE_HOURS),"
echo "  failure reasons (resolve_failure_reason), next hints (resolve_next_hint)."
echo "- src/app/fleet.rs: dismiss_fleet_agent, agents_view with older bucket & stale filter."
echo "- src/cli/agent.rs: agent_fleet_cli, agent_fleet_dismiss_cli, agent_next_cli."
echo ""

echo "--- [3/5] Unit Test Qualification Gates ---"
echo "Executing focused unit tests for fleet and Pi lifecycle qualification:"
bash "$ROOT_DIR/scripts/cargo-with-lease.sh" cargo test --bin plexi fleet < /dev/null
echo "✓ Fleet unit suite PASSED."
echo ""

if [[ "$LIVE_MODE" == 1 ]]; then
  echo "--- [4/5] Live Channel Isolation Qualification ---"
  target_name="$(basename "${TARGET:-}")"
  case "$target_name" in
    plexi-pr-*) LIVE_CHAN="${target_name#plexi-}" ;;
    pr-*) LIVE_CHAN="$target_name" ;;
    *) LIVE_CHAN="qualif-fleet-$$" ;;
  esac
  unset PLEXI_SOCKET PLEXI_PANE_ID PLEXI_CONTEXT_ID PLEXI_CONTEXT_NAME PLEXI_CONTEXT_ROOT PLEXI_RUNNING
  WS_DIR="$(mktemp -d "${TMPDIR:-/tmp}/plexi-fleet-qualif-ws.XXXXXX")"
  SHIM_BIN="$WS_DIR/plexi-$LIVE_CHAN"
  ln "$PLEXI_BIN" "$SHIM_BIN" 2>/dev/null || cp "$PLEXI_BIN" "$SHIM_BIN"
  echo "Starting isolated host on channel $LIVE_CHAN (scratch retained at $WS_DIR)..."
  env PLEXI_CHANNEL="$LIVE_CHAN" \
    "$SHIM_BIN" host start --ephemeral --timeout-secs 15 --background --pane "cwd=$WS_DIR"

  STATUS_JSON="$(env PLEXI_CHANNEL="$LIVE_CHAN" "$SHIM_BIN" host status --json)"
  echo "Host status: $STATUS_JSON"
  OWNED_PID="$(printf '%s' "$STATUS_JSON" | python3 -c 'import json,sys; print(json.load(sys.stdin)["pid"])')"
  OWNED_SOCKET="$(printf '%s' "$STATUS_JSON" | python3 -c 'import json,sys; print(json.load(sys.stdin)["socket"])')"
  stop_live_host() {
    local current
    current="$(env PLEXI_CHANNEL="$LIVE_CHAN" "$SHIM_BIN" host status --json 2>/dev/null)" || return 0
    if printf '%s' "$current" | python3 -c 'import json,sys; s=json.load(sys.stdin); sys.exit(0 if str(s.get("pid")) == sys.argv[1] and s.get("socket") == sys.argv[2] else 1)' "$OWNED_PID" "$OWNED_SOCKET"; then
      env PLEXI_CHANNEL="$LIVE_CHAN" "$SHIM_BIN" host stop >/dev/null 2>&1 || true
    else
      echo "Not stopping host: channel pid or socket changed" >&2
    fi
  }
  trap stop_live_host EXIT

  echo "Creating real Pi-reported lifecycle panes..."
  printf -v WORKING_BODY '"%s" agent report --state working --agent pi --detail "qualifying working"; sleep 30' "$SHIM_BIN"
  printf -v WAITING_BODY '"%s" agent report --state blocked --agent pi --detail "Need a decision"; sleep 30' "$SHIM_BIN"
  printf -v DONE_BODY '"%s" agent report --state completed --agent pi --detail "qualification done"; sleep 30' "$SHIM_BIN"
  printf -v FAILED_BODY '"%s" agent report --state working --agent pi --detail "building2"; sleep 2; exit 3' "$SHIM_BIN"
  printf -v WORKING_CMD 'exec bash -c %q' "$WORKING_BODY"
  printf -v WAITING_CMD 'exec bash -c %q' "$WAITING_BODY"
  printf -v DONE_CMD 'exec bash -c %q' "$DONE_BODY"
  printf -v FAILED_CMD 'exec bash -c %q' "$FAILED_BODY"

  WORKING_PANE="$(env PLEXI_CHANNEL="$LIVE_CHAN" "$SHIM_BIN" pane new --no-focus --name "Pi qualification working" --cwd "$WS_DIR" "$WORKING_CMD")"
  WAITING_PANE="$(env PLEXI_CHANNEL="$LIVE_CHAN" "$SHIM_BIN" pane new --no-focus --name "Pi qualification waiting" --cwd "$WS_DIR" "$WAITING_CMD")"
  DONE_PANE="$(env PLEXI_CHANNEL="$LIVE_CHAN" "$SHIM_BIN" pane new --no-focus --name "Pi qualification done" --cwd "$WS_DIR" "$DONE_CMD")"
  FAILED_PANE="$(env PLEXI_CHANNEL="$LIVE_CHAN" "$SHIM_BIN" pane new --no-focus --name "Pi qualification failed" --cwd "$WS_DIR" "$FAILED_CMD")"
  echo "Panes: working=$WORKING_PANE waiting=$WAITING_PANE done=$DONE_PANE failed=$FAILED_PANE"
  sleep 3

  echo "Inspecting fleet and CLI status after the failed Pi process exits:"
  env PLEXI_CHANNEL="$LIVE_CHAN" "$SHIM_BIN" agent fleet
  FLEET_JSON="$(env PLEXI_CHANNEL="$LIVE_CHAN" "$SHIM_BIN" agent fleet --json)"
  STATUS_TEXT="$(env PLEXI_CHANNEL="$LIVE_CHAN" "$SHIM_BIN" agent status)"
  printf '%s\n' "$FLEET_JSON"
  printf '%s\n' "$STATUS_TEXT"
  if [[ "$FLEET_JSON" != *'exited (3)'* || "$STATUS_TEXT" != *'failed'* ]]; then
    echo "error: fleet/status did not preserve the failed Pi exit state" >&2
    exit 1
  fi

  echo "Dismissing the failed fleet row, closing its pane, and inspecting preserved history:"
  env PLEXI_CHANNEL="$LIVE_CHAN" "$SHIM_BIN" agent fleet dismiss "$FAILED_PANE"
  env PLEXI_CHANNEL="$LIVE_CHAN" "$SHIM_BIN" pane close "$FAILED_PANE"
  env PLEXI_CHANNEL="$LIVE_CHAN" "$SHIM_BIN" agent history

  echo "✓ Live Pi lifecycle trace captured. Scratch workspace and channel profile are retained for inspection."
else
  echo "--- [4/5] Live Qualification Instruction ---"
  echo "To drive a real headed host with interactive Pi agent, run:"
  echo "  ./scripts/qualify-agent-fleet.sh --live"
  echo "or observe in a running host pane with:"
  echo "  plexi agent fleet"
  echo "  plexi agent fleet --json"
  echo "  plexi agent next"
  echo "  plexi agent fleet dismiss --all-failed"
fi
echo ""

echo "--- [5/5] Qualification Summary ---"
if [[ "$LIVE_MODE" == 1 ]]; then
  echo "Status: Live CLI trace captured against shipped implementation (#2873 / 1df1cddb)."
  echo "Inspect the named Agents-pane and Command-Palette UI surfaces before recording human acceptance."
else
  echo "Status: Qualification not yet executed."
  echo "Focused unit coverage passed; run with --live and inspect the named UI surfaces before claiming human qualification."
fi
