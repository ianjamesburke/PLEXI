#!/usr/bin/env bash
# Installed-binary check that command view steers real leads (V1-11).
# Usage: scripts/command-view-steer-e2e.sh <PR>
# Depends on the agents API records from #2706 / #2715 / #2716, not the
# in-memory board on plexi-command-view-alpha (70139c1b).
# Human approval clicks are VERIFIED-VIA-BYPASS. W15 HUMAN_APPROVE is not used.
set -euo pipefail

if [[ -z "${DISPLAY:-}" ]] && command -v xvfb-run >/dev/null 2>&1; then
  exec xvfb-run -a "$0" "$@"
fi

PR="${1:?usage: scripts/command-view-steer-e2e.sh <PR>}"
BIN="plexi-pr-${PR}"
if ! command -v "$BIN" >/dev/null 2>&1; then
  echo "FAIL: $BIN is not on PATH. Run: just pr-install ${PR}"
  exit 1
fi
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN_PATH="$(command -v "$BIN")"
BIN_NAME="$(basename "$BIN_PATH")"
PROFILE="${HOME}/.${BIN_NAME}"
unset PLEXI_SOCKET
unset PLEXI_CHANNEL
unset OPENROUTER_API_KEY
unset PLEXI_PANE_ID

WORKDIR="$(mktemp -d)"
HOST_STARTED=0
MOCK_PID=""
SEND_PID=""

cleanup() {
  local status=$?
  if [[ -n "$SEND_PID" ]]; then
    kill "$SEND_PID" >/dev/null 2>&1 || true
  fi
  if [[ "$HOST_STARTED" == 1 ]]; then
    "$BIN" host stop >/dev/null 2>&1 || true
  fi
  if [[ -n "$MOCK_PID" ]]; then
    kill "$MOCK_PID" >/dev/null 2>&1 || true
  fi
  rm -rf "$WORKDIR"
  exit "$status"
}
trap cleanup EXIT

cd "$WORKDIR"
python3 "$ROOT/scripts/e2e/lead_mock.py" >"$WORKDIR/mock.port" &
MOCK_PID=$!
for _ in 1 2 3 4 5 6 7 8 9 10; do
  [[ -s "$WORKDIR/mock.port" ]] && break
  sleep 0.1
done
PORT="$(head -n 1 "$WORKDIR/mock.port")"
export PLEXI_OPENROUTER_BASE_URL="http://127.0.0.1:${PORT}"
export PLEXI_LEAD_MODEL="mock/lead"
echo "workspace: $WORKDIR"
echo "binary: $BIN_PATH"
echo "mock: $PLEXI_OPENROUTER_BASE_URL"
echo "approvals: VERIFIED-VIA-BYPASS (grant is pre-recorded; no human click)"
echo "command-view board dependency: plexi-command-view-alpha @ 70139c1b is not the data source"

"$BIN" workspace init
if ! "$BIN" host start --background --ephemeral --timeout-secs 45; then
  echo "FAIL: host start"
  exit 1
fi
HOST_STARTED=1

cli() { "$BIN" "$@"; }

echo "STEP create two leads"
A="$(cli agent head create lead-a --display-name 'Lead A' --grant assistant.turn=allow --json)"
B="$(cli agent head create lead-b --display-name 'Lead B' --grant assistant.turn=allow --grant lead.step=allow --json)"
python3 -c 'import json,sys; assert json.loads(sys.argv[1]).get("ok") is True' "$A"
python3 -c 'import json,sys; assert json.loads(sys.argv[1]).get("ok") is True' "$B"

echo "STEP ungranted write becomes a pending approval"
set +e
PENDING="$(cli command-view send lead-a "write-file")"
PENDING_CODE=$?
set -e
printf '%s\n' "$PENDING" >"$WORKDIR/pending.json"
python3 -c 'import json,sys; body=json.loads(sys.argv[1]); assert body.get("state")=="permission_required", body' "$PENDING"
echo "pending send exited $PENDING_CODE"

echo "STEP command view pane shows both leads and the pending approval"
OPEN="$(cli command-view open)"
python3 -c 'import json,sys; assert json.loads(sys.argv[1]).get("ok") is True' "$OPEN"
PANE="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["pane_id"])' "$OPEN")"
STATE="$(cli pane state "$PANE")"
printf '%s\n' "$STATE" >"$WORKDIR/pane-state.json"
python3 - "$WORKDIR/pane-state.json" <<'PY'
import json, sys
body = json.load(open(sys.argv[1]))
app = body.get("app_state") or {}
ids = {item.get("id") for item in app.get("heads", [])}
assert {"lead-a", "lead-b"} <= ids, body
needs = app.get("needs_you") or []
assert needs, body
print("pane state heads", sorted(ids), "needs_you", len(needs))
PY
SHOT="$WORKDIR/command-view.png"
if cli host screenshot --pane "$PANE" --output "$SHOT"; then
  test -s "$SHOT"
  mkdir -p /tmp/plexi-e2e
  cp "$SHOT" /tmp/plexi-e2e/command-view-steer.png
  echo "screenshot $SHOT"
else
  echo "FAIL: host screenshot"
  exit 1
fi

echo "STEP command-view send produces a real turn"
STATUS="$(cli command-view send lead-a "status?")"
python3 -c 'import json,sys; body=json.loads(sys.argv[1]); assert body.get("state")=="succeeded", body; assert "idle" in (body.get("reply") or ""), body' "$STATUS"
CONV="$(cli agent conversation --head lead-a --json)"
python3 -c 'import json,sys; text=json.dumps(json.loads(sys.argv[1])); assert "idle" in text, text' "$CONV"
echo "lead A conversation contains the status reply"

echo "STEP cancel a live run"
cli command-view send lead-b "long-task" >"$WORKDIR/long.out" 2>"$WORKDIR/long.err" &
SEND_PID=$!
RUN=""
for _ in $(seq 1 40); do
  VIEW="$(cli command-view --json)"
  RUN="$(python3 - "$VIEW" <<'PY'
import json, sys
body = json.loads(sys.argv[1])
for run in body.get("runs", []):
    if run.get("head_id") == "lead-b" and run.get("active") is True:
        print(run["id"])
        break
PY
)"
  if [[ -n "$RUN" ]]; then
    break
  fi
  sleep 0.1
done
if [[ -z "$RUN" ]]; then
  echo "FAIL: no active run for lead-b"
  cat "$WORKDIR/long.out" "$WORKDIR/long.err" || true
  exit 1
fi
echo "cancelling $RUN"
cli command-view cancel "$RUN"
wait "$SEND_PID" || true
SEND_PID=""
sleep 1
AUDIT="${PROFILE}/permission-audit.jsonl"
STEPS=0
if [[ -f "$AUDIT" ]]; then
  STEPS="$(python3 -c 'import pathlib,sys; print(pathlib.Path(sys.argv[1]).read_text().count("\"operation_id\":\"lead.step\""))' "$AUDIT")"
fi
sleep 2
LATER="$STEPS"
if [[ -f "$AUDIT" ]]; then
  LATER="$(python3 -c 'import pathlib,sys; print(pathlib.Path(sys.argv[1]).read_text().count("\"operation_id\":\"lead.step\""))' "$AUDIT")"
fi
python3 -c 'import sys; a,b=int(sys.argv[1]),int(sys.argv[2]); assert a==b, (a,b)' "$STEPS" "$LATER"
CONV_B="$(cli agent conversation --head lead-b --json)"
python3 -c 'import json,sys; text=json.dumps(json.loads(sys.argv[1])); assert "long task done" not in text, text' "$CONV_B"
echo "cancel stopped run $RUN; lead.step rows stayed $STEPS"

echo "STEP restart shows the same leads and runs"
BEFORE="$(cli command-view --json)"
"$BIN" host stop >/dev/null 2>&1 || true
HOST_STARTED=0
if ! "$BIN" host start --background --ephemeral --timeout-secs 45; then
  echo "FAIL: host restart"
  exit 1
fi
HOST_STARTED=1
AFTER="$(cli command-view --json)"
python3 - "$BEFORE" "$AFTER" <<'PY'
import json, sys
before = json.loads(sys.argv[1])
after = json.loads(sys.argv[2])
b_heads = {item["id"] for item in before["heads"]}
a_heads = {item["id"] for item in after["heads"]}
assert {"lead-a", "lead-b"} <= b_heads <= a_heads or b_heads == a_heads, (b_heads, a_heads)
b_runs = {item["id"] for item in before["runs"]}
a_runs = {item["id"] for item in after["runs"]}
assert b_runs <= a_runs, (b_runs, a_runs)
print("restart kept", len(a_heads), "heads and", len(a_runs), "runs")
PY

echo "STEP resolve and allow from an agent pane are refused"
BEFORE="$(cli pane list)"
cli pane new "$BIN_PATH command-view resolve pending_example; $BIN_PATH command-view allow --tool assistant.turn; sleep 20" --no-focus >/dev/null
FOUND=0
CAPTURE=""
for _ in $(seq 1 40); do
  LIST="$(cli pane list)"
  NEW_IDS="$(python3 - "$BEFORE" "$LIST" <<'PY'
import json, sys
before = {row.get("id") for row in json.loads(sys.argv[1])}
rows = json.loads(sys.argv[2])
for row in rows:
    if row.get("id") not in before:
        print(row.get("id"))
PY
)"
  CAPTURE=""
  for pane_id in $NEW_IDS; do
    CAPTURE+=$'\n'"$(cli pane capture "$pane_id" --plain 2>/dev/null || true)"
  done
  if printf '%s\n' "$CAPTURE" | python3 -c 'import sys; raise SystemExit(0 if sys.stdin.read().count("agent_cannot_approve") >= 2 else 1)'; then
    FOUND=1
    break
  fi
  sleep 0.5
done
if [[ "$FOUND" != 1 ]]; then
  echo "FAIL: agent pane did not refuse resolve and allow"
  printf '%s\n' "$CAPTURE"
  exit 1
fi
echo "agent pane refused resolve and allow"

echo "PASS command view steers real leads"
