#!/usr/bin/env bash
# Installed-build proof for W15: HUMAN_APPROVE clicks the real permission
# sheet and a pending chess move commits. No CLI resolve.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$ROOT/../.." && pwd)"
# shellcheck disable=SC1091
source "$ROOT/human.sh"

EVID="${EVID:-/tmp/plexi-w15-evidence}"
CHANNEL="${CHANNEL:-alpha}"
BIN="${BIN:-$HOME/.local/bin/plexi-$CHANNEL}"
PROFILE="${PROFILE:-$HOME/.plexi-$CHANNEL}"
WS="${WS:-/tmp/plexi-w15-ws}"
MOCK_PORT="${MOCK_PORT:-8765}"
MOCK_CONTROL="${MOCK_CONTROL:-/tmp/plexi-e2e/move.json}"
DISPLAY_NUM="${DISPLAY:-:99}"
AUDIT="$PROFILE/permission-audit.jsonl"
LOG="$PROFILE/plexi.log"

mkdir -p "$EVID" "$(dirname "$MOCK_CONTROL")"
export PATH="$HOME/.local/bin:$PATH"
export DISPLAY="$DISPLAY_NUM"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp/runtime-ubuntu}"
export VK_DRIVER_FILES="${VK_DRIVER_FILES:-/usr/share/vulkan/icd.d/lvp_icd.json}"
export WGPU_BACKEND="${WGPU_BACKEND:-vulkan}"
unset PLEXI_SOCKET PLEXI_CHANNEL PLEXI_PANE_ID PLEXI_CONTEXT_ID PLEXI_CONTEXT_ROOT \
  PLEXI_CONTEXT_NAME PLEXI_RUNNING PLEXI_CALL_CREDENTIAL || true
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR" || true

fail() {
  echo "FAIL: $*" | tee -a "$EVID/result.txt"
  exit 1
}
note() { echo "$*" | tee -a "$EVID/log.txt"; }

: > "$EVID/log.txt"
: > "$EVID/result.txt"

cleanup() {
  if [[ -n "${BIN:-}" && -x "$BIN" ]]; then
    "$BIN" host stop >>"$EVID/log.txt" 2>&1 || true
  fi
  if [[ -n "${MOCK_PID:-}" ]]; then
    kill "$MOCK_PID" 2>/dev/null || true
  fi
  if [[ -n "${XVFB_PID:-}" ]]; then
    kill "$XVFB_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

if ! pgrep -f "Xvfb $DISPLAY_NUM" >/dev/null 2>&1; then
  Xvfb "$DISPLAY_NUM" -screen 0 1400x900x24 >/tmp/xvfb-w15.log 2>&1 &
  XVFB_PID=$!
  sleep 0.4
fi

if [[ ! -x "$BIN" ]]; then
  fail "installed binary missing at $BIN"
fi
note "binary: $BIN ($("$BIN" --version 2>&1 || true))"
note "sha: $(git -C "$REPO" rev-parse HEAD)"

python3 - "$PROFILE/config.toml" "$MOCK_PORT" <<'PY'
import sys
from pathlib import Path
path, port = Path(sys.argv[1]), sys.argv[2]
path.parent.mkdir(parents=True, exist_ok=True)
text = path.read_text() if path.exists() else ""
if 'backend = "openrouter"' in text:
    text = text.replace('backend = "openrouter"', 'backend = "local"', 1)
elif "[ai]" not in text:
    text = '[ai]\nbackend = "local"\n' + text
elif 'backend = "local"' not in text:
    text = text.replace("[ai]\n", '[ai]\nbackend = "local"\n', 1)
marker = "\n# w15 human-approve mock\n"
if marker not in text:
    text += f"""{marker}
[ai.local]
base_url = "http://127.0.0.1:{port}"
model_low = "mock-chess"
model_medium = "mock-chess"
model_high = "mock-chess"

[log]
level = "info"
"""
path.write_text(text)
PY

export MOCK_CONTROL MOCK_PORT
python3 "$ROOT/mock_chess_model.py" >"$EVID/mock.log" 2>&1 &
MOCK_PID=$!
sleep 0.3

python3 - "$MOCK_CONTROL" <<'PY'
import json, sys
open(sys.argv[1], "w").write(json.dumps({
    "tool_substr": "play",
    "arguments": {
        "game_id": "game-1",
        "expected_revision": 0,
        "operation_id": "human-approve-e2e",
        "move": "e2e4",
    },
}))
PY

rm -rf "$WS"
mkdir -p "$WS"
( cd "$WS" && "$BIN" workspace init >"$EVID/workspace-init.out" 2>"$EVID/workspace-init.err" || true )

note "starting host"
set +e
"$BIN" host start --ephemeral --timeout-secs 90 --pane "cwd=$WS" >"$EVID/host-start.out" 2>"$EVID/host-start.err"
HOST_RC=$?
set -e
"$BIN" host status --json >"$EVID/host-status.json" 2>"$EVID/host-status.err" || true
if [[ "$HOST_RC" -ne 0 ]] || ! grep -q '"ready"[[:space:]]*:[[:space:]]*true' "$EVID/host-status.json"; then
  fail "host did not become ready (exit $HOST_RC). status=$(cat "$EVID/host-status.json") stderr=$(tail -30 "$EVID/host-start.err")"
fi
note "host ready: $(tr '\n' ' ' < "$EVID/host-status.json")"

"$BIN" app install "$REPO/apps/chess" --yes >"$EVID/app-install.out" 2>"$EVID/app-install.err" || fail "chess install failed: $(tail -20 "$EVID/app-install.err")"
"$BIN" app open chess >"$EVID/open-chess.out" 2>"$EVID/open-chess.err" || true
mkdir -p "$EVID/jobs"
"$BIN" pane list >"$EVID/panes-boot.json" 2>/dev/null || true
TERM="$(python3 - "$EVID/panes-boot.json" <<'PY'
import json, sys
rows=json.load(open(sys.argv[1]))
terms=[str(r["id"]) for r in rows if r.get("type")=="terminal"]
print(terms[0] if terms else "")
PY
)"
[[ -n "$TERM" ]] || fail "no terminal pane to probe chess tools"
in_pane() {
  local name="$1" body="$2"
  local script="$EVID/jobs/$name.sh"
  local donef="$EVID/jobs/$name.done"
  cat >"$script" <<EOF
#!/bin/bash
set -o pipefail
$body
echo \$? > "$donef"
EOF
  chmod +x "$script"
  rm -f "$donef"
  "$BIN" pane send "$TERM" --submit "bash '$script'" >/dev/null 2>&1 || true
  local i
  for i in $(seq 1 30); do
    [[ -f "$donef" ]] && return 0
    sleep 1
  done
  return 1
}
CHESS_READY=0
for attempt in $(seq 1 12); do
  in_pane "ready-$attempt" "\"$BIN\" app call chess chess.state --json > '$EVID/chess-state.json' 2> '$EVID/chess-state.err'" || true
  if [[ -f "$EVID/chess-state.json" || -f "$EVID/chess-state.err" ]] \
     && ! grep -q 'tool_not_found' "$EVID/chess-state.json" "$EVID/chess-state.err" 2>/dev/null \
     && grep -Eq 'revision|permission_required' "$EVID/chess-state.json" "$EVID/chess-state.err" 2>/dev/null; then
    CHESS_READY=1
    break
  fi
  sleep 2
done
[[ "$CHESS_READY" -eq 1 ]] || fail "chess.state never registered: $(cat "$EVID/chess-state.json" "$EVID/chess-state.err" 2>/dev/null)"
note "chess tool registered"
"$BIN" app open assistant >"$EVID/open-assistant.out" 2>"$EVID/open-assistant.err" || true

ASSIST=""
for _ in $(seq 1 20); do
  "$BIN" pane list >"$EVID/panes.json" 2>/dev/null || true
  ASSIST="$(python3 - "$EVID/panes.json" <<'PY'
import json, sys
rows=json.load(open(sys.argv[1]))
hits=[str(r["id"]) for r in rows if r.get("type")=="app" and "assistant" in str(r.get("title","")).lower()]
print(hits[-1] if hits else "")
PY
)"
  [[ -n "$ASSIST" ]] && break
  sleep 1
done
[[ -n "$ASSIST" ]] || fail "assistant pane did not open: $(cat "$EVID/panes.json" "$EVID/open-assistant.err")"
note "assistant pane $ASSIST"

"$BIN" pane focus "$ASSIST" >"$EVID/focus.out" 2>"$EVID/focus.err" || true
"$BIN" assistant send --pane-id "$ASSIST" --text "Play e2e4 on game-1." --request-id human-approve-smoke --json >"$EVID/send.json" 2>"$EVID/send.err" &
SEND_PID=$!

PENDING=""
for _ in $(seq 1 45); do
  "$BIN" assistant permission list >"$EVID/pending.json" 2>/dev/null || true
  PENDING="$(python3 - "$EVID/pending.json" <<'PY'
import json, sys
try:
    data=json.load(open(sys.argv[1]))
except Exception:
    print(""); raise SystemExit
for row in data.get("pending") or []:
    tool=str(row.get("tool") or "")
    blob=json.dumps(row)
    if "chess.play" in tool or "chess_play" in tool or "chess.play" in blob:
        print(row.get("pending_request_id",""))
        raise SystemExit
print("")
PY
)"
  [[ -n "$PENDING" ]] && break
  sleep 1
done
[[ -n "$PENDING" ]] || fail "no pending chess request. list=$(cat "$EVID/pending.json" 2>/dev/null) send=$(tail -40 "$EVID/send.err")"
note "pending $PENDING"

if ! HUMAN_APPROVE "$PENDING" once; then
  "$BIN" host screenshot --output "$EVID/miss.png" >/dev/null 2>&1 || true
  "$BIN" pane state "$ASSIST" >"$EVID/assistant-state.json" 2>"$EVID/assistant-state.err" || true
  fail "HUMAN_APPROVE did not resolve $PENDING. pane state tail: $(python3 - "$EVID/assistant-state.json" <<'PY'
import json,sys
try:
    data=json.load(open(sys.argv[1]))
except Exception as exc:
    print(exc); raise SystemExit
nodes=(data.get("semantic") or {}).get("nodes") or []
labels=[n.get("label") for n in nodes if n.get("label")]
print("labels", labels[:30], "nodes", len(nodes))
PY
)"
fi
note "HUMAN_APPROVE resolved $PENDING"

wait "$SEND_PID" || true
SEND_BODY="$(cat "$EVID/send.json" "$EVID/send.err" 2>/dev/null || true)"
note "send body: $SEND_BODY"

if human__pending_present "$PENDING"; then
  fail "pending $PENDING still listed after HUMAN_APPROVE"
fi

if ! printf '%s' "$SEND_BODY" | grep -Eq 'revision_after'; then
  fail "approved call did not return a chess receipt. send=$SEND_BODY"
fi
if ! printf '%s' "$SEND_BODY" | grep -q 'e2e4'; then
  fail "approved receipt is not e2e4. send=$SEND_BODY"
fi

echo "PASS: HUMAN_APPROVE $PENDING committed the chess move" | tee "$EVID/result.txt"
note "PASS"
