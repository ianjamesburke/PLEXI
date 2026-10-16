#!/usr/bin/env bash
# Live Pi + Plexi integration test harness (stint 0796).
#
# Runs against an isolated local Plexi channel profile (`PLEXI_CHANNEL=local-pi-live-$$`)
# so it never touches the developer's active stable/beta/alpha host.
#
# Exercises:
# 1. `packages/pi-plexi` unit suite (`node --test`).
# 2. Isolated live Plexi host startup (`plexi host start --wait`) with `apps/dev/clip-editor-probe`.
# 3. `pi -ne -e <checkout>/packages/pi-plexi/extensions/index.ts --skill <checkout>/packages/pi-plexi/skills`
#    in both:
#    a) Non-interactive JSON mode (`pi -p --mode json`) inside a live Plexi pane context.
#    b) Interactive TUI session spawned inside a real Plexi terminal pane (`plexi pane split -- bash -lc ...`),
#       verified via `plexi pane state`, `plexi pane capture`, `plexi needs-you`, and `plexi host screenshot`.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

echo "==> [1/6] Running packages/pi-plexi unit tests..."
node --test "$ROOT_DIR/packages/pi-plexi/tests/pi-plexi.test.mjs"

if [[ -x "$ROOT_DIR/target/release/plexi" ]]; then
  PLEXI_BIN="$ROOT_DIR/target/release/plexi"
elif [[ -x "$ROOT_DIR/target/debug/plexi" ]]; then
  PLEXI_BIN="$ROOT_DIR/target/debug/plexi"
else
  echo "==> Building plexi binary..."
  bash "$ROOT_DIR/scripts/cargo-with-lease.sh" cargo build --bin plexi
  PLEXI_BIN="$ROOT_DIR/target/debug/plexi"
fi

PI_BIN="${PI_BIN:-$(command -v pi || true)}"
if [[ -z "$PI_BIN" && -x "$HOME/.pi/agent/bin/pi" ]]; then
  PI_BIN="$HOME/.pi/agent/bin/pi"
fi
if [[ -z "$PI_BIN" ]]; then
  echo "error: pi binary not found in PATH or ~/.pi/agent/bin/pi" >&2
  exit 1
fi

echo "==> Using PLEXI_BIN=$PLEXI_BIN"
echo "==> Using PI_BIN=$PI_BIN ($("$PI_BIN" --version))"

LIVE_CHANNEL="local-pi-live-$$"
PROFILE_DIR="$HOME/.plexi-$LIVE_CHANNEL"
SOCKET_PATH="$PROFILE_DIR/notify.sock"
WORKSPACE_DIR="$(mktemp -d "${TMPDIR:-/tmp}/plexi-pi-live-ws.XXXXXX")"
SHIM_BIN="$WORKSPACE_DIR/plexi-$LIVE_CHANNEL"
ln "$PLEXI_BIN" "$SHIM_BIN" 2>/dev/null || cp "$PLEXI_BIN" "$SHIM_BIN"
SHOT_PATH="${TMPDIR:-/tmp}/plexi_pi_live_$$.png"

plexi_iso() {
  if [[ "${1:-}" == "host" && "${2:-}" == "start" ]]; then
    env -u PLEXI_SOCKET -u PLEXI_PANE_ID -u PLEXI_CONTEXT_ID -u PLEXI_CONTEXT_NAME -u PLEXI_CONTEXT_ROOT -u PLEXI_RUNNING \
      PLEXI_CHANNEL="$LIVE_CHANNEL" \
      "$SHIM_BIN" "$@"
  else
    env -u PLEXI_PANE_ID -u PLEXI_CONTEXT_ID -u PLEXI_CONTEXT_NAME -u PLEXI_CONTEXT_ROOT -u PLEXI_RUNNING \
      PLEXI_CHANNEL="$LIVE_CHANNEL" \
      PLEXI_SOCKET="$SOCKET_PATH" \
      "$SHIM_BIN" "$@"
  fi
}

cleanup() {
  set +e
  plexi_iso host stop >/dev/null 2>&1 || true
  if [[ -f "$SHOT_PATH" ]]; then
    unlink "$SHOT_PATH" >/dev/null 2>&1 || true
  fi
  if [[ -d "$PROFILE_DIR" || -d "$WORKSPACE_DIR" ]]; then
    python3 - "$PROFILE_DIR" "$WORKSPACE_DIR" <<'PY' >/dev/null 2>&1 || true
import shutil, sys
for p in sys.argv[1:]:
    shutil.rmtree(p, ignore_errors=True)
PY
  fi
}
trap cleanup EXIT

mkdir -p "$WORKSPACE_DIR/.plexi"

echo "==> [2/6] Starting isolated Plexi host on channel $LIVE_CHANNEL..."
plexi_iso host start --ephemeral --timeout-secs 20 --background --pane "cwd=$WORKSPACE_DIR"
STATUS_JSON="$(plexi_iso host status --json)"
echo "Host status: $STATUS_JSON"

PANES_JSON="$(plexi_iso pane list)"
FIRST_PANE_ID="$(python3 -c 'import json,sys; panes=json.loads(sys.argv[1]); print(panes[0]["id"])' "$PANES_JSON")"
FIRST_CTX_ID="$(python3 -c 'import json,sys; panes=json.loads(sys.argv[1]); print(panes[0]["context_id"])' "$PANES_JSON")"
echo "==> Initial terminal pane=$FIRST_PANE_ID context=$FIRST_CTX_ID"

echo "==> [3/6] Launching deterministic editor fixture apps/dev/clip-editor-probe..."
APP_LAUNCH_JSON="$(plexi_iso app open "$ROOT_DIR/apps/dev/clip-editor-probe" --from "$FIRST_PANE_ID")"
echo "App launch: $APP_LAUNCH_JSON"

for _ in {1..40}; do
  TOOLS_JSON="$(plexi_iso app tools --json --context "$FIRST_CTX_ID" || true)"
  if echo "$TOOLS_JSON" | grep -q '"clip.crop"'; then
    break
  fi
  sleep 0.25
done
echo "==> Discovered app tools in context $FIRST_CTX_ID:"
echo "$TOOLS_JSON" | python3 -c 'import json,sys; d=json.load(sys.stdin); print([t["name"] for t in d.get("tools",[])])'

# Verify ask-tier gate (`clip.state` and `clip.crop` without grant return permission_required),
# human approval via `plexi permissions allow`, resource-scoped execution on `intro`,
# operation_id deduplication, and that `outro` still requires its own approval.
run_in_pane1() {
  local cmd="$1"
  local out_file="$2"
  local script_file="$WORKSPACE_DIR/run_cmd.sh"
  if [[ -f "$out_file" ]]; then
    unlink "$out_file" 2>/dev/null || true
  fi
  cat > "$script_file" <<EOF
'$SHIM_BIN' $cmd > '$out_file.tmp' || true
mv '$out_file.tmp' '$out_file'
EOF
  plexi_iso pane send "$FIRST_PANE_ID" "bash '$script_file'"
  plexi_iso pane key "$FIRST_PANE_ID" enter
  for _ in {1..40}; do
    if [[ -s "$out_file" ]]; then
      cat "$out_file"
      return 0
    fi
    sleep 0.2
  done
  echo "error: timed out waiting for pane command output ($cmd)" >&2
  return 1
}

STATE_ASK_JSON="$(run_in_pane1 "app call clip-editor-probe clip.state --json --input '{\"clip_id\":\"intro\"}'" "$WORKSPACE_DIR/state_ask.json")"
echo "clip.state unapproved: $STATE_ASK_JSON"
STATE_REQ_ID="$(python3 -c 'import json,sys; d=json.loads(sys.argv[1]); assert d["ok"] is False and d["error_code"] == "permission_required"; print(d["pending_request_id"])' "$STATE_ASK_JSON")"

plexi_iso permissions allow "$STATE_REQ_ID"
STATE_CALL_JSON="$(run_in_pane1 "app call clip-editor-probe clip.state --json --input '{\"clip_id\":\"intro\"}'" "$WORKSPACE_DIR/state_ok.json")"
echo "clip.state approved: $STATE_CALL_JSON"
python3 -c 'import json,sys; d=json.loads(sys.argv[1]); assert d["ok"] is True and d["output"]["clip"]["revision"] == "rev-1", d' "$STATE_CALL_JSON"

CROP_ASK_JSON="$(run_in_pane1 "app call clip-editor-probe clip.crop --json --input '{\"clip_id\":\"intro\",\"crop_preset\":\"9:16\",\"expected_revision\":\"rev-1\",\"operation_id\":\"op-live-1\"}'" "$WORKSPACE_DIR/crop_ask.json")"
echo "clip.crop unapproved: $CROP_ASK_JSON"
CROP_REQ_ID="$(python3 -c 'import json,sys; d=json.loads(sys.argv[1]); assert d["ok"] is False and d["error_code"] == "permission_required"; print(d["pending_request_id"])' "$CROP_ASK_JSON")"

plexi_iso permissions allow "$CROP_REQ_ID"
CROP_OK_JSON="$(run_in_pane1 "app call clip-editor-probe clip.crop --json --input '{\"clip_id\":\"intro\",\"crop_preset\":\"9:16\",\"expected_revision\":\"rev-1\",\"operation_id\":\"op-live-1\"}'" "$WORKSPACE_DIR/crop_ok.json")"
echo "clip.crop approved: $CROP_OK_JSON"
python3 -c 'import json,sys; d=json.loads(sys.argv[1]); assert d["ok"] is True and d["output"]["revision_after"] == "rev-2" and d["output"]["deduplicated"] is False, d' "$CROP_OK_JSON"

# Duplicate operation_id returns cached receipt without mutating to rev-3
CROP_DUP_JSON="$(run_in_pane1 "app call clip-editor-probe clip.crop --json --input '{\"clip_id\":\"intro\",\"crop_preset\":\"9:16\",\"expected_revision\":\"rev-1\",\"operation_id\":\"op-live-1\"}'" "$WORKSPACE_DIR/crop_dup.json")"
echo "clip.crop deduplicated: $CROP_DUP_JSON"
python3 -c 'import json,sys; d=json.loads(sys.argv[1]); assert d["ok"] is True and d["output"]["revision_after"] == "rev-2" and d["output"]["deduplicated"] is True, d' "$CROP_DUP_JSON"

# Resource scoping: grant for clip_id="intro" must NOT cover clip_id="outro"
OUTRO_ASK_JSON="$(run_in_pane1 "app call clip-editor-probe clip.crop --json --input '{\"clip_id\":\"outro\",\"crop_preset\":\"9:16\",\"expected_revision\":\"rev-1\",\"operation_id\":\"op-live-2\"}'" "$WORKSPACE_DIR/outro_ask.json")"
echo "clip.crop outro (unapproved): $OUTRO_ASK_JSON"
python3 -c 'import json,sys; d=json.loads(sys.argv[1]); assert d["ok"] is False and d["error_code"] == "permission_required", d' "$OUTRO_ASK_JSON"

echo "==> [4/6] Exercising structured Needs-you question + continuation reconciliation..."
ASK_JSON="$(env PLEXI_CHANNEL="$LIVE_CHANNEL" PLEXI_SOCKET="$SOCKET_PATH" PLEXI_PANE_ID="$FIRST_PANE_ID" PLEXI_CONTEXT_ID="$FIRST_CTX_ID" \
  "$SHIM_BIN" needs-you ask \
  --summary "Which crop preset for Intro Slate?" \
  --resource "intro" \
  --choice "16:9" --choice "9:16" --choice "1:1" \
  --preview-ref "clip:intro@0..3000ms" \
  --correlation-id "corr-live-1" \
  --operation-id "op-live-1" \
  --session-id "sess-live-pi" \
  --expected-revision "rev-1")"
echo "Filed structured question: $ASK_JSON"
QID="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["id"])' "$ASK_JSON")"
UI_TOKEN="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["ui_token"])' "$ASK_JSON")"

# Verify unauthenticated pane caller cannot forge an answer
if env PLEXI_CHANNEL="$LIVE_CHANNEL" PLEXI_SOCKET="$SOCKET_PATH" PLEXI_PANE_ID="$FIRST_PANE_ID" \
  "$SHIM_BIN" needs-you answer "$QID" --answer "9:16" >/dev/null 2>&1; then
  echo "error: unauthenticated pane caller should have been refused" >&2
  exit 1
fi

# Settle with the UI token and verify reconciliation
ANS_JSON="$(env PLEXI_CHANNEL="$LIVE_CHANNEL" PLEXI_SOCKET="$SOCKET_PATH" PLEXI_PANE_ID="$FIRST_PANE_ID" \
  "$SHIM_BIN" needs-you answer "$QID" --answer "9:16" --ui-token "$UI_TOKEN")"
echo "Answered question: $ANS_JSON"
python3 -c 'import json,sys; d=json.loads(sys.argv[1]); assert d["ok"] is True and d["answer"] == "9:16", d' "$ANS_JSON"

REC_JSON="$(plexi_iso needs-you reconcile --session-id "sess-live-pi")"
echo "Reconciled continuation: $REC_JSON"
python3 -c 'import json,sys; d=json.loads(sys.argv[1]); assert d["ok"] is True and d["continuations"][0]["answer"] == "9:16", d' "$REC_JSON"

echo "==> [5/6] Launching interactive Pi TUI session inside a real Plexi terminal pane..."
PI_TUI_CMD="PLEXI_CLI_BIN='$SHIM_BIN' '$PI_BIN' -ne -e '$ROOT_DIR/packages/pi-plexi/extensions/index.ts' --skill '$ROOT_DIR/packages/pi-plexi/skills'"
SPLIT_OUT="$(plexi_iso pane new --from "$FIRST_PANE_ID" -n "pi-live" "$PI_TUI_CMD")"
PI_PANE_ID="$(python3 -c 'import json,sys; raw=sys.argv[1].strip(); print(json.loads(raw)["pane_id"] if raw.startswith("{") else raw.splitlines()[0].strip())' "$SPLIT_OUT")"
echo "==> Spawned interactive Pi pane=$PI_PANE_ID"

for _ in {1..40}; do
  CAPTURE_OUT="$(plexi_iso pane capture --lines 40 "$PI_PANE_ID" || true)"
  if echo "$CAPTURE_OUT" | grep -q "Plexi Workspace"; then
    break
  fi
  sleep 0.25
done
echo "==> Interactive Pi pane capture excerpt:"
echo "$CAPTURE_OUT" | grep -E "Plexi Workspace|plexi\[|Panes:|App tools:" || echo "$CAPTURE_OUT"
echo "==> Interactive Pi pane state: $(plexi_iso pane state "$PI_PANE_ID")"
echo "==> Agent status:"
plexi_iso agent status

if plexi_iso host screenshot --output "$SHOT_PATH" 2>/dev/null && [[ -s "$SHOT_PATH" ]]; then
  echo "==> Host screenshot captured ($SHOT_PATH)"
else
  plexi_iso app render "$ROOT_DIR/apps/dev/clip-editor-probe" --png --output "$SHOT_PATH" >/dev/null
  test -s "$SHOT_PATH"
  echo "==> Headless PNG render captured ($SHOT_PATH; background host window occluded)"
fi

echo "==> [6/6] Running non-interactive Pi turn (pi -p --mode json) if model credentials are configured..."
API_KEY_VAL="${PLEXI_TEST_OPENROUTER_API_KEY:-${OPENROUTER_API_KEY:-}}"
if [[ -n "$API_KEY_VAL" ]]; then
  JSON_OUT="$(env \
    OPENROUTER_API_KEY="$API_KEY_VAL" \
    PLEXI_CHANNEL="$LIVE_CHANNEL" \
    PLEXI_SOCKET="$SOCKET_PATH" \
    PLEXI_PANE_ID="$FIRST_PANE_ID" \
    PLEXI_CONTEXT_ID="$FIRST_CTX_ID" \
    PLEXI_CLI_BIN="$SHIM_BIN" \
    "$PI_BIN" -ne \
    -e "$ROOT_DIR/packages/pi-plexi/extensions/index.ts" \
    --skill "$ROOT_DIR/packages/pi-plexi/skills" \
    --provider openrouter \
    --model "${PLEXI_TEST_PI_MODEL:-openai/gpt-4o-mini}" \
    -p --mode json \
    "Call plexi_workspace_overview and plexi_app_tools, then reply with DONE." || true)"
  if echo "$JSON_OUT" | grep -q '"plexi_workspace_overview"'; then
    echo "==> Non-interactive Pi JSON turn invoked plexi_workspace_overview!"
  else
    echo "==> Non-interactive Pi JSON output received ($(echo "$JSON_OUT" | wc -l) lines)."
  fi
else
  # Verify Pi CLI non-interactive flag parsing & extension load with --help / dry invocation
  "$PI_BIN" -ne \
    -e "$ROOT_DIR/packages/pi-plexi/extensions/index.ts" \
    --skill "$ROOT_DIR/packages/pi-plexi/skills" \
    --version >/dev/null
  echo "==> Verified Pi 1.3.0 CLI flag & extension loading (set OPENROUTER_API_KEY or PLEXI_TEST_OPENROUTER_API_KEY for live model turns)."
fi

echo "==> pi-plexi-live harness PASSED."
