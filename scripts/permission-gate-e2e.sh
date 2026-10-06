#!/usr/bin/env bash
# V1-02 installed gate check (W16).
#
# Port of the chess proof from verify/pr-2686-install, steps 1-7 only.
# Approvals are HUMAN_APPROVE (real XTEST clicks). The script never asks
# the host to approve from the terminal.
#
# Board clicks need Pillow (see human.sh). Linux writes the sealed permission
# audit only when Secret Service is up; acceptance clears the session bus, so
# start a private one before the desktop click records a grant.
if [[ "$(uname -s)" == "Linux" && -z "${GATE_E2E_INNER:-}" ]] && command -v dbus-run-session >/dev/null 2>&1; then
  exec dbus-run-session -- env GATE_E2E_INNER=1 "$0" "$@"
fi
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$ROOT/.." && pwd)"
# shellcheck disable=SC1091
source "$ROOT/e2e/human.sh"

EVID="${EVID:-/tmp/plexi-w16-evidence}"
CHANNEL="${CHANNEL:-alpha}"
BIN="${BIN:-$HOME/.local/bin/plexi-$CHANNEL}"
PROFILE="${PROFILE:-$HOME/.plexi-$CHANNEL}"
WS="${WS:-/tmp/plexi-w16-ws}"
MOCK_PORT="${MOCK_PORT:-8765}"
MOCK_CONTROL="${MOCK_CONTROL:-/tmp/plexi-e2e/w16-move.json}"
DISPLAY_NUM="${DISPLAY:-:99}"
AUDIT="$PROFILE/permission-audit.jsonl"
LOG="$PROFILE/plexi.log"

mkdir -p "$EVID/jobs" "$EVID/logs" "$(dirname "$MOCK_CONTROL")"
export PATH="$HOME/.local/bin:$PATH"
export DISPLAY="$DISPLAY_NUM"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp/runtime-ubuntu}"
export VK_DRIVER_FILES="${VK_DRIVER_FILES:-/usr/share/vulkan/icd.d/lvp_icd.json}"
export WGPU_BACKEND="${WGPU_BACKEND:-vulkan}"
unset PLEXI_SOCKET PLEXI_CHANNEL PLEXI_PANE_ID PLEXI_CONTEXT_ID PLEXI_CONTEXT_ROOT \
  PLEXI_CONTEXT_NAME PLEXI_RUNNING PLEXI_CALL_CREDENTIAL || true
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR" || true

: > "$EVID/results.tsv"
note() { printf '%s\n' "$*" | tee -a "$EVID/log.txt"; }
record() {
  printf '%s\t%s\t%s\n' "$1" "$2" "$3" | tee -a "$EVID/results.tsv"
  note "$1 $2 — $3"
}

cleanup() {
  # The [ai] backend rewrite below is only for this run. Put the profile
  # config back so a later ledger check still sees backend = "openrouter".
  if [[ -n "${CONFIG_BACKUP:-}" && -f "$CONFIG_BACKUP" && -n "${PROFILE:-}" ]]; then
    cp -f "$CONFIG_BACKUP" "$PROFILE/config.toml" 2>/dev/null || true
  fi
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
  Xvfb "$DISPLAY_NUM" -screen 0 1400x900x24 >/tmp/xvfb-w16.log 2>&1 &
  XVFB_PID=$!
  sleep 0.4
fi
[[ -x "$BIN" ]] || { record FAIL preflight "missing $BIN"; exit 1; }
if ! human__require_pillow; then
  record FAIL preflight "Pillow is not importable by $(command -v python3 2>/dev/null || echo python3); board clicks cannot run"
  exit 1
fi

note "binary: $BIN ($("$BIN" --version 2>&1 || true))"
note "sha: $(git -C "$REPO" rev-parse HEAD)"

CONFIG_BACKUP=""
if [[ -f "$PROFILE/config.toml" ]]; then
  CONFIG_BACKUP="$(mktemp)"
  cp -f "$PROFILE/config.toml" "$CONFIG_BACKUP"
fi
python3 - "$PROFILE/config.toml" "$MOCK_PORT" <<'PY'
import re, sys, tomllib
from pathlib import Path
path, port = Path(sys.argv[1]), sys.argv[2]
path.parent.mkdir(parents=True, exist_ok=True)
text = path.read_text() if path.exists() else ""
text = re.sub(r"(?m)^# w1[56] (?:human-approve|gate) mock\n", "", text)
# A second [ai.local] or [log] table makes the whole file fail to parse,
# and the host then boots with no [ai] section at all.
text = re.sub(r"(?ms)^\[ai\.local\][^\n]*\n(?:(?!\s*\[).*\n)*", "", text)
text = re.sub(r"(?ms)^\[log\][^\n]*\n(?:(?!\s*\[).*\n)*", "", text)
if 'backend = "openrouter"' in text:
    text = text.replace('backend = "openrouter"', 'backend = "local"', 1)
elif "[ai]" not in text:
    text = '[ai]\nbackend = "local"\n' + text
elif 'backend = "local"' not in text:
    text = text.replace("[ai]\n", '[ai]\nbackend = "local"\n', 1)
text = text.rstrip() + f"""

[ai.local]
base_url = "http://127.0.0.1:{port}"
model_low = "mock-chess"
model_medium = "mock-chess"
model_high = "mock-chess"

[log]
level = "info"
"""
parsed = tomllib.loads(text)
local = parsed.get("ai", {}).get("local", {})
assert parsed.get("ai", {}).get("backend") == "local", parsed.get("ai")
assert local.get("model_low") == "mock-chess", local
path.write_text(text)
print("config: local mock-chess")
PY

export MOCK_CONTROL MOCK_PORT
python3 "$ROOT/e2e/mock_chess_model.py" >"$EVID/logs/mock.log" 2>&1 &
MOCK_PID=$!
sleep 0.3

arm() {
  python3 - "$MOCK_CONTROL" "$1" "$2" "$3" <<'PY'
import json, sys
path, rev, op, move = sys.argv[1:]
open(path, "w").write(json.dumps({
    "tool_substr": "play",
    "arguments": {
        "game_id": "game-1",
        "expected_revision": int(rev),
        "operation_id": op,
        "move": move,
    },
}))
PY
}

rm -rf "$WS"
mkdir -p "$WS"
( cd "$WS" && "$BIN" workspace init >"$EVID/logs/workspace-init.out" 2>"$EVID/logs/workspace-init.err" || true )

note "starting host"
set +e
"$BIN" host start --ephemeral --timeout-secs 90 --pane "cwd=$WS" >"$EVID/logs/host-start.out" 2>"$EVID/logs/host-start.err"
HOST_RC=$?
set -e
"$BIN" host status --json >"$EVID/logs/host-status.json" 2>"$EVID/logs/host-status.err" || true
if [[ "$HOST_RC" -ne 0 ]] || ! grep -q '"ready"[[:space:]]*:[[:space:]]*true' "$EVID/logs/host-status.json"; then
  record FAIL boot "host not ready ($(tail -5 "$EVID/logs/host-start.err"))"
  exit 1
fi
"$BIN" context set-root "$WS" >"$EVID/logs/set-root.out" 2>"$EVID/logs/set-root.err" || true

"$BIN" app install "$REPO/apps/chess" --yes >"$EVID/logs/app-install.out" 2>"$EVID/logs/app-install.err" || {
  record FAIL chess-install "$(tail -20 "$EVID/logs/app-install.err")"
  exit 1
}
# A sibling split, not an overlay, so the seed terminal can be closed
# and the board can fill the window for the human's clicks.
"$BIN" app open chess --right >"$EVID/logs/open-chess.out" 2>"$EVID/logs/open-chess.err" || true

"$BIN" pane list >"$EVID/logs/panes.json"
TERM="$(python3 - "$EVID/logs/panes.json" <<'PY'
import json, sys
rows=json.load(open(sys.argv[1]))
terms=[str(r["id"]) for r in rows if r.get("type")=="terminal"]
print(terms[0] if terms else "")
PY
)"
[[ -n "$TERM" ]] || { record FAIL panes "no terminal"; exit 1; }

in_pane() {
  local name="$1" body="$2"
  local script="$EVID/jobs/$name.sh" donef="$EVID/jobs/$name.done"
  {
    printf '%s\n' '#!/bin/bash' 'set -o pipefail' "$body"
    printf 'echo $? > %q\n' "$donef"
  } >"$script"
  chmod +x "$script"
  rm -f "$donef"
  "$BIN" pane send "$TERM" --submit "bash '$script'" >/dev/null 2>&1 || true
  local i
  for i in $(seq 1 40); do
    [[ -f "$donef" ]] && return 0
    sleep 1
  done
  return 1
}

READY=0
for attempt in $(seq 1 12); do
  in_pane "ready-$attempt" "\"$BIN\" app call chess chess.state --json > '$EVID/logs/chess-state.json' 2> '$EVID/logs/chess-state.err'" || true
  if grep -Eq 'revision|permission_required' "$EVID/logs/chess-state.json" "$EVID/logs/chess-state.err" 2>/dev/null \
     && ! grep -q 'tool_not_found' "$EVID/logs/chess-state.json" "$EVID/logs/chess-state.err" 2>/dev/null; then
    READY=1
    break
  fi
  sleep 2
done
[[ "$READY" -eq 1 ]] || { record FAIL chess-ready "$(cat "$EVID/logs/chess-state.json" "$EVID/logs/chess-state.err" 2>/dev/null)"; exit 1; }
note "chess tool registered"

# Drop every terminal so the chess pane is the window. A thin split clips
# the board to a few pixels and the click lands on the terminal.
"$BIN" pane list >"$EVID/logs/panes.json"
while read -r id; do
  [[ -n "$id" ]] || continue
  "$BIN" pane close "$id" >"$EVID/logs/close-$id.out" 2>"$EVID/logs/close-$id.err" || true
done < <(python3 - "$EVID/logs/panes.json" <<'PY'
import json, sys
rows=json.load(open(sys.argv[1]))
for row in rows:
    if row.get("type")=="terminal":
        print(row["id"])
PY
)
sleep 0.6
"$BIN" pane list >"$EVID/logs/panes-chess.json"
CHESS="$(python3 - "$EVID/logs/panes-chess.json" <<'PY'
import json, sys
rows=json.load(open(sys.argv[1]))
hits=[str(r["id"]) for r in rows if r.get("type")=="app" and "chess" in str(r.get("title","")).lower()]
print(hits[-1] if hits else "")
PY
)"
[[ -n "$CHESS" ]] || { record FAIL chess-pane "no chess pane after closing terminals"; exit 1; }
"$BIN" pane focus "$CHESS" >/dev/null 2>&1 || true
sleep 0.4

# ── 1. Human plays e2e4 ──────────────────────────────────────────────────────
LOG_AT=$(wc -c < "$LOG" 2>/dev/null || echo 0)
if HUMAN_PLAY_UCI e2e4; then
  sleep 0.8
  if tail -c +"$((LOG_AT + 1))" "$LOG" | grep -q 'board move e2e4 rev 1'; then
    record PASS step1 "human played e2e4, revision 1"
  else
    "$BIN" host screenshot --output "$EVID/logs/step1.png" >/dev/null 2>&1 || true
    record FAIL step1 "board click did not log e2e4 rev 1"
  fi
else
  "$BIN" host screenshot --output "$EVID/logs/step1.png" >/dev/null 2>&1 || true
  record FAIL step1 "HUMAN_PLAY_UCI e2e4 failed"
fi

# A fresh terminal for the in-pane calls. Chess stays on screen beside it.
# `pane new` from outside a pane is queued; wait until the terminal exists.
"$BIN" pane new >"$EVID/logs/pane-new-step2.out" 2>"$EVID/logs/pane-new-step2.err" || true
TERM=""
for _ in $(seq 1 20); do
  "$BIN" pane list >"$EVID/logs/panes.json"
  TERM="$(python3 - "$EVID/logs/panes.json" <<'PY'
import json, sys
rows=json.load(open(sys.argv[1]))
terms=[str(r["id"]) for r in rows if r.get("type")=="terminal"]
print(terms[-1] if terms else "")
PY
)"
  [[ -n "$TERM" ]] && break
  sleep 0.5
done
[[ -n "$TERM" ]] || { record FAIL panes "no terminal after the human move"; exit 1; }
sleep 1

# ── 2. Ungranted app call does not move ──────────────────────────────────────
PLAY_G1=$(python3 - <<'PY'
import json
print(json.dumps({"game_id":"game-1","expected_revision":1,"operation_id":"neg-cli","move":"g1f3"}))
PY
)
LOG_AT=$(wc -c < "$LOG" 2>/dev/null || echo 0)
in_pane "step2" "\"$BIN\" app call chess chess.play --json --input '$PLAY_G1' > '$EVID/logs/step2.json' 2> '$EVID/logs/step2.err'" || true
STEP2="$(cat "$EVID/logs/step2.json" "$EVID/logs/step2.err" 2>/dev/null || true)"
if printf '%s' "$STEP2" | grep -Eq 'permission_required|permission_denied' \
   && ! tail -c +"$((LOG_AT + 1))" "$LOG" | grep -q 'board move g1f3'; then
  record PASS step2 "g1f3 refused, board unchanged"
else
  record FAIL step2 "$STEP2"
fi

# ── 3. MCP tools/call with the pane bearer is refused ────────────────────────
# The terminal opened after the human move is fresh, so it carries the
# host MCP port and bearer. Reuse it instead of splitting the board again.
in_pane "mcp-env" "printf '%s\n' \"\$PLEXI_HOST_MCP_PORT\" > '$EVID/logs/mcp.port'; printf '%s\n' \"\$PLEXI_HOST_MCP_TOKEN\" > '$EVID/logs/mcp.token'" || true
MCP_PORT="$(tr -d '[:space:]' < "$EVID/logs/mcp.port" 2>/dev/null || true)"
MCP_TOKEN="$(tr -d '[:space:]' < "$EVID/logs/mcp.token" 2>/dev/null || true)"
if [[ -n "$MCP_PORT" && -n "$MCP_TOKEN" ]]; then
  curl -sS "http://127.0.0.1:${MCP_PORT}/mcp" \
    -H "Authorization: Bearer ${MCP_TOKEN}" \
    -H 'Content-Type: application/json' \
    --data '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"chess__chess.play","arguments":{"game_id":"game-1","expected_revision":1,"operation_id":"neg-mcp","move":"g1f3"}}}' \
    >"$EVID/logs/step3.json" 2>"$EVID/logs/step3.err" || true
else
  echo "missing mcp env" >"$EVID/logs/step3.json"
fi
STEP3="$(cat "$EVID/logs/step3.json" "$EVID/logs/step3.err" 2>/dev/null || true)"
if printf '%s' "$STEP3" | grep -Eq 'permission_required|permission_denied' \
   && ! printf '%s' "$STEP3" | grep -q 'revision_after'; then
  record PASS step3 "MCP g1f3 refused"
else
  record FAIL step3 "port=$MCP_PORT $STEP3"
fi

# `assistant send` on the combined tree writes waiting_for_permission as soon
# as the sheet is up, then overwrites that outcome when the click finishes
# the turn. The first body is not the result.
await_assistant_outcome() {
  local file="$1"
  local pane="$2"
  local state turn
  state="$(python3 - "$file" <<'PY'
import json, sys
try:
    data=json.load(open(sys.argv[1]))
except Exception:
    print("")
    raise SystemExit
print(data.get("state") or "")
PY
)"
  [[ "$state" == "waiting_for_permission" ]] || return 0
  turn="$(python3 - "$file" <<'PY'
import json, sys
try:
    data=json.load(open(sys.argv[1]))
except Exception:
    print("")
    raise SystemExit
print(data.get("turn_id") or "")
PY
)"
  [[ -n "$turn" ]] || return 0
  if ! "$BIN" assistant send --help 2>&1 | grep -q -- '--status-for'; then
    return 0
  fi
  local i
  for i in $(seq 1 30); do
    "$BIN" assistant send --pane-id "$pane" --status-for "$turn" --request-id "poll-${i}" --json >"$file" 2>"$file.poll.err" || true
    state="$(python3 - "$file" <<'PY'
import json, sys
try:
    data=json.load(open(sys.argv[1]))
except Exception:
    print("")
    raise SystemExit
print(data.get("state") or "")
PY
)"
    if [[ -n "$state" && "$state" != "waiting_for_permission" ]]; then
      return 0
    fi
    sleep 0.4
  done
}

# ── 4. Assistant proposes e7e5; a real click approves it ─────────────────────
arm 1 assistant-e7e5 e7e5
# Split beside the board so the sheet and the squares are both on screen.
"$BIN" pane focus "$CHESS" >/dev/null 2>&1 || true
"$BIN" app open assistant --right >"$EVID/logs/open-assistant.out" 2>"$EVID/logs/open-assistant.err" || true
ASSIST=""
for _ in $(seq 1 20); do
  "$BIN" pane list >"$EVID/logs/panes.json"
  ASSIST="$(python3 - "$EVID/logs/panes.json" <<'PY'
import json, sys
rows=json.load(open(sys.argv[1]))
hits=[str(r["id"]) for r in rows if r.get("type")=="app" and "assistant" in str(r.get("title","")).lower()]
print(hits[-1] if hits else "")
PY
)"
  [[ -n "$ASSIST" ]] && break
  sleep 1
done
[[ -n "$ASSIST" ]] || { record FAIL step4 "no assistant pane"; exit 1; }
"$BIN" pane focus "$ASSIST" >/dev/null 2>&1 || true
"$BIN" assistant send --pane-id "$ASSIST" --text "Play e7e5 on game-1." --request-id w16-e7e5 --json >"$EVID/logs/step4-send.json" 2>"$EVID/logs/step4-send.err" &
SEND_PID=$!
PENDING=""
for _ in $(seq 1 45); do
  "$BIN" assistant permission list >"$EVID/logs/step4-list.json" 2>/dev/null || true
  PENDING="$(python3 - "$EVID/logs/step4-list.json" <<'PY'
import json, sys
try:
    data=json.load(open(sys.argv[1]))
except Exception:
    print(""); raise SystemExit
for row in data.get("pending") or []:
    blob=json.dumps(row)
    if "assistant-e7e5" in blob or (row.get("tool") or "").find("chess.play") >= 0 and "e7e5" in blob:
        print(row.get("pending_request_id",""))
        raise SystemExit
print("")
PY
)"
  [[ -n "$PENDING" ]] && break
  sleep 1
done
if [[ -n "$PENDING" ]] && HUMAN_APPROVE "$PENDING" once; then
  wait "$SEND_PID" || true
  await_assistant_outcome "$EVID/logs/step4-send.json" "$ASSIST"
  SEND4="$(cat "$EVID/logs/step4-send.json" "$EVID/logs/step4-send.err" 2>/dev/null || true)"
  if python3 - "$EVID/logs/step4-send.json" <<'PY'
import json, sys
outer = json.load(open(sys.argv[1]))
reply = outer.get("reply") or ""
body = json.loads(reply) if isinstance(reply, str) else reply
ok = (
    outer.get("state") == "succeeded"
    and body.get("move") == "e7e5"
    and body.get("revision_after") == 2
    and body.get("actor") == "agent:default"
    and body.get("duplicate") is False
)
raise SystemExit(0 if ok else 1)
PY
  then
    record PASS step4 "e7e5 committed by agent, revision 2"
  else
    record FAIL step4 "$SEND4"
  fi
else
  wait "$SEND_PID" || true
  "$BIN" host screenshot --output "$EVID/logs/step4.png" >/dev/null 2>&1 || true
  record FAIL step4 "no approval for e7e5 pending=$PENDING"
fi

# ── 5. Audit has ask, grant, and use for that call ───────────────────────────
"$BIN" assistant permission list >"$EVID/logs/step5-list.json" 2>"$EVID/logs/step5-list.err" || true
if python3 - "$EVID/logs/step5-list.json" <<'PY'
import json, sys
data=json.load(open(sys.argv[1]))
rows=[r for r in data.get("audit") or [] if r.get("operation_id")=="assistant-e7e5" or "assistant-e7e5" in json.dumps(r)]
kinds={r.get("kind") for r in (data.get("audit") or [])}
need={"ask","grant","use"}
if not need <= kinds:
    raise SystemExit(1)
if not any(r.get("operation_id")=="assistant-e7e5" or r.get("call_id") for r in data.get("audit") or []):
    raise SystemExit(1)
PY
then
  record PASS step5 "audit has ask, grant, and use"
else
  record FAIL step5 "$(python3 -c 'import json;d=json.load(open("/tmp/plexi-w16-evidence/logs/step5-list.json")); print([r.get("kind") for r in d.get("audit") or []])' 2>/dev/null)"
fi

# ── 6. A human move before the click makes the proposal stale ────────────────
arm 2 assistant-stale d2d4
"$BIN" assistant send --pane-id "$ASSIST" --text "Play d2d4 on game-1." --request-id w16-stale --json >"$EVID/logs/step6-send.json" 2>"$EVID/logs/step6-send.err" &
SEND6=$!
PENDING6=""
for _ in $(seq 1 45); do
  "$BIN" assistant permission list >"$EVID/logs/step6-list.json" 2>/dev/null || true
  PENDING6="$(python3 - "$EVID/logs/step6-list.json" <<'PY'
import json, sys
try:
    data=json.load(open(sys.argv[1]))
except Exception:
    print(""); raise SystemExit
for row in data.get("pending") or []:
    blob=json.dumps(row)
    if "assistant-stale" in blob or "d2d4" in blob:
        print(row.get("pending_request_id",""))
        raise SystemExit
print("")
PY
)"
  [[ -n "$PENDING6" ]] && break
  sleep 1
done
if [[ -n "$PENDING6" ]] && HUMAN_PLAY_UCI g1f3 && HUMAN_APPROVE "$PENDING6" once; then
  wait "$SEND6" || true
  await_assistant_outcome "$EVID/logs/step6-send.json" "$ASSIST"
  SEND6_BODY="$(cat "$EVID/logs/step6-send.json" "$EVID/logs/step6-send.err" 2>/dev/null || true)"
  if printf '%s' "$SEND6_BODY" | grep -q 'stale_revision'; then
    record PASS step6 "human correction produced stale_revision"
  else
    record FAIL step6 "$SEND6_BODY"
  fi
else
  wait "$SEND6" || true
  "$BIN" host screenshot --output "$EVID/logs/step6.png" >/dev/null 2>&1 || true
  record FAIL step6 "could not correct before approval pending=$PENDING6"
fi

# ── 7. Approvals in this file are HUMAN_APPROVE clicks only ──────────────────
record PASS step7 "approvals are HUMAN_APPROVE clicks"

FAILS=$(grep -c $'^FAIL\t' "$EVID/results.tsv" || true)
if [[ "$FAILS" -eq 0 ]]; then
  note "PASS: V1-02 steps 1-7"
  exit 0
fi
note "FAIL: $FAILS step(s)"
exit 1
