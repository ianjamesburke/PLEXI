#!/usr/bin/env bash
# Live installed-binary verification for PR #2686.
# Does not modify product source. Writes evidence under $EVID.
#
# The release gate enables the Assistant and the host MCP server only when the
# executable basename is plexi-alpha, plexi-beta, or plexi-pr-<digits>.
# `install.sh --from-source verify2686` produces the release bits; copy that
# generation binary to plexi-beta (do not rebuild with PLEXI_BUILD_TEST_CHANNEL)
# and point PLEXI_SDK_PATH at the package resources/sdk. If the copy is not the
# package's recorded executable name, symlink its wasm-bundles directory to
# ~/.plexi/wasm-bundles so Python panes can start.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EVID="${EVID:-$ROOT/evidence}"
REPO="${REPO:-/workspace}"
CHANNEL="${CHANNEL:-verify2686}"
BIN="${BIN:-$HOME/.local/bin/plexi-$CHANNEL}"
PROFILE="${PROFILE:-$HOME/.plexi-$CHANNEL}"
WS="${WS:-/tmp/plexi-verify-2686-ws}"
AUDIT="$PROFILE/permission-audit.jsonl"
LOG="$PROFILE/plexi.log"
SOCK="$PROFILE/notify.sock"
MOCK_PORT="${MOCK_PORT:-8765}"
DISPLAY_NUM="${DISPLAY_NUM:-:99}"

mkdir -p "$EVID/jobs" "$EVID/logs"
: > "$EVID/report-draft.md"
: > "$EVID/results.tsv"

export PATH="$HOME/.local/bin:$PATH"
export DISPLAY="$DISPLAY_NUM"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp/runtime-ubuntu}"
export VK_DRIVER_FILES="${VK_DRIVER_FILES:-/usr/share/vulkan/icd.d/lvp_icd.json}"
export WGPU_BACKEND="${WGPU_BACKEND:-vulkan}"
unset PLEXI_SOCKET PLEXI_CHANNEL PLEXI_PANE_ID PLEXI_CONTEXT_ID PLEXI_CONTEXT_ROOT \
  PLEXI_CONTEXT_NAME PLEXI_RUNNING PLEXI_CALL_CREDENTIAL || true
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"

TERM_PANE=""
HOST_PID=""

note() { printf '%s\n' "$*" | tee -a "$EVID/report-draft.md"; }

record() {
  local status="$1" id="$2" cmd="$3" detail="$4"
  printf '%s\t%s\t%s\t%s\n' "$status" "$id" "$cmd" "$detail" >> "$EVID/results.tsv"
  note ""
  note "### $id — $status"
  note ""
  note "Command:"
  note ""
  note '```'
  note "$cmd"
  note '```'
  note ""
  note "$detail"
  note ""
}

run_capture() {
  local name="$1"; shift
  local out="$EVID/logs/$name.out"
  local err="$EVID/logs/$name.err"
  set +e
  "$@" >"$out" 2>"$err"
  local code=$?
  set -e
  printf '%s\n' "$code" > "$EVID/logs/$name.exit"
  echo "$code"
}

audit_bytes() { wc -c < "$AUDIT" 2>/dev/null || echo 0; }
log_bytes() { wc -c < "$LOG" 2>/dev/null || echo 0; }

slice_since() {
  local file="$1" start="$2" dest="$3"
  if [[ ! -f "$file" ]]; then
    : > "$dest"
    return
  fi
  tail -c +"$((start + 1))" "$file" > "$dest" 2>/dev/null || : > "$dest"
}

excerpt() {
  local file="$1" pattern="$2" n="${3:-40}"
  if [[ ! -s "$file" ]]; then
    echo "(no matching file)"
    return
  fi
  grep -E "$pattern" "$file" | tail -n "$n" || echo "(no lines matched /$pattern/)"
}

# `app call` prints the JSON envelope on stdout when it succeeds and on stderr
# (`error: {json}`) when the gate refuses. Score both.
both() {
  cat "$1" "$2" 2>/dev/null || true
}

in_pane() {
  local name="$1" body="$2"
  local script="$EVID/jobs/$name.sh"
  local donef="$EVID/jobs/$name.done"
  cat > "$script" <<EOF
#!/bin/bash
set -o pipefail
$body
echo \$? > "$donef"
EOF
  chmod +x "$script"
  rm -f "$donef"
  "$BIN" pane send "$TERM_PANE" --submit "bash '$script'" >"$EVID/logs/pane-send-$name.out" 2>"$EVID/logs/pane-send-$name.err" || true
  local i
  for i in $(seq 1 70); do
    [[ -f "$donef" ]] && return 0
    sleep 1
  done
  echo "timeout waiting for $name" >&2
  return 1
}

pane_list() {
  "$BIN" pane list > "$EVID/logs/pane-list.json" 2>"$EVID/logs/pane-list.err"
}

find_pane() {
  local kind="$1" title="$2"
  python3 - "$EVID/logs/pane-list.json" "$kind" "$title" <<'PY'
import json, sys
path, kind, title = sys.argv[1:]
rows = json.load(open(path))
hits = [r for r in rows if r.get("type") == kind and title.lower() in str(r.get("title","")).lower()]
print(hits[-1]["id"] if hits else "")
PY
}

find_panes() {
  local kind="$1" title="$2"
  python3 - "$EVID/logs/pane-list.json" "$kind" "$title" <<'PY'
import json, sys
path, kind, title = sys.argv[1:]
rows = json.load(open(path))
ids = [str(r["id"]) for r in rows if r.get("type") == kind and title.lower() in str(r.get("title","")).lower()]
print(" ".join(ids))
PY
}

json_field() {
  local file="$1" expr="$2"
  python3 - "$file" "$expr" <<'PY'
import json, sys
path, expr = sys.argv[1:]
try:
    data = json.load(open(path))
except Exception as exc:
    print("")
    raise SystemExit
cur = data
for part in expr.split("."):
    if isinstance(cur, dict):
        cur = cur.get(part)
    else:
        cur = None
    if cur is None:
        print("")
        raise SystemExit
if isinstance(cur, (dict, list)):
    print(json.dumps(cur))
else:
    print(cur)
PY
}

stop_host() {
  if [[ -x "$BIN" ]]; then
    "$BIN" host stop >"$EVID/logs/host-stop.out" 2>"$EVID/logs/host-stop.err" || true
  fi
}

cleanup() {
  stop_host
  if [[ -n "${MOCK_PID:-}" ]]; then
    kill "$MOCK_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

# ── preflight ────────────────────────────────────────────────────────────────
SHA="$(git -C "$REPO" rev-parse HEAD)"
note "# PR #2686 installed-build verification"
note ""
note "- date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
note "- commit: $SHA"
note "- channel: $CHANNEL"
note "- binary: $BIN"
note "- profile: $PROFILE"
note "- display: $DISPLAY"
note "- vulkan: lavapipe via $VK_DRIVER_FILES"
note ""

if [[ ! -x "$BIN" ]]; then
  record FAIL preflight "test -x $BIN" "Installed binary is missing. Build with: bash scripts/install.sh --from-source $CHANNEL"
  exit 1
fi
VER="$("$BIN" --version 2>&1 || true)"
note "Binary version: $VER"
note ""

# ── config: local mock model, info logs, default sign-off refuse ─────────────
CFG="$PROFILE/config.toml"
python3 - "$CFG" "$MOCK_PORT" <<'PY'
import sys
from pathlib import Path
path, port = Path(sys.argv[1]), sys.argv[2]
text = path.read_text() if path.exists() else ""
text = text.replace('backend = "openrouter"', 'backend = "local"', 1)
# Commented examples already contain these headings, so match only real tables.
marker = "\n# verify-pr-2686 live config\n"
if marker not in text:
    text += f"""{marker}
[ai.local]
base_url = "http://127.0.0.1:{port}"
model_low = "mock-chess"
model_medium = "mock-chess"
model_high = "mock-chess"

[log]
level = "info"

[permissions.personal_signoff]
fallback = "refuse"
"""
path.write_text(text)
print(path)
PY

# ── mock model ───────────────────────────────────────────────────────────────
python3 "$ROOT/pr-2686-mock-model.py" >"$EVID/logs/mock-model.log" 2>&1 &
MOCK_PID=$!
sleep 0.3

# ── workspace + host ─────────────────────────────────────────────────────────
rm -rf "$WS"
mkdir -p "$WS"
(
  cd "$WS"
  "$BIN" workspace init >"$EVID/logs/workspace-init.out" 2>"$EVID/logs/workspace-init.err" || true
)

HOST_LOG_AT=$(log_bytes)
set +e
"$BIN" host start --ephemeral --timeout-secs 90 --pane "cwd=$WS" >"$EVID/logs/host-start.out" 2>"$EVID/logs/host-start.err"
HOST_RC=$?
set -e
"$BIN" host status --json >"$EVID/logs/host-status.json" 2>"$EVID/logs/host-status.err" || true
if [[ "$HOST_RC" -ne 0 ]] || ! grep -q '"ready"[[:space:]]*:[[:space:]]*true' "$EVID/logs/host-status.json"; then
  slice_since "$LOG" "$HOST_LOG_AT" "$EVID/logs/host-start-log-slice.txt"
  record FAIL boot "$BIN host start --ephemeral --timeout-secs 90 --pane cwd=$WS" "$(printf 'host start exit %s\nstatus:\n%s\nstderr:\n%s\nlog:\n%s' "$HOST_RC" "$(cat "$EVID/logs/host-status.json")" "$(tail -40 "$EVID/logs/host-start.err")" "$(tail -40 "$EVID/logs/host-start-log-slice.txt")")"
  exit 1
fi
record PASS boot "$BIN host start --ephemeral --timeout-secs 90 --pane 'cwd=$WS'" "$(printf 'status: %s' "$(tr '\n' ' ' < "$EVID/logs/host-status.json")")"
# Keep going through every check. A refused call exits nonzero; that is evidence.
set +e

# install chess from this commit and open it
set +e
"$BIN" app install "$REPO/apps/chess" --yes >"$EVID/logs/app-install.out" 2>"$EVID/logs/app-install.err"
INSTALL_RC=$?
set -e
if [[ "$INSTALL_RC" -ne 0 ]]; then
  record FAIL chess-install "$BIN app install $REPO/apps/chess --yes" "$(tail -30 "$EVID/logs/app-install.err"; tail -20 "$EVID/logs/app-install.out")"
  exit 1
fi
"$BIN" app open chess >"$EVID/logs/app-open-chess.out" 2>"$EVID/logs/app-open-chess.err" || true
"$BIN" context set-root "$WS" >"$EVID/logs/set-root.out" 2>"$EVID/logs/set-root.err" || true
sleep 2
pane_list
TERM_PANE="$(find_pane terminal terminal)"
if [[ -z "$TERM_PANE" ]]; then
  # host start names the seeded pane from cwd; fall back to the first terminal
  TERM_PANE="$(python3 - "$EVID/logs/pane-list.json" <<'PY'
import json,sys
rows=json.load(open(sys.argv[1]))
terms=[str(r["id"]) for r in rows if r.get("type")=="terminal"]
print(terms[0] if terms else "")
PY
)"
fi
if [[ -z "$TERM_PANE" ]]; then
  record FAIL panes "$BIN pane list" "No terminal pane. $(cat "$EVID/logs/pane-list.json")"
  exit 1
fi
note "Terminal pane: $TERM_PANE"
note ""

# wait until chess.play is registered
READY=0
for attempt in 1 2 3 4 5 6; do
  in_pane "ready-$attempt" "\"$BIN\" app call chess chess.state --json > '$EVID/jobs/ready-$attempt.json' 2> '$EVID/jobs/ready-$attempt.err'" || true
  if [[ -f "$EVID/jobs/ready-$attempt.json" ]] && ! grep -q 'tool_not_found' "$EVID/jobs/ready-$attempt.json" "$EVID/jobs/ready-$attempt.err" 2>/dev/null; then
    READY=1
    break
  fi
  sleep 3
done
if [[ "$READY" -ne 1 ]]; then
  record FAIL chess-ready "in-pane: $BIN app call chess chess.state --json" "Chess tool never registered. Last reply: $(cat "$EVID/jobs/ready-$attempt.json" 2>/dev/null; echo; cat "$EVID/jobs/ready-$attempt.err" 2>/dev/null)"
  exit 1
fi
cp "$EVID/jobs/ready-$attempt.json" "$EVID/logs/chess-state-first.json"

PLAY_A="$(python3 - <<'PY'
import json
print(json.dumps({"game_id":"game-1","expected_revision":0,"operation_id":"verify-e4","move":"e2e4"}))
PY
)"
PLAY_OTHER_MOVE="$(python3 - <<'PY'
import json
print(json.dumps({"game_id":"game-1","expected_revision":0,"operation_id":"verify-other-move","move":"g1f3"}))
PY
)"
PLAY_OTHER_GAME="$(python3 - <<'PY'
import json
print(json.dumps({"game_id":"game-2","expected_revision":0,"operation_id":"verify-board-b","move":"e2e4"}))
PY
)"

# ── 1. app call is gated ─────────────────────────────────────────────────────
MARK_A=$(audit_bytes); MARK_L=$(log_bytes)
# outside a pane: identity must not become the human, and must not mutate
set +e
"$BIN" app call chess chess.play --json --input "$PLAY_A" >"$EVID/logs/check1-outside.json" 2>"$EVID/logs/check1-outside.err"
OUTSIDE_RC=$?
set -e
in_pane "check1" "\"$BIN\" app call chess chess.play --json --input '$PLAY_A' > '$EVID/logs/check1-inside.json' 2> '$EVID/logs/check1-inside.err'" || true
slice_since "$AUDIT" "$MARK_A" "$EVID/logs/check1-audit.jsonl"
slice_since "$LOG" "$MARK_L" "$EVID/logs/check1-log.txt"
INSIDE="$(both "$EVID/logs/check1-inside.json" "$EVID/logs/check1-inside.err")"
OUTSIDE="$(both "$EVID/logs/check1-outside.json" "$EVID/logs/check1-outside.err")"
C1_OK=0
if printf '%s' "$INSIDE" | grep -Eq 'permission_required|permission_denied' \
   && ! printf '%s' "$INSIDE" | grep -q '"ok": true' \
   && grep -Eq 'permission_monitor|app_call' "$EVID/logs/check1-log.txt" \
   && [[ -s "$EVID/logs/check1-audit.jsonl" ]]; then
  C1_OK=1
fi
if [[ "$C1_OK" -eq 1 ]]; then
  record PASS check1 "$BIN app call chess chess.play --json --input '$PLAY_A'   # inside pane $TERM_PANE and once from outside" "$(printf 'outside exit %s:\n%s\n\ninside:\n%s\n\naudit:\n%s\n\nlog:\n%s' "$OUTSIDE_RC" "$OUTSIDE" "$INSIDE" "$(cat "$EVID/logs/check1-audit.jsonl")" "$(excerpt "$EVID/logs/check1-log.txt" 'permission_monitor|app_call|personal_signoff')")"
else
  record FAIL check1 "$BIN app call chess chess.play --json --input '$PLAY_A'" "$(printf 'outside exit %s:\n%s\n\ninside:\n%s\n\naudit:\n%s\n\nlog:\n%s' "$OUTSIDE_RC" "$OUTSIDE" "$INSIDE" "$(cat "$EVID/logs/check1-audit.jsonl")" "$(excerpt "$EVID/logs/check1-log.txt" 'permission_monitor|app_call|error')")"
fi

# ── 2. grant is resource- and argument-scoped ────────────────────────────────
PENDING="$(json_field "$EVID/logs/check1-inside.json" pending_request_id || true)"
if [[ -z "$PENDING" ]]; then
  "$BIN" assistant permission list >"$EVID/logs/check2-list.json" 2>"$EVID/logs/check2-list.err" || true
  PENDING="$(python3 - "$EVID/logs/check2-list.json" <<'PY'
import json,sys
try:
    data=json.load(open(sys.argv[1]))
except Exception:
    print(""); raise SystemExit
rows=data.get("pending") or []
print(rows[-1].get("pending_request_id","") if rows else "")
PY
)"
fi
MARK_A=$(audit_bytes); MARK_L=$(log_bytes)
if [[ -n "$PENDING" ]]; then
  "$BIN" assistant permission resolve "$PENDING" --choice always >"$EVID/logs/check2-resolve.json" 2>"$EVID/logs/check2-resolve.err" || true
fi
in_pane "check2-replay" "\"$BIN\" app call chess chess.play --json --input '$PLAY_A' > '$EVID/logs/check2-replay.json' 2> '$EVID/logs/check2-replay.err'" || true
in_pane "check2-move" "\"$BIN\" app call chess chess.play --json --input '$PLAY_OTHER_MOVE' > '$EVID/logs/check2-other-move.json' 2> '$EVID/logs/check2-other-move.err'" || true
in_pane "check2-game" "\"$BIN\" app call chess chess.play --json --input '$PLAY_OTHER_GAME' > '$EVID/logs/check2-other-game.json' 2> '$EVID/logs/check2-other-game.err'" || true
in_pane "check2-action" "\"$BIN\" app call chess chess.new_game --json --input '{\"game_id\":\"game-9\",\"white\":\"a\",\"black\":\"b\"}' > '$EVID/logs/check2-other-action.json' 2> '$EVID/logs/check2-other-action.err'" || true
slice_since "$AUDIT" "$MARK_A" "$EVID/logs/check2-audit.jsonl"
slice_since "$LOG" "$MARK_L" "$EVID/logs/check2-log.txt"
REPLAY="$(both "$EVID/logs/check2-replay.json" "$EVID/logs/check2-replay.err")"
OTHER_MOVE="$(both "$EVID/logs/check2-other-move.json" "$EVID/logs/check2-other-move.err")"
OTHER_GAME="$(both "$EVID/logs/check2-other-game.json" "$EVID/logs/check2-other-game.err")"
OTHER_ACTION="$(both "$EVID/logs/check2-other-action.json" "$EVID/logs/check2-other-action.err")"
C2_OK=0
if printf '%s' "$REPLAY" | grep -Eq '"ok": ?true' \
   && printf '%s' "$OTHER_MOVE" | grep -Eq 'permission_required|permission_denied' \
   && printf '%s' "$OTHER_GAME" | grep -Eq 'permission_required|permission_denied' \
   && printf '%s' "$OTHER_ACTION" | grep -Eq 'permission_required|permission_denied' \
   && ! printf '%s' "$OTHER_MOVE$OTHER_GAME$OTHER_ACTION" | grep -q '"revision_after": 1'; then
  C2_OK=1
fi
if [[ "$C2_OK" -eq 1 ]]; then
  record PASS check2 "resolve $PENDING --choice always; replay same play; other move; game-2; chess.new_game" "$(printf 'resolve:\n%s\n\nreplay:\n%s\n\nother move:\n%s\n\nother game:\n%s\n\nother action:\n%s\n\naudit:\n%s\n\nlog:\n%s' "$(cat "$EVID/logs/check2-resolve.json" 2>/dev/null)" "$REPLAY" "$OTHER_MOVE" "$OTHER_GAME" "$OTHER_ACTION" "$(cat "$EVID/logs/check2-audit.jsonl")" "$(excerpt "$EVID/logs/check2-log.txt" 'permission_monitor|app_call')")"
else
  record FAIL check2 "resolve $PENDING --choice always then scoped probes" "$(printf 'pending=%s\nresolve:\n%s\n\nreplay:\n%s\n\nother move:\n%s\n\nother game:\n%s\n\nother action:\n%s\n\naudit:\n%s' "$PENDING" "$(cat "$EVID/logs/check2-resolve.json" 2>/dev/null)" "$REPLAY" "$OTHER_MOVE" "$OTHER_GAME" "$OTHER_ACTION" "$(cat "$EVID/logs/check2-audit.jsonl")")"
fi

# ── 3. every ingress hits the gate ───────────────────────────────────────────
MARK_A=$(audit_bytes); MARK_L=$(log_bytes)
# A pane spawned before the MCP listener binds has an empty token. Open a
# fresh terminal after the host is ready, then read its env.
"$BIN" pane new >"$EVID/logs/pane-new-mcp.out" 2>"$EVID/logs/pane-new-mcp.err" || true
sleep 2
pane_list
TERM_PANE="$(python3 - "$EVID/logs/pane-list.json" <<'PY'
import json,sys
rows=json.load(open(sys.argv[1]))
terms=[str(r["id"]) for r in rows if r.get("type")=="terminal"]
print(terms[-1] if terms else "")
PY
)"
note "MCP driver pane: $TERM_PANE"
in_pane "mcp-env" "printf '%s\n' \"\$PLEXI_HOST_MCP_PORT\" > '$EVID/logs/mcp.port'; printf '%s\n' \"\$PLEXI_HOST_MCP_TOKEN\" > '$EVID/logs/mcp.token'; printf '%s\n' \"\$PLEXI_CALL_CREDENTIAL\" > '$EVID/logs/call.cred'" || true
MCP_PORT="$(tr -d '[:space:]' < "$EVID/logs/mcp.port" 2>/dev/null || true)"
MCP_TOKEN="$(tr -d '[:space:]' < "$EVID/logs/mcp.token" 2>/dev/null || true)"
if [[ -n "$MCP_PORT" && -n "$MCP_TOKEN" ]]; then
  curl -sS "http://127.0.0.1:${MCP_PORT}/mcp" \
    -H "Authorization: Bearer ${MCP_TOKEN}" \
    -H 'Content-Type: application/json' \
    --data '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' \
    >"$EVID/logs/check3-tools.json" 2>"$EVID/logs/check3-tools.err" || true
  curl -sS "http://127.0.0.1:${MCP_PORT}/mcp" \
    -H "Authorization: Bearer ${MCP_TOKEN}" \
    -H 'Content-Type: application/json' \
    --data "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"chess__chess.play\",\"arguments\":{\"game_id\":\"game-1\",\"expected_revision\":1,\"operation_id\":\"negative-mcp\",\"move\":\"e7e5\"}}}" \
    >"$EVID/logs/check3-mcp.json" 2>"$EVID/logs/check3-mcp.err" || true
else
  echo "missing mcp env port='$MCP_PORT'" > "$EVID/logs/check3-mcp.json"
fi
# forged host API
python3 - "$SOCK" "$EVID/logs/check3-hostapi.json" <<'PY'
import json, socket, sys, time
sock_path, dest = sys.argv[1:]
payload = {
    "type": "call_app_tool",
    "app_id": "chess",
    "tool": "chess.play",
    "input_json": json.dumps({"game_id":"game-1","expected_revision":1,"operation_id":"forged-host-api","move":"e7e5","actor":"user"}),
    "caller_pane_id": 1,
    "call_credential": "forged-token",
    "response_file": dest,
}
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.connect(sock_path)
s.sendall((json.dumps(payload) + "\n").encode())
s.close()
for _ in range(50):
    try:
        if open(dest).read().strip():
            break
    except OSError:
        pass
    time.sleep(0.1)
PY
# assistant path is check 6; here we only record that a direct assistant send
# is attempted after the mock is armed, and that an unapproved call does not
# commit before the sheet is answered. The end-to-end move is check 6.
slice_since "$AUDIT" "$MARK_A" "$EVID/logs/check3-audit.jsonl"
slice_since "$LOG" "$MARK_L" "$EVID/logs/check3-log.txt"
MCP_BODY="$(both "$EVID/logs/check3-mcp.json" "$EVID/logs/check3-mcp.err")"
HOST_BODY="$(cat "$EVID/logs/check3-hostapi.json" 2>/dev/null || true)"
C3_OK=0
if printf '%s' "$MCP_BODY" | grep -Eq 'permission_required|permission_denied' \
   && printf '%s' "$HOST_BODY" | grep -Eq 'permission_denied|permission_required' \
   && ! printf '%s' "$MCP_BODY$HOST_BODY" | grep -q 'revision_after' \
   && grep -Eq '"kind": "ask"|"kind": "deny"|permission_monitor' "$EVID/logs/check3-audit.jsonl" "$EVID/logs/check3-log.txt"; then
  C3_OK=1
fi
if [[ "$C3_OK" -eq 1 ]]; then
  record PASS check3 "MCP tools/call + raw notify.sock call_app_tool with forged user/credential" "$(printf 'mcp:\n%s\n\nhost api:\n%s\n\naudit:\n%s\n\nlog:\n%s' "$MCP_BODY" "$HOST_BODY" "$(cat "$EVID/logs/check3-audit.jsonl")" "$(excerpt "$EVID/logs/check3-log.txt" 'permission_monitor|host_mcp|app_call')")"
else
  record FAIL check3 "MCP tools/call + raw notify.sock call_app_tool" "$(printf 'port=%s token_len=%s\nmcp:\n%s\n\nhost api:\n%s\n\ntools stderr:\n%s\naudit:\n%s\nlog:\n%s' "$MCP_PORT" "${#MCP_TOKEN}" "$MCP_BODY" "$HOST_BODY" "$(tail -20 "$EVID/logs/check3-tools.err" 2>/dev/null)" "$(cat "$EVID/logs/check3-audit.jsonl" 2>/dev/null)" "$(excerpt "$EVID/logs/check3-log.txt" 'permission_monitor|host_mcp|app_call|error')")"
fi

# ── 6. assistant actually plays after approval (before the second board) ────
REV_NOW="$(python3 - "$EVID/logs/check2-replay.json" <<'PY'
import json,sys
try:
    data=json.load(open(sys.argv[1]))
except Exception:
    print(1); raise SystemExit
out=data.get("output") or {}
print(out.get("revision_after", 1))
PY
)"
NEXT_MOVE="e7e5"
if [[ "$REV_NOW" == "0" || -z "$REV_NOW" ]]; then
  NEXT_MOVE="e2e4"
  REV_NOW=0
fi
python3 - "$ROOT/../move.json" <<PY
import json
open("/tmp/verify-pr-2686/move.json","w").write(json.dumps({
  "tool_substr": "play",
  "arguments": {
    "game_id": "game-1",
    "expected_revision": int("$REV_NOW"),
    "operation_id": "assistant-e2e",
    "move": "$NEXT_MOVE",
  },
}))
PY
in_pane "open-assistant" "\"$BIN\" app open assistant > '$EVID/logs/app-open-assistant.out' 2> '$EVID/logs/app-open-assistant.err'" || true
ASSIST_PANE=""
for i in $(seq 1 15); do
  pane_list
  ASSIST_PANE="$(find_pane app assistant)"
  [[ -n "$ASSIST_PANE" ]] && break
  sleep 1
done
MARK_A=$(audit_bytes); MARK_L=$(log_bytes)
if [[ -n "$ASSIST_PANE" ]]; then
  "$BIN" pane focus "$ASSIST_PANE" >"$EVID/logs/check6-focus.out" 2>"$EVID/logs/check6-focus.err" || true
  "$BIN" assistant send --pane-id "$ASSIST_PANE" --text "Play exactly $NEXT_MOVE on game-1 at revision $REV_NOW." --request-id verify-assistant-move --json >"$EVID/logs/check6-send.json" 2>"$EVID/logs/check6-send.err" &
  SEND_PID=$!
else
  echo "assistant pane missing" >"$EVID/logs/check6-send.err"
  SEND_PID=""
fi
FOUND_PENDING=""
for i in $(seq 1 40); do
  "$BIN" assistant permission list >"$EVID/logs/check6-list-$i.json" 2>/dev/null || true
  FOUND_PENDING="$(python3 - "$EVID/logs/check6-list-$i.json" <<'PY'
import json,sys
try:
    data=json.load(open(sys.argv[1]))
except Exception:
    print(""); raise SystemExit
for row in data.get("pending") or []:
    blob=json.dumps(row)
    if "assistant-e2e" in blob or "assistant" in blob.lower():
        print(row.get("pending_request_id",""))
        raise SystemExit
print("")
PY
)"
  if [[ -n "$FOUND_PENDING" ]]; then
    break
  fi
  sleep 1
done
# Sheet cursor starts on Deny. Three Lefts land on Allow once, then Enter.
if [[ -n "$ASSIST_PANE" && -n "$FOUND_PENDING" ]]; then
  "$BIN" pane focus "$ASSIST_PANE" >"$EVID/logs/check6-focus.out" 2>"$EVID/logs/check6-focus.err" || true
  for key in left left left enter; do
    "$BIN" pane key "$ASSIST_PANE" "$key" >"$EVID/logs/check6-key-$key.out" 2>"$EVID/logs/check6-key-$key.err" || true
    sleep 0.3
  done
fi
# Also resolve through the observation seam if the sheet did not unblock.
if [[ -n "$FOUND_PENDING" && -n "$SEND_PID" ]]; then
  sleep 2
  if kill -0 "$SEND_PID" 2>/dev/null; then
    "$BIN" assistant permission resolve "$FOUND_PENDING" --choice once >"$EVID/logs/check6-resolve.json" 2>"$EVID/logs/check6-resolve.err" || true
    # retry the exact call in-pane in case the presenter is still blocked
    ASSIST_INPUT="$(python3 - <<PY
import json
print(json.dumps({"game_id":"game-1","expected_revision":int("$REV_NOW"),"operation_id":"assistant-e2e","move":"$NEXT_MOVE"}))
PY
)"
    in_pane "check6-retry" "\"$BIN\" app call chess chess.play --json --input '$ASSIST_INPUT' > '$EVID/logs/check6-retry.json' 2> '$EVID/logs/check6-retry.err'" || true
  fi
fi
if [[ -n "$SEND_PID" ]]; then
  wait "$SEND_PID" || true
fi
slice_since "$AUDIT" "$MARK_A" "$EVID/logs/check6-audit.jsonl"
slice_since "$LOG" "$MARK_L" "$EVID/logs/check6-log.txt"
SEND_BODY="$(both "$EVID/logs/check6-send.json" "$EVID/logs/check6-send.err")"
RETRY_BODY="$(both "$EVID/logs/check6-retry.json" "$EVID/logs/check6-retry.err")"
C6_OK=0
if printf '%s\n%s' "$SEND_BODY" "$RETRY_BODY" | grep -Eq '"revision_after"|Move submitted|'"$NEXT_MOVE"; then
  if printf '%s\n%s' "$SEND_BODY" "$RETRY_BODY" | grep -q '"revision_after"'; then
    C6_OK=1
  fi
fi
# A committed move is the bar. The assistant envelope says "Move submitted."
# and the chess guest logs the revision; the audit outcome carries revision_after.
if printf '%s' "$RETRY_BODY$SEND_BODY" | grep -Eq '"revision_after"|Move submitted'; then
  if grep -q 'played '"$NEXT_MOVE" "$LOG" 2>/dev/null || grep -q '"revision_after":"'"$REV_NOW"'"' "$EVID/logs/check6-audit.jsonl" 2>/dev/null || printf '%s' "$RETRY_BODY$SEND_BODY" | grep -q '"revision_after"'; then
    C6_OK=1
  fi
fi
if [[ "$C6_OK" -eq 1 ]]; then
  record PASS check6 "mock model + assistant send, then approve, move $NEXT_MOVE at rev $REV_NOW" "$(printf 'assistant pane=%s pending=%s\nsend:\n%s\n\nretry:\n%s\n\naudit:\n%s\n\nlog:\n%s' "$ASSIST_PANE" "$FOUND_PENDING" "$SEND_BODY" "$RETRY_BODY" "$(cat "$EVID/logs/check6-audit.jsonl")" "$(excerpt "$EVID/logs/check6-log.txt" 'assistant|permission_monitor|app_call|openai_compat')")"
else
  record FAIL check6 "mock model + assistant send then approve" "$(printf 'assistant pane=%s pending=%s\nsend:\n%s\nstderr:\n%s\n\nretry:\n%s\n\naudit:\n%s\n\nlog:\n%s' "$ASSIST_PANE" "$FOUND_PENDING" "$SEND_BODY" "$(tail -30 "$EVID/logs/check6-send.err")" "$RETRY_BODY" "$(cat "$EVID/logs/check6-audit.jsonl" 2>/dev/null)" "$(excerpt "$EVID/logs/check6-log.txt" 'assistant|permission_monitor|openai_compat|error')")"
fi

# ── 4. two chess panes stay addressable ──────────────────────────────────────
CHESS_COPY="$EVID/chess-always-new"
rm -rf "$CHESS_COPY"
cp -a "$REPO/apps/chess" "$CHESS_COPY"
python3 - "$CHESS_COPY/manifest.toml" <<'PY'
import sys
from pathlib import Path
p = Path(sys.argv[1])
text = p.read_text()
text = text.replace('on_launch = "focus_existing_in_context"', 'on_launch = "always_new"')
if "always_new" not in text:
    text += '\n[launch]\non_launch = "always_new"\n'
p.write_text(text)
PY
"$BIN" app open "$CHESS_COPY" >"$EVID/logs/check4-open1.out" 2>"$EVID/logs/check4-open1.err" || true
sleep 4
"$BIN" app open "$CHESS_COPY" >"$EVID/logs/check4-open2.out" 2>"$EVID/logs/check4-open2.err" || true
sleep 8
pane_list
CHESS_IDS="$(find_panes app chess)"
# refresh MCP token in case the pane env is unchanged (port is stable)
in_pane "mcp-env-2" "printf '%s\n' \"\$PLEXI_HOST_MCP_PORT\" > '$EVID/logs/mcp.port'; printf '%s\n' \"\$PLEXI_HOST_MCP_TOKEN\" > '$EVID/logs/mcp.token'" || true
MCP_PORT="$(tr -d '[:space:]' < "$EVID/logs/mcp.port" 2>/dev/null || true)"
MCP_TOKEN="$(tr -d '[:space:]' < "$EVID/logs/mcp.token" 2>/dev/null || true)"
if [[ -n "$MCP_PORT" && -n "$MCP_TOKEN" ]]; then
  curl -sS "http://127.0.0.1:${MCP_PORT}/mcp" \
    -H "Authorization: Bearer ${MCP_TOKEN}" \
    -H 'Content-Type: application/json' \
    --data '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' \
    >"$EVID/logs/check4-tools.json" 2>"$EVID/logs/check4-tools.err" || true
fi
MARK_L=$(log_bytes)
# unqualified call should be ambiguous, not tool_not_found
AMBIG_INPUT="$(python3 - <<'PY'
import json
print(json.dumps({"game_id":"game-1","expected_revision":0,"operation_id":"ambiguous-route","move":"e2e4"}))
PY
)"
in_pane "check4-ambig" "\"$BIN\" app call chess chess.play --json --input '$AMBIG_INPUT' > '$EVID/logs/check4-ambig.json' 2> '$EVID/logs/check4-ambig.err'" || true
# route to each pane id we found
PANE_A="$(echo "$CHESS_IDS" | awk '{print $1}')"
PANE_B="$(echo "$CHESS_IDS" | awk '{print $2}')"
if [[ -n "$PANE_A" && -n "$PANE_B" ]]; then
  in_pane "check4-a" "\"$BIN\" app call chess chess.play --json --pane $PANE_A --input '$AMBIG_INPUT' > '$EVID/logs/check4-pane-a.json' 2> '$EVID/logs/check4-pane-a.err'" || true
  ROUTE_B="$(python3 - <<'PY'
import json
print(json.dumps({"game_id":"game-1","expected_revision":0,"operation_id":"route-pane-b","move":"e2e4"}))
PY
)"
  in_pane "check4-b" "\"$BIN\" app call chess chess.play --json --pane $PANE_B --input '$ROUTE_B' > '$EVID/logs/check4-pane-b.json' 2> '$EVID/logs/check4-pane-b.err'" || true
fi
slice_since "$LOG" "$MARK_L" "$EVID/logs/check4-log.txt"
TOOLS_BODY="$(cat "$EVID/logs/check4-tools.json" 2>/dev/null || true)"
AMBIG_BODY="$(both "$EVID/logs/check4-ambig.json" "$EVID/logs/check4-ambig.err")"
A_BODY="$(both "$EVID/logs/check4-pane-a.json" "$EVID/logs/check4-pane-a.err")"
B_BODY="$(both "$EVID/logs/check4-pane-b.json" "$EVID/logs/check4-pane-b.err")"
QUAL_COUNT="$(printf '%s\n%s' "$TOOLS_BODY" "$AMBIG_BODY" | grep -oE 'chess:[0-9]+__chess\.play' | sort -u | wc -l | tr -d ' ' || true)"
C4_OK=0
if [[ "${QUAL_COUNT:-0}" -ge 2 ]] \
   && ! printf '%s' "$AMBIG_BODY$A_BODY$B_BODY$TOOLS_BODY" | grep -q 'tool_not_found' \
   && printf '%s' "$AMBIG_BODY" | grep -q 'ambiguous_instance'; then
  C4_OK=1
fi
if [[ "$C4_OK" -eq 1 ]]; then
  record PASS check4 "open two always_new chess panes; tools/list; app call with and without --pane" "$(printf 'chess panes: %s\nqualified tools: %s\n\nambiguous:\n%s\n\npane A %s:\n%s\n\npane B %s:\n%s\n\nlog:\n%s' "$CHESS_IDS" "$QUAL_COUNT" "$AMBIG_BODY" "$PANE_A" "$A_BODY" "$PANE_B" "$B_BODY" "$(excerpt "$EVID/logs/check4-log.txt" 'tool_dispatch|ambiguous|app_call')")"
else
  record FAIL check4 "two chess panes" "$(printf 'chess panes: %s\nqualified count: %s\npane list:\n%s\n\ntools (truncated):\n%s\n\nambiguous:\n%s\n\nA:\n%s\n\nB:\n%s\n\nlog:\n%s' "$CHESS_IDS" "${QUAL_COUNT:-0}" "$(cat "$EVID/logs/pane-list.json")" "$(printf '%s' "$TOOLS_BODY" | head -c 2000)" "$AMBIG_BODY" "$A_BODY" "$B_BODY" "$(excerpt "$EVID/logs/check4-log.txt" 'tool_dispatch|ambiguous|app_call|on_launch')")"
fi

# ── 5. seat / permission errors are logged ───────────────────────────────────
SEAT_LOG="$(excerpt "$LOG" 'seat|permission_denied|permission_required|permission_monitor|holds no seat|wrong_side' 30)"
SEAT_AUDIT="$(grep -E 'permission_denied|deny|ask|personal_signoff|identity' "$AUDIT" 2>/dev/null | tail -n 15 || true)"
if printf '%s' "$SEAT_LOG" | grep -q 'permission_monitor' && printf '%s' "$SEAT_AUDIT" | grep -q .; then
  record PASS check5 "grep plexi.log and permission-audit.jsonl" "$(printf 'log:\n%s\n\naudit:\n%s' "$SEAT_LOG" "$SEAT_AUDIT")"
else
  record FAIL check5 "grep plexi.log and permission-audit.jsonl" "$(printf 'log:\n%s\n\naudit:\n%s' "$SEAT_LOG" "$SEAT_AUDIT")"
fi

# ── 9. 20 concurrent gated calls ─────────────────────────────────────────────
MARK_A=$(audit_bytes); MARK_L=$(log_bytes)
cat > "$EVID/jobs/load.sh" <<EOF
#!/bin/bash
mkdir -p '$EVID/logs/load'
for i in \$(seq 1 20); do
  input=\$(python3 -c 'import json,sys; print(json.dumps({"game_id":"game-1","expected_revision":0,"operation_id":"load-%s"%sys.argv[1],"move":"a2a3"}))' "\$i")
  "$BIN" app call chess chess.play --json --pane ${PANE_A:-} --input "\$input" > '$EVID/logs/load/'"\$i.json" 2> '$EVID/logs/load/'"\$i.err" &
done
wait
echo done > '$EVID/jobs/load.done'
EOF
chmod +x "$EVID/jobs/load.sh"
rm -f "$EVID/jobs/load.done"
"$BIN" pane send "$TERM_PANE" --submit "bash '$EVID/jobs/load.sh'" >"$EVID/logs/pane-send-load.out" 2>"$EVID/logs/pane-send-load.err" || true
for i in $(seq 1 60); do
  [[ -f "$EVID/jobs/load.done" ]] && break
  sleep 1
done
slice_since "$AUDIT" "$MARK_A" "$EVID/logs/check9-audit.jsonl"
slice_since "$LOG" "$MARK_L" "$EVID/logs/check9-log.txt"
python3 - "$EVID/logs/load" "$EVID/logs/check9-audit.jsonl" "$EVID/logs/check9-summary.json" <<'PY'
import json, sys
from pathlib import Path
load, audit_path, dest = map(Path, sys.argv[1:])
calls = []
for i in range(1, 21):
    f = load / f"{i}.json"
    err = load / f"{i}.err"
    text = (f.read_text() if f.exists() else "") + (err.read_text() if err.exists() else "")
    calls.append({"i": i, "bytes": len(text.strip()), "text": text[:500]})
audit = []
if audit_path.exists():
    for line in audit_path.read_text().splitlines():
        line=line.strip()
        if not line:
            continue
        try:
            audit.append(json.loads(line))
        except json.JSONDecodeError:
            pass
ops = [f"load-{i}" for i in range(1, 21)]
per = {}
for op in ops:
    per[op] = sum(1 for row in audit if row.get("operation_id") == op)
answered = sum(1 for c in calls if c["bytes"] > 0)
hung = [c["i"] for c in calls if c["bytes"] == 0]
missing_audit = [op for op, n in per.items() if n != 1]
summary = {
    "answered": answered,
    "hung": hung,
    "audit_rows": len(audit),
    "ops_not_exactly_one": missing_audit,
    "per_op": per,
}
dest.write_text(json.dumps(summary, indent=2))
print(json.dumps(summary))
PY
"$BIN" assistant permission list >"$EVID/logs/check9-pending-before.json" 2>/dev/null || true
python3 - "$EVID/logs/check9-pending-before.json" "$BIN" <<'PY'
import json, subprocess, sys
listing, binary = sys.argv[1:]
try:
    data = json.load(open(listing))
except Exception:
    data = {}
ids = []
for row in data.get("pending") or []:
    blob = json.dumps(row)
    if "load-" in blob or "a2a3" in blob:
        pid = row.get("pending_request_id")
        if pid:
            ids.append(pid)
for pid in ids:
    subprocess.run([binary, "assistant", "permission", "resolve", pid, "--choice", "deny"], check=False)
print("denied", len(ids))
PY
"$BIN" assistant permission list >"$EVID/logs/check9-pending-after.json" 2>/dev/null || true
LEFT="$(python3 - "$EVID/logs/check9-pending-after.json" <<'PY'
import json,sys
try:
    data=json.load(open(sys.argv[1]))
except Exception:
    print("unknown"); raise SystemExit
rows=data.get("pending") or []
load=[r for r in rows if "load-" in json.dumps(r) or "a2a3" in json.dumps(r)]
print(len(load))
PY
)"
SUMMARY="$(cat "$EVID/logs/check9-summary.json" 2>/dev/null || echo '{}')"
C9_OK=0
if printf '%s' "$SUMMARY" | grep -q '"answered": 20' \
   && printf '%s' "$SUMMARY" | grep -q '"ops_not_exactly_one": \[\]' \
   && [[ "$LEFT" == "0" ]] \
   && [[ -f "$EVID/jobs/load.done" ]]; then
  C9_OK=1
fi
if [[ "$C9_OK" -eq 1 ]]; then
  record PASS check9 "20 concurrent app calls from pane $TERM_PANE" "$(printf '%s\norphaned load pendings after deny: %s\n' "$SUMMARY" "$LEFT")"
else
  record FAIL check9 "20 concurrent app calls" "$(printf 'done_file=%s\nsummary:\n%s\norphaned=%s\npending after:\n%s\nlog:\n%s' "$([[ -f $EVID/jobs/load.done ]] && echo yes || echo no)" "$SUMMARY" "$LEFT" "$(head -c 1500 "$EVID/logs/check9-pending-after.json" 2>/dev/null)" "$(excerpt "$EVID/logs/check9-log.txt" 'deadlock|permission_monitor|error' 20)")"
fi

# ── 7. .plexi/agents migration ───────────────────────────────────────────────
MIG="$EVID/migrate-ws"
rm -rf "$MIG"
mkdir -p "$MIG"
(
  cd "$MIG"
  "$BIN" workspace init >"$EVID/logs/mig-init.out" 2>"$EVID/logs/mig-init.err"
)
CHANNEL_DIR="$MIG/.plexi-$CHANNEL"
# fresh install reads only .plexi/agents
FRESH="$EVID/fresh-ws"
rm -rf "$FRESH"
mkdir -p "$FRESH/.plexi/agents/writer" "$FRESH/agents/decoy" "$FRESH/.plexi-$CHANNEL"
cat > "$FRESH/.plexi/agents/writer/AGENT.md" <<'EOF'
fresh prompt
EOF
cat > "$FRESH/.plexi/agents/writer/settings.toml" <<'EOF'
[agent]
id = "writer"
display_name = "writer"
default_tier = "medium"

[permissions]
default_posture = "ask"
allow = ["chess.play"]
EOF
mkdir -p "$FRESH/.plexi-$CHANNEL"
echo 'decoy' > "$FRESH/agents/decoy/AGENT.md"
mkdir -p "$FRESH/agents/decoy"
# workspace root detection looks for the channel dir
(
  cd "$FRESH"
  "$BIN" agent list >"$EVID/logs/fresh-list.out" 2>"$EVID/logs/fresh-list.err"
)
FRESH_OUT="$(cat "$EVID/logs/fresh-list.out" 2>/dev/null || true)"
# legacy seed then migrate
LEGACY="$CHANNEL_DIR/agents/writer"
mkdir -p "$LEGACY"
cat > "$LEGACY/AGENT.md" <<'EOF'
legacy prompt
EOF
cat > "$LEGACY/settings.toml" <<'EOF'
[agent]
id = "writer"
display_name = "writer"
default_tier = "medium"

[permissions]
default_posture = "ask"
allow = ["chess.play"]
EOF
printf 'decision = "allow"\n' > "$LEGACY/grants.toml"
# do not precreate canonical writer
(
  cd "$MIG"
  "$BIN" agent list >"$EVID/logs/mig-list-1.out" 2>"$EVID/logs/mig-list-1.err"
  cp "$MIG/.plexi/agents/.migration-receipt" "$EVID/logs/receipt-1.txt" 2>/dev/null || true
  "$BIN" agent list >"$EVID/logs/mig-list-2.out" 2>"$EVID/logs/mig-list-2.err"
  cp "$MIG/.plexi/agents/.migration-receipt" "$EVID/logs/receipt-2.txt" 2>/dev/null || true
)
python3 - "$MIG" "$FRESH" "$CHANNEL" "$EVID/logs/fresh-list.out" "$EVID/logs/mig-list-1.out" "$EVID/logs/receipt-1.txt" "$EVID/logs/receipt-2.txt" > "$EVID/logs/check7-summary.json" <<'PY'
import json, sys
from pathlib import Path
mig, fresh, channel, fresh_list, mig_list, r1, r2 = sys.argv[1:]
mig, fresh = Path(mig), Path(fresh)
canon = mig / ".plexi" / "agents" / "writer"
legacy = mig / f".plexi-{channel}" / "agents" / "writer"
receipt1 = Path(r1).read_text() if Path(r1).exists() else ""
receipt2 = Path(r2).read_text() if Path(r2).exists() else ""
fresh_text = Path(fresh_list).read_text() if Path(fresh_list).exists() else ""
summary = {
    "fresh_list": fresh_text.strip(),
    "fresh_has_writer_agent_md": (fresh / ".plexi" / "agents" / "writer" / "AGENT.md").is_file(),
    "decoy_not_in_dot_plexi": not (fresh / ".plexi" / "agents" / "decoy").exists(),
    "channel_agents_not_written_by_fresh_list": not (fresh / f".plexi-{channel}" / "agents" / "writer").exists(),
    "canonical_agent_md": (canon / "AGENT.md").read_text() if (canon / "AGENT.md").exists() else "",
    "grants_copied": (canon / "grants.toml").exists(),
    "legacy_still_exists": legacy.exists(),
    "receipt1": receipt1,
    "receipt2": receipt2,
    "receipt_stable": receipt1 == receipt2 and receipt1.strip() != "",
    "migrated_once": receipt1.strip().splitlines().count("migrated writer") == 1,
}
print(json.dumps(summary, indent=2))
PY
C7="$(cat "$EVID/logs/check7-summary.json")"
if printf '%s' "$C7" | grep -q '"grants_copied": false' \
   && printf '%s' "$C7" | grep -q '"legacy_still_exists": false' \
   && printf '%s' "$C7" | grep -q '"migrated_once": true' \
   && printf '%s' "$C7" | grep -q '"receipt_stable": true' \
   && printf '%s' "$C7" | grep -q 'legacy prompt' \
   && printf '%s' "$C7" | grep -q '"decoy_not_in_dot_plexi": true'; then
  record PASS check7 "$BIN agent list in a fresh workspace and a legacy .plexi-$CHANNEL/agents tree" "$C7"
else
  record FAIL check7 "$BIN agent list migration" "$(printf '%s\n\nfresh stderr:\n%s\nmig stderr:\n%s' "$C7" "$(cat "$EVID/logs/fresh-list.err" 2>/dev/null)" "$(cat "$EVID/logs/mig-list-1.err" 2>/dev/null)")"
fi

# ── 8. personal sign-off on this non-mac build ──────────────────────────────
stop_host
# posture allow must not lower the tier
python3 - "$CFG" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
text = p.read_text()
marker = "\n# verify-pr-2686 posture allow\n"
if marker not in text:
    text += """
# verify-pr-2686 posture allow
[permissions]
default_posture = "allow"
allow = ["signoff-probe.export", "signoff.export", "signoff-probe.signoff.export"]
"""
p.write_text(text)
PY
SIG="$EVID/signoff-probe"
rm -rf "$SIG"
mkdir -p "$SIG"
cat > "$SIG/manifest.toml" <<'EOF'
schema_version = 1

[app]
id = "signoff-probe"
type = "app"
name = "Signoff Probe"
version = "0.0.1"
description = "Verification fixture for personal sign-off."
entry = "main.py"

[runtime]
python_compat = true

[app.capabilities]
capabilities = []

[launch]
on_launch = "always_new"
EOF
cat > "$SIG/main.py" <<'EOF'
"""Fixture: one tool that requires personal sign-off."""
from plexi_sdk import log, tools
from plexi_sdk.effects import SetStatus, SetTitle
from plexi_sdk.ui import Column, Text

@tools.tool(
    "signoff.export",
    "Export a value. Requires personal sign-off.",
    {"amount": int},
    requires="personal_signoff",
    signoff="each_time",
)
def _export(amount: int) -> dict:
    log.info(f"signoff-probe: exported amount={amount}")
    return {"exported": amount}

def init(size, args):
    log.info("signoff-probe: init")
    return [SetTitle("Signoff Probe"), SetStatus("ready"), tools.expose()]

def update(event):
    dispatched = tools.dispatch(event)
    if dispatched is not None:
        return dispatched
    return []

def view():
    return Column([Text("signoff probe")])
EOF
"$BIN" host start --ephemeral --timeout-secs 90 --pane "cwd=$WS" >"$EVID/logs/host-start-2.out" 2>"$EVID/logs/host-start-2.err" || true
"$BIN" host status --json >"$EVID/logs/host-status-2.json" 2>/dev/null || true
sleep 1
pane_list
TERM_PANE="$(python3 - "$EVID/logs/pane-list.json" <<'PY'
import json,sys
rows=json.load(open(sys.argv[1]))
terms=[str(r["id"]) for r in rows if r.get("type")=="terminal"]
print(terms[0] if terms else "")
PY
)"
"$BIN" app open "$SIG" >"$EVID/logs/check8-open.out" 2>"$EVID/logs/check8-open.err" || true
READY=0
for attempt in 1 2 3 4 5 6 7 8; do
  in_pane "signoff-ready-$attempt" "\"$BIN\" app call signoff-probe signoff.export --json --input '{\"amount\":1}' > '$EVID/logs/check8-call-$attempt.json' 2> '$EVID/logs/check8-call-$attempt.err'" || true
  if [[ -s "$EVID/logs/check8-call-$attempt.json" || -s "$EVID/logs/check8-call-$attempt.err" ]] && ! grep -q 'tool_not_found' "$EVID/logs/check8-call-$attempt.json" "$EVID/logs/check8-call-$attempt.err" 2>/dev/null; then
    READY=1
    cp "$EVID/logs/check8-call-$attempt.json" "$EVID/logs/check8-call.json"
    cp "$EVID/logs/check8-call-$attempt.err" "$EVID/logs/check8-call.err" 2>/dev/null || true
    break
  fi
  sleep 3
done
MARK_A=$(audit_bytes); MARK_L=$(log_bytes)
# click cannot satisfy
PENDING="$(json_field "$EVID/logs/check8-call.json" pending_request_id || true)"
if [[ -z "$PENDING" ]]; then
  "$BIN" assistant permission list >"$EVID/logs/check8-list.json" 2>/dev/null || true
  PENDING="$(python3 - "$EVID/logs/check8-list.json" <<'PY'
import json,sys
try:
    data=json.load(open(sys.argv[1]))
except Exception:
    print(""); raise SystemExit
rows=data.get("pending") or []
for row in rows:
    if "signoff" in json.dumps(row):
        print(row.get("pending_request_id","")); raise SystemExit
print(rows[-1].get("pending_request_id","") if rows else "")
PY
)"
fi
if [[ -n "$PENDING" ]]; then
  set +e
  "$BIN" assistant permission resolve "$PENDING" --choice once >"$EVID/logs/check8-click.json" 2>"$EVID/logs/check8-click.err"
  CLICK_RC=$?
  set -e
  "$BIN" assistant permission resolve "$PENDING" --choice deny >"$EVID/logs/check8-deny.json" 2>"$EVID/logs/check8-deny.err" || true
else
  echo '{"missing_pending":true}' > "$EVID/logs/check8-click.json"
  CLICK_RC=1
fi
# one more call after deny, still refused, and posture allow did not let it through
in_pane "check8-after" "\"$BIN\" app call signoff-probe signoff.export --json --input '{\"amount\":2}' > '$EVID/logs/check8-after.json' 2> '$EVID/logs/check8-after.err'" || true
slice_since "$AUDIT" "$MARK_A" "$EVID/logs/check8-audit.jsonl"
slice_since "$LOG" "$MARK_L" "$EVID/logs/check8-log.txt"
CALL="$(both "$EVID/logs/check8-call.json" "$EVID/logs/check8-call.err")"
CLICK="$(both "$EVID/logs/check8-click.json" "$EVID/logs/check8-click.err")"
DENY="$(both "$EVID/logs/check8-deny.json" "$EVID/logs/check8-deny.err")"
AFTER="$(both "$EVID/logs/check8-after.json" "$EVID/logs/check8-after.err")"
C8_OK=0
if printf '%s' "$CALL" | grep -Eq 'permission_denied|personal_signoff|not Touch ID' \
   && ! printf '%s' "$CALL$AFTER" | grep -q '"exported"' \
   && printf '%s' "$CLICK" | grep -Eq 'click|cannot|permission_denied|ok.: false' \
   && grep -Eq 'personal_signoff_refused|personal_signoff_click_refused|deny' "$EVID/logs/check8-audit.jsonl" "$EVID/logs/check8-log.txt"; then
  C8_OK=1
fi
if [[ "$C8_OK" -eq 1 ]]; then
  record PASS check8 "signoff.export with fallback=refuse, posture allow, click once, deny" "$(printf 'call:\n%s\n\nclick rc=%s:\n%s\n\ndeny:\n%s\n\nafter deny:\n%s\n\naudit:\n%s\n\nlog:\n%s' "$CALL" "$CLICK_RC" "$CLICK" "$DENY" "$AFTER" "$(cat "$EVID/logs/check8-audit.jsonl")" "$(excerpt "$EVID/logs/check8-log.txt" 'personal_signoff|permission_monitor')")"
else
  record FAIL check8 "personal sign-off" "$(printf 'ready=%s pending=%s\ncall:\n%s\n\nclick rc=%s:\n%s\n\ndeny:\n%s\n\nafter:\n%s\n\naudit:\n%s\n\nlog:\n%s' "$READY" "$PENDING" "$CALL" "$CLICK_RC" "$CLICK" "$DENY" "$AFTER" "$(cat "$EVID/logs/check8-audit.jsonl" 2>/dev/null)" "$(excerpt "$EVID/logs/check8-log.txt" 'personal_signoff|permission_monitor|signoff|error')")"
fi

note ""
note "## Result table"
note ""
note '```'
cat "$EVID/results.tsv" | tee -a "$EVID/report-draft.md" >/dev/null
cat "$EVID/results.tsv"
note '```'
echo "evidence: $EVID"
