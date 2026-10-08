#!/usr/bin/env bash
# Installed-binary check that command view steers real leads (V1-11).
# Usage: scripts/command-view-steer-e2e.sh <PR>
# Depends on the agents API records from #2706 / #2715 / #2716, not the
# in-memory board on plexi-command-view-alpha (70139c1b).
# The write is left pending so the command pane can show the warning-color
# waiting line, then HUMAN_APPROVE clicks Allow once. The script never calls
# assistant permission resolve or command-view resolve.
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
FOLLOW_PID=""

cleanup() {
  local status=$?
  if [[ -n "$FOLLOW_PID" ]]; then
    kill "$FOLLOW_PID" >/dev/null 2>&1 || true
    wait "$FOLLOW_PID" 2>/dev/null || true
  fi
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
echo "approvals: HUMAN_APPROVE (real pointer click; no CLI resolve)"
echo "command-view board dependency: plexi-command-view-alpha @ 70139c1b is not the data source"

"$BIN" workspace init
if ! "$BIN" host start --background --ephemeral --timeout-secs 45; then
  echo "FAIL: host start"
  exit 1
fi
HOST_STARTED=1

cli() { "$BIN" "$@"; }

open_command_pane() {
  local id="$1"
  local before list pane
  before="$(cli pane list)"
  cli app open "$id" >/dev/null
  pane=""
  for _ in $(seq 1 40); do
    list="$(cli pane list)"
    pane="$(python3 - "$before" "$list" <<'PY'
import json, sys
before = {row.get("id") for row in json.loads(sys.argv[1])}
for row in json.loads(sys.argv[2]):
    if row.get("id") in before:
        continue
    if row.get("manifest_id") in ("command-view", "command") or row.get("title") == "Command":
        print(row.get("id", ""))
        break
PY
)"
    if [[ -n "$pane" ]]; then
      printf '%s\n' "$pane"
      return 0
    fi
    sleep 0.25
  done
  echo "FAIL: app open $id did not open a Command pane" >&2
  cli pane list >&2 || true
  return 1
}

wait_pane_gone() {
  local id="$1"
  for _ in $(seq 1 40); do
    if python3 - "$id" "$(cli pane list)" <<'PY'
import json, sys
want = int(sys.argv[1])
ids = {row.get("id") for row in json.loads(sys.argv[2])}
raise SystemExit(0 if want not in ids else 1)
PY
    then
      return 0
    fi
    sleep 0.25
  done
  echo "FAIL: pane $id did not close" >&2
  return 1
}

echo "STEP app open command and command-view both open the Command pane"
PANE_CMD="$(open_command_pane command)"
echo "app open command -> pane $PANE_CMD"
cli pane close "$PANE_CMD" >/dev/null
wait_pane_gone "$PANE_CMD"
PANE_VIEW="$(open_command_pane command-view)"
echo "app open command-view -> pane $PANE_VIEW"
cli pane close "$PANE_VIEW" >/dev/null
wait_pane_gone "$PANE_VIEW"

echo "STEP ungranted send exits 2 and creates no lead"
set +e
MISSING="$(cli command-view send not-a-lead "hello")"
MISSING_CODE=$?
set -e
printf '%s\n' "$MISSING" >"$WORKDIR/missing.json"
python3 -c 'import json,sys; body=json.loads(sys.argv[1]); assert body.get("error_code")=="permission_required", body; assert body.get("pending_request_id"), body' "$MISSING"
test "$MISSING_CODE" -eq 2
test ! -e "$WORKDIR/.plexi/agents/not-a-lead"
echo "ungranted send exited $MISSING_CODE and created no lead"
# shellcheck disable=SC1091
source "$ROOT/scripts/e2e/human.sh"
export BIN
MISSING_PENDING="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("pending_request_id") or "")' "$WORKDIR/missing.json")"
if [[ -z "$MISSING_PENDING" ]]; then
  echo "FAIL: ungranted send did not name a pending id"
  exit 1
fi
echo "HUMAN_DENY pending $MISSING_PENDING so the later write owns the banner"
if ! HUMAN_DENY "$MISSING_PENDING"; then
  "$BIN" host screenshot --output "$WORKDIR/deny-miss.png" >/dev/null 2>&1 || true
  echo "FAIL: HUMAN_DENY did not clear $MISSING_PENDING"
  exit 1
fi

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

echo "STEP command-view send produces a real turn"
STATUS="$(cli command-view send lead-a "status?")"
python3 -c 'import json,sys; body=json.loads(sys.argv[1]); assert body.get("state")=="succeeded", body; assert "idle" in (body.get("reply") or ""), body' "$STATUS"
CONV="$(cli agent conversation --head lead-a --json)"
python3 -c 'import json,sys; text=json.dumps(json.loads(sys.argv[1])); assert "idle" in text, text' "$CONV"
echo "lead A conversation contains the status reply"

printf '%s\n' '{"text":"remember 4"}' >"$WORKDIR/queue-task.json"
echo "STEP queue a task so the pane can show it"
QUEUED="$(cli agent assign --head lead-b --input "$WORKDIR/queue-task.json" --json)"
python3 -c 'import json,sys; body=json.loads(sys.argv[1]); assert body.get("ok") is True, body' "$QUEUED"
QTASK="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["task"]["id"])' "$QUEUED")"
for _ in $(seq 1 40); do
  if python3 - "$(cli command-view --json)" "$QTASK" <<'PY'
import json, sys
body = json.loads(sys.argv[1])
task = next((item for item in body.get("tasks", []) if item.get("id") == sys.argv[2]), None)
raise SystemExit(0 if task and task.get("state") not in (None, "queued", "running") else 1)
PY
  then
    break
  fi
  sleep 0.25
done
python3 - "$(cli command-view --json)" "$QTASK" <<'PY'
import json, sys
body = json.loads(sys.argv[1])
task = next((item for item in body.get("tasks", []) if item.get("id") == sys.argv[2]), None)
assert task and task.get("state") not in (None, "queued", "running"), task
print("queue task", task["id"], task["state"])
PY

echo "STEP command pane pixels show both leads, the queue, last output, and a waiting line"
SHOT="$WORKDIR/command-view.png"
if ! cli host screenshot --pane "$PANE" --output "$SHOT"; then
  echo "FAIL: host screenshot"
  exit 1
fi
test -s "$SHOT"
mkdir -p /tmp/plexi-e2e /opt/cursor/artifacts
cp "$SHOT" /tmp/plexi-e2e/command-view-steer.png
cp "$SHOT" /opt/cursor/artifacts/command-view-steer.png
python3 - "$SHOT" <<'PY'
import struct, sys, zlib
path = sys.argv[1]
data = open(path, "rb").read()
assert data[:8] == b"\x89PNG\r\n\x1a\n", "not a png"
pos = 8
width = height = bit_depth = color_type = None
idat = b""
while pos + 8 <= len(data):
    length = struct.unpack(">I", data[pos:pos + 4])[0]
    kind = data[pos + 4:pos + 8]
    chunk = data[pos + 8:pos + 8 + length]
    pos += 12 + length
    if kind == b"IHDR":
        width, height, bit_depth, color_type = struct.unpack(">IIBB", chunk[:10])
    elif kind == b"IDAT":
        idat += chunk
    elif kind == b"IEND":
        break
assert width and height and width >= 200 and height >= 80, (width, height)
assert bit_depth == 8 and color_type in (2, 6), (bit_depth, color_type)
raw = zlib.decompress(idat)
channels = 3 if color_type == 2 else 4
stride = width * channels
rows = []
prev = bytearray(stride)
i = 0
for _y in range(height):
    filt = raw[i]
    i += 1
    row = bytearray(raw[i:i + stride])
    i += stride
    if filt == 1:
        for x in range(stride):
            left = row[x - channels] if x >= channels else 0
            row[x] = (row[x] + left) & 255
    elif filt == 2:
        for x in range(stride):
            row[x] = (row[x] + prev[x]) & 255
    elif filt == 3:
        for x in range(stride):
            left = row[x - channels] if x >= channels else 0
            row[x] = (row[x] + ((left + prev[x]) // 2)) & 255
    elif filt == 4:
        def paeth(a, b, c):
            p = a + b - c
            pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
            if pa <= pb and pa <= pc:
                return a
            if pb <= pc:
                return b
            return c
        for x in range(stride):
            left = row[x - channels] if x >= channels else 0
            up = prev[x]
            ul = prev[x - channels] if x >= channels else 0
            row[x] = (row[x] + paeth(left, up, ul)) & 255
    elif filt != 0:
        raise SystemExit(f"unsupported png filter {filt}")
    prev = row
    rows.append(row)
target = (0xF9, 0xE2, 0xAF)
close = 0
for row in rows:
    for x in range(0, stride, channels):
        r, g, b = row[x], row[x + 1], row[x + 2]
        dist = (r - target[0]) ** 2 + (g - target[1]) ** 2 + (b - target[2]) ** 2
        if dist <= 40 * 40:
            close += 1
print(f"png {width}x{height} warning-pixels {close}")
if close < 12:
    raise SystemExit(f"waiting line is not in the warning color ({close} pixels near #f9e2af)")
PY
if command -v tesseract >/dev/null 2>&1; then
  tesseract "$SHOT" stdout --psm 6 >"$WORKDIR/shot.txt" 2>"$WORKDIR/shot.ocr" || true
  python3 - "$WORKDIR/shot.txt" <<'PY'
import sys
text = open(sys.argv[1], encoding="utf-8", errors="replace").read().lower()
# Monospace "queue" is read back as "queve" at the acceptance display size.
text = text.replace("queve", "queue")
missing = [word for word in ("lead a", "lead b", "queue", "idle", "waiting") if word not in text]
if missing:
    raise SystemExit("ocr missed " + ", ".join(missing) + "\n" + text)
print("ocr saw both leads, the queue, the last output, and the waiting line")
PY
else
  echo "FAIL: tesseract is required to read the command pane"
  exit 1
fi
echo "screenshot $SHOT"

WRITE_PENDING="$(python3 -c 'import json,sys
body=json.load(open(sys.argv[1]))
text=" ".join(str(body.get(key) or "") for key in ("error","reply","pending_request_id"))
for token in text.split():
    if token.startswith("req_"):
        print(token)
        break
' "$WORKDIR/pending.json")"
if [[ -z "$WRITE_PENDING" ]]; then
  echo "FAIL: pending write did not name a request id"
  cat "$WORKDIR/pending.json"
  exit 1
fi
echo "HUMAN_APPROVE pending $WRITE_PENDING once"
if ! HUMAN_APPROVE "$WRITE_PENDING" once; then
  "$BIN" host screenshot --output "$WORKDIR/approve-miss.png" >/dev/null 2>&1 || true
  echo "FAIL: HUMAN_APPROVE did not grant $WRITE_PENDING"
  "$BIN" assistant permission list || true
  exit 1
fi
echo "STEP the same write proceeds after Allow once"
set +e
WROTE="$(cli command-view send lead-a "write-file")"
set -e
printf '%s\n' "$WROTE" >"$WORKDIR/wrote.json"
python3 -c 'import json,sys; body=json.loads(sys.argv[1]); assert body.get("state")=="succeeded", body' "$WROTE"
test -f "$WORKDIR/out.txt"
python3 -c 'import pathlib,sys; assert pathlib.Path(sys.argv[1]).read_text()=="ok"' "$WORKDIR/out.txt"
echo "HUMAN_APPROVE wrote out.txt"

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
  STEPS="$(python3 -c 'import pathlib,sys; print(pathlib.Path(sys.argv[1]).read_text().count("lead.step"))' "$AUDIT")"
fi
sleep 2
LATER="$STEPS"
if [[ -f "$AUDIT" ]]; then
  LATER="$(python3 -c 'import pathlib,sys; print(pathlib.Path(sys.argv[1]).read_text().count("lead.step"))' "$AUDIT")"
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
  if printf '%s\n' "$CAPTURE" | python3 -c 'import sys; text="".join(sys.stdin.read().split()); raise SystemExit(0 if text.count("agent_cannot_approve") >= 2 else 1)'; then
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

echo "STEP follow streams command.view without another list"
: >"$WORKDIR/follow.ndjson"
cli command-view --follow >"$WORKDIR/follow.ndjson" 2>"$WORKDIR/follow.err" &
FOLLOW_PID=$!
SUBSCRIBED=0
for _ in $(seq 1 40); do
  if python3 - "$WORKDIR/follow.ndjson" <<'PY'
import json, sys
ok = False
for line in open(sys.argv[1]):
    line = line.strip()
    if not line:
        continue
    body = json.loads(line)
    if body.get("type") == "subscribed" and body.get("event") == "command.view" and body.get("app_id") == "plexi.host.command":
        ok = True
raise SystemExit(0 if ok else 1)
PY
  then
    SUBSCRIBED=1
    break
  fi
  sleep 0.25
done
if [[ "$SUBSCRIBED" != 1 ]]; then
  echo "FAIL: follow did not subscribe to command.view"
  cat "$WORKDIR/follow.ndjson" "$WORKDIR/follow.err" >&2 || true
  exit 1
fi
BEFORE="$(wc -l <"$WORKDIR/follow.ndjson" | tr -d ' ')"
cli agent head create follow-lead --display-name 'Follow Lead' --grant assistant.turn=allow --json >/dev/null
SEEN=0
for _ in $(seq 1 40); do
  if python3 - "$WORKDIR/follow.ndjson" "$BEFORE" <<'PY'
import json, sys
lines = [line for line in open(sys.argv[1]) if line.strip()]
start = int(sys.argv[2])
for line in lines[start:]:
    body = json.loads(line)
    heads = body.get("heads") or []
    if (
        body.get("event") == "command.view"
        and body.get("app_id") == "plexi.host.command"
        and any(item.get("id") == "follow-lead" for item in heads)
    ):
        raise SystemExit(0)
raise SystemExit(1)
PY
  then
    SEEN=1
    break
  fi
  sleep 0.25
done
kill "$FOLLOW_PID" >/dev/null 2>&1 || true
wait "$FOLLOW_PID" 2>/dev/null || true
FOLLOW_PID=""
if [[ "$SEEN" != 1 ]]; then
  echo "FAIL: follow did not stream command.view for the new lead"
  cat "$WORKDIR/follow.ndjson" "$WORKDIR/follow.err" >&2 || true
  exit 1
fi
echo "follow streamed command.view from plexi.host.command without another list"

echo "PASS command view steers real leads"
