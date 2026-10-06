#!/usr/bin/env bash
# Installed-binary check for a second real lead (V1-10 steps 1, 2, and 7).
# Usage: scripts/multi-lead-e2e.sh <PR>
# The binary is plexi-pr-<PR>. The model is scripts/e2e/lead_mock.py.
# Human approval clicks are VERIFIED-VIA-BYPASS: the ungranted tool stops at
# permission_required and does not run. W15 HUMAN_APPROVE is not invoked.
set -euo pipefail

if [[ -z "${DISPLAY:-}" ]] && command -v xvfb-run >/dev/null 2>&1; then
  exec xvfb-run -a "$0" "$@"
fi

PR="${1:?usage: scripts/multi-lead-e2e.sh <PR>}"
BIN="plexi-pr-${PR}"
if ! command -v "$BIN" >/dev/null 2>&1; then
  echo "FAIL: $BIN is not on PATH. Run: just pr-install ${PR}"
  exit 1
fi
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN_PATH="$(command -v "$BIN")"
unset PLEXI_SOCKET
unset PLEXI_CHANNEL
unset OPENROUTER_API_KEY

WORKDIR="$(mktemp -d)"
HOST_STARTED=0
MOCK_PID=""

cleanup() {
  local status=$?
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
  if [[ -s "$WORKDIR/mock.port" ]]; then
    break
  fi
  sleep 0.1
done
PORT="$(head -n 1 "$WORKDIR/mock.port")"
if [[ -z "$PORT" ]]; then
  echo "FAIL: mock model did not print a port"
  exit 1
fi
export PLEXI_OPENROUTER_BASE_URL="http://127.0.0.1:${PORT}"
export PLEXI_LEAD_MODEL="mock/lead"
echo "workspace: $WORKDIR"
echo "binary: $BIN_PATH"
echo "mock: $PLEXI_OPENROUTER_BASE_URL"
echo "approvals: VERIFIED-VIA-BYPASS (no human click; ungranted tool must not run)"

"$BIN" workspace init
if ! "$BIN" host start --background --ephemeral --timeout-secs 45; then
  echo "FAIL: host start"
  exit 1
fi
HOST_STARTED=1

must_ok() {
  local label="$1"
  local body="$2"
  if ! python3 -c 'import json,sys; body=json.loads(sys.argv[1]); assert body.get("ok") is True, body' "$body"; then
    echo "FAIL: $label"
    echo "$body"
    exit 1
  fi
}

cli() {
  "$BIN" "$@"
}

echo "STEP create leads"
A="$(cli agent head create lead-a --display-name 'Lead A' --grant assistant.turn=allow --grant leads.conversation.read=allow --json)"
must_ok "create lead-a" "$A"
B="$(cli agent head create lead-b --display-name 'Lead B' --grant assistant.turn=allow --json)"
must_ok "create lead-b" "$B"

echo "STEP open both assistant panes"
OPEN_A="$(cli assistant open --head lead-a)"
OPEN_B="$(cli assistant open --head lead-b)"
must_ok "open lead-a" "$OPEN_A"
must_ok "open lead-b" "$OPEN_B"
python3 - "$OPEN_A" "$OPEN_B" <<'PY'
import json, sys
a = json.loads(sys.argv[1])
b = json.loads(sys.argv[2])
assert a["pane_id"] != b["pane_id"], (a, b)
print("two assistant panes", a["pane_id"], b["pane_id"])
PY

echo "STEP pane list shows both heads"
LIST="$(cli pane list)"
python3 - "$LIST" <<'PY'
import json, sys
rows = json.loads(sys.argv[1])
titles = [row.get("title") for row in rows if row.get("type") == "app"]
assert "Lead A" in titles, titles
assert "Lead B" in titles, titles
print("pane titles", titles)
PY

echo "STEP separate turns"
SEND_A="$(cli assistant send --head lead-a --text 'remember 7')"
SEND_B="$(cli assistant send --head lead-b --text 'what number?')"
python3 - "$SEND_A" "$SEND_B" <<'PY'
import json, sys
a = json.loads(sys.argv[1])
b = json.loads(sys.argv[2])
assert a.get("state") == "succeeded", a
assert b.get("state") == "succeeded", b
assert a.get("reply") == "Noted 7", a
assert b.get("reply") == "I do not know a number", b
assert a.get("head") == "lead-a" and b.get("head") == "lead-b"
print("turns", a["reply"], "|", b["reply"])
PY

echo "STEP transcripts stay separate"
CONV_A="$(cli agent conversation --head lead-a --json)"
CONV_B="$(cli agent conversation --head lead-b --json)"
python3 - "$CONV_A" "$CONV_B" <<'PY'
import json, sys
a = json.dumps(json.loads(sys.argv[1]))
b = json.dumps(json.loads(sys.argv[2]))
assert "remember 7" in a and "Noted 7" in a, a
assert "remember 7" not in b and "Noted 7" not in b, b
assert "I do not know a number" in b, b
print("transcripts are separate files")
PY

echo "STEP command view lists both heads"
VIEW="$(cli command-view --json)"
python3 - "$VIEW" <<'PY'
import json, sys
body = json.loads(sys.argv[1])
ids = [head.get("id") for head in body.get("heads", [])]
assert "lead-a" in ids and "lead-b" in ids, body
print("command view heads", ids)
PY
OPEN_VIEW="$(cli command-view open)"
must_ok "open command view" "$OPEN_VIEW"
VIEW_ID="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["pane_id"])' "$OPEN_VIEW")"
STATE="$(cli pane state "$VIEW_ID")"
python3 - "$STATE" <<'PY'
import json, sys
body = json.loads(sys.argv[1])
app_state = body.get("app_state") or {}
ids = [head.get("id") for head in app_state.get("heads", [])]
assert "lead-a" in ids and "lead-b" in ids, body
print("command view pane state", ids)
PY

echo "STEP lead A cannot read lead B"
DENIED="$(cli agent conversation --head lead-b --as lead-a --json || true)"
python3 - "$DENIED" <<'PY'
import json, sys
body = json.loads(sys.argv[1])
assert body.get("ok") is False, body
assert body.get("error_code") == "permission_denied", body
print("cross-lead read refused")
PY
READ="$(cli assistant send --head lead-a --text 'read-other lead-b')"
python3 - "$READ" <<'PY'
import json, sys
body = json.loads(sys.argv[1])
assert body.get("state") == "succeeded", body
assert "could not read" in body.get("reply", ""), body
print("tool read refused", body.get("reply"))
PY
CONV_A2="$(cli agent conversation --head lead-a --json)"
python3 - "$CONV_A2" <<'PY'
import json, sys
text = json.dumps(json.loads(sys.argv[1]))
assert "cannot read another lead" in text, text
assert "I do not know a number" not in text, text
print("lead A transcript has no lead B messages")
PY

echo "STEP ungranted tool does not run (VERIFIED-VIA-BYPASS)"
WRITE="$(cli assistant send --head lead-a --text 'write-file out.txt' || true)"
python3 - "$WRITE" "$WORKDIR" <<'PY'
import json, sys
from pathlib import Path
body = json.loads(sys.argv[1])
assert body.get("state") == "permission_required", body
assert not (Path(sys.argv[2]) / "out.txt").exists()
print("VERIFIED-VIA-BYPASS permission_required; file was not written")
PY

echo "PASS multi-lead e2e"
