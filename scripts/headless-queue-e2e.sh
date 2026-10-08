#!/usr/bin/env bash
# Installed-binary check for the headless lead queue (V1-10 steps 3-6).
# Usage: scripts/headless-queue-e2e.sh <PR>
# host.files.write is not granted at head create. The script clicks Allow
# once through HUMAN_APPROVE (scripts/e2e/human.sh). The queue resumes that
# same task after the click. It never calls assistant permission resolve
# or command-view resolve.
set -euo pipefail

if [[ -z "${DISPLAY:-}" ]] && command -v xvfb-run >/dev/null 2>&1; then
  exec xvfb-run -a "$0" "$@"
fi

PR="${1:?usage: scripts/headless-queue-e2e.sh <PR>}"
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

"$BIN" workspace init
if ! "$BIN" host start --background --ephemeral --timeout-secs 45; then
  echo "FAIL: host start"
  exit 1
fi
HOST_STARTED=1

cli() { "$BIN" "$@"; }

echo "STEP create lead B with no assistant pane"
CREATE="$(cli agent head create lead-b --display-name 'Lead B' --grant assistant.turn=allow --grant lead.step=allow --json)"
python3 -c 'import json,sys; body=json.loads(sys.argv[1]); assert body.get("ok") is True' "$CREATE"
BEFORE="$(cli pane list)"
python3 -c 'import json,sys; rows=json.loads(sys.argv[1]); assert all(row.get("title")!="Lead B" for row in rows)' "$BEFORE"

# shellcheck disable=SC1091
source "$ROOT/scripts/e2e/human.sh"
export BIN

printf '%s\n' '{"text":"write-file"}' >"$WORKDIR/task.json"
echo "STEP assign with no pane open asks before it writes"
ASK="$(cli agent assign --head lead-b --input "$WORKDIR/task.json" --json)"
python3 -c 'import json,sys; body=json.loads(sys.argv[1]); assert body.get("ok") is True, body' "$ASK"
ASK_ID="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["task"]["id"])' "$ASK")"
PENDING=""
for _ in $(seq 1 40); do
  VIEW="$(cli command-view --json)"
  PENDING="$(python3 - "$VIEW" "$ASK_ID" <<'PY'
import json, sys
body = json.loads(sys.argv[1])
task = next((item for item in body.get("tasks", []) if item.get("id") == sys.argv[2]), None)
if task is None or task.get("state") != "permission_required":
    raise SystemExit
text = task.get("error") or ""
for token in text.split():
    if token.startswith("req_"):
        print(token)
        raise SystemExit
print("")
PY
)"
  if [[ -n "$PENDING" ]]; then
    break
  fi
  sleep 0.5
done
if [[ -z "$PENDING" ]]; then
  PENDING="$("$BIN" assistant permission list | python3 -c 'import json,sys
data=json.load(sys.stdin)
rows=data.get("pending") or []
print(rows[0].get("pending_request_id","") if rows else "")')"
fi
if [[ -z "$PENDING" ]]; then
  echo "FAIL: headless write did not file a pending approval"
  echo "$VIEW"
  exit 1
fi
test ! -f "$WORKDIR/out.txt"
echo "HUMAN_APPROVE pending $PENDING once"
if ! HUMAN_APPROVE "$PENDING" once; then
  "$BIN" host screenshot --output "$WORKDIR/approve-miss.png" >/dev/null 2>&1 || true
  echo "FAIL: HUMAN_APPROVE did not grant $PENDING"
  "$BIN" assistant permission list || true
  exit 1
fi

echo "STEP the same assignment continues after Allow once"
for _ in $(seq 1 40); do
  VIEW="$(cli command-view --json)"
  if python3 - "$VIEW" "$ASK_ID" <<'PY'
import json, sys
body = json.loads(sys.argv[1])
task = next((item for item in body.get("tasks", []) if item.get("id") == sys.argv[2]), None)
assert task is not None, body
raise SystemExit(0 if task.get("state") == "succeeded" else 1)
PY
  then
    break
  fi
  sleep 0.5
done
test -f "$WORKDIR/out.txt"
python3 -c 'import pathlib,sys; assert pathlib.Path(sys.argv[1]).read_text()=="ok"' "$WORKDIR/out.txt"
AFTER="$(cli pane list)"
python3 -c 'import json,sys; rows=json.loads(sys.argv[1]); assert all(row.get("title")!="Lead B" for row in rows), rows' "$AFTER"
echo "headless assign wrote out.txt and opened no lead pane"

echo "STEP result is in the conversation and command view"
OPEN="$(cli assistant open --head lead-b)"
python3 -c 'import json,sys; assert json.loads(sys.argv[1]).get("ok") is True' "$OPEN"
CONV="$(cli agent conversation --head lead-b --json)"
python3 -c 'import json,sys; text=json.dumps(json.loads(sys.argv[1])); assert "out.txt" in text or "wrote" in text, text' "$CONV"
python3 -c 'import json,sys; body=json.loads(sys.argv[1]); ids=[h.get("id") for h in body.get("heads",[])]; assert "lead-b" in ids' "$VIEW"
echo "conversation and command view show lead B"

PANE="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["pane_id"])' "$OPEN")"
cli pane close "$PANE" >/dev/null

echo "STEP cancel a long task before it finishes"
printf '%s\n' '{"text":"long-task"}' >"$WORKDIR/long.json"
LONG="$(cli agent assign --head lead-b --input "$WORKDIR/long.json" --json)"
LONG_ID="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["task"]["id"])' "$LONG")"
cli agent cancel --id "$LONG_ID" --json >/dev/null
sleep 2
AUDIT="${PROFILE}/permission-audit.jsonl"
STEPS_AFTER=0
if [[ -f "$AUDIT" ]]; then
  STEPS_AFTER="$(python3 -c 'import pathlib,sys; print(pathlib.Path(sys.argv[1]).read_text().count("lead.step"))' "$AUDIT")"
fi
sleep 2
if [[ -f "$AUDIT" ]]; then
  STEPS_LATER="$(python3 -c 'import pathlib,sys; print(pathlib.Path(sys.argv[1]).read_text().count("lead.step"))' "$AUDIT")"
else
  STEPS_LATER=0
fi
python3 -c 'import sys; a,b=int(sys.argv[1]),int(sys.argv[2]); assert a==b, (a,b)' "$STEPS_AFTER" "$STEPS_LATER"
FINAL="$(cli command-view --json)"
python3 - "$FINAL" "$LONG_ID" <<'PY'
import json, sys
body = json.loads(sys.argv[1])
task = next(item for item in body["tasks"] if item["id"] == sys.argv[2])
assert task["state"] in {"cancelled", "failed"}, task
print("cancel settled", task["state"], "lead.step rows stable")
PY
CONV2="$(cli agent conversation --head lead-b --json)"
python3 -c 'import json,sys; text=json.dumps(json.loads(sys.argv[1])); assert "long task done" not in text, text' "$CONV2"

echo "STEP restart keeps a queued task and reports a crashed run"
"$BIN" host stop >/dev/null 2>&1 || true
HOST_STARTED=0
rm -f "$WORKDIR/out.txt"
printf '%s\n' '{"text":"write-file"}' >"$WORKDIR/again.json"
QUEUED="$(cli agent assign --head lead-b --input "$WORKDIR/again.json" --json)"
python3 -c 'import json,sys; body=json.loads(sys.argv[1]); assert body["task"]["state"]=="queued"' "$QUEUED"
AGAIN_ID="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["task"]["id"])' "$QUEUED")"
python3 - "$WORKDIR" <<'PY'
import json
from pathlib import Path
path = Path(".plexi/agents/queue.json")
tasks = json.loads(path.read_text())
tasks.append({
    "id": "task_orphan",
    "head": "lead-b",
    "text": "orphan",
    "state": "running",
    "cancel": False,
    "run_id": "",
    "error": "",
    "created_at": "2026-10-06T00:00:00Z",
    "updated_at": "2026-10-06T00:00:00Z",
})
path.write_text(json.dumps(tasks))
print("planted orphan running task")
PY
if ! "$BIN" host start --background --ephemeral --timeout-secs 45; then
  echo "FAIL: host restart"
  exit 1
fi
HOST_STARTED=1
CLICKED=0
for _ in $(seq 1 40); do
  if [[ -f "$WORKDIR/out.txt" ]]; then
    break
  fi
  if [[ "$CLICKED" == 0 ]]; then
    VIEW="$(cli command-view --json 2>/dev/null || true)"
    PENDING="$(python3 - "$VIEW" "$AGAIN_ID" <<'PY' || true
import json, sys
raw = sys.argv[1]
if not raw.strip():
    raise SystemExit
try:
    body = json.loads(raw)
except json.JSONDecodeError:
    raise SystemExit
task = next((item for item in body.get("tasks", []) if item.get("id") == sys.argv[2]), None)
if task is None or task.get("state") != "permission_required":
    raise SystemExit
for token in (task.get("error") or "").split():
    if token.startswith("req_"):
        print(token)
        raise SystemExit
PY
)"
    if [[ -n "$PENDING" ]]; then
      echo "HUMAN_APPROVE restart pending $PENDING once"
      if HUMAN_APPROVE "$PENDING" once; then
        CLICKED=1
      fi
    fi
  fi
  sleep 0.5
done
test -f "$WORKDIR/out.txt"
RESTORED="$(cli command-view --json)"
python3 - "$RESTORED" <<'PY'
import json, sys
body = json.loads(sys.argv[1])
orphan = next(item for item in body["tasks"] if item["id"] == "task_orphan")
assert orphan["state"] == "outcome_unknown", orphan
print("restart ran the queued task and marked the orphan outcome_unknown")
PY

echo "PASS headless queue e2e"
