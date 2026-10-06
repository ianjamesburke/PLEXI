#!/usr/bin/env bash
# Installed-binary check: create a click approval, list it, resolve it, and
# confirm the tool proceeds with an audit row.
set -euo pipefail

# The journal HMAC is the host seal key. A private session bus is where that
# key is stored on Linux. Without it the queue file is never written.
if [[ "$(uname -s)" == "Linux" && -z "${NEEDS_YOU_E2E_INNER:-}" ]] && command -v dbus-run-session >/dev/null 2>&1; then
  exec dbus-run-session -- env NEEDS_YOU_E2E_INNER=1 "$0" "$@"
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${PLEXI_BIN:-$ROOT/target/release/plexi}"
if [[ ! -x "$BIN" ]]; then
  echo "error: missing binary $BIN (run just build first)" >&2
  exit 1
fi

WORK="$(mktemp -d)"
trap 'if [[ -n "${HOST_PID:-}" ]]; then kill "$HOST_PID" 2>/dev/null || true; wait "$HOST_PID" 2>/dev/null || true; fi; rm -rf "$WORK"' EXIT

ORIG_HOME="${HOME}"
export HOME="$WORK/home"
mkdir -p "$HOME/.plexi"
if [[ -d "$ORIG_HOME/.plexi/wasm-bundles" ]]; then
  ln -s "$ORIG_HOME/.plexi/wasm-bundles" "$HOME/.plexi/wasm-bundles"
fi
export PLEXI_CHANNEL="needs-you-e2e"
unset PLEXI_SOCKET PLEXI_PANE_ID PLEXI_CONTEXT_ID PLEXI_CONTEXT_ROOT PLEXI_RUNNING || true
export VK_DRIVER_FILES="${VK_DRIVER_FILES:-/usr/share/vulkan/icd.d/lvp_icd.json}"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp/runtime-ubuntu}"
mkdir -p "$XDG_RUNTIME_DIR"

# A channel-named binary (`plexi-pr-N`) ignores PLEXI_CHANNEL and uses its own
# profile. The bare binary follows PLEXI_CHANNEL.
BIN_NAME="$(basename "$BIN")"
if [[ "$BIN_NAME" == plexi-* ]]; then
  PROFILE="$HOME/.plexi-${BIN_NAME#plexi-}"
else
  PROFILE="$HOME/.plexi-$PLEXI_CHANNEL"
fi
SOCKET="$PROFILE/notify.sock"
INPUT='{"game_id":"game-1","expected_revision":0,"operation_id":"op-needs-you","move":"e2e4"}'

start_host() {
  # A stale DISPLAY (common on this VM) makes winit fail before the socket exists.
  unset DISPLAY
  if command -v xvfb-run >/dev/null 2>&1; then
    xvfb-run -a "$BIN" >"$WORK/host.log" 2>&1 &
  else
    "$BIN" >"$WORK/host.log" 2>&1 &
  fi
  HOST_PID=$!
}

echo "starting host $BIN"
start_host
for _ in $(seq 1 90); do
  if [[ -S "$SOCKET" ]]; then
    break
  fi
  if ! kill -0 "$HOST_PID" 2>/dev/null; then
    echo "error: host exited before the socket appeared" >&2
    cat "$WORK/host.log" >&2 || true
    exit 1
  fi
  sleep 1
done
if [[ ! -S "$SOCKET" ]]; then
  echo "error: notify socket did not appear at $SOCKET" >&2
  cat "$WORK/host.log" >&2 || true
  exit 1
fi
export PLEXI_SOCKET="$SOCKET"

echo "opening chess"
"$BIN" app open "$ROOT/apps/chess" >"$WORK/open.log" 2>&1 || {
  echo "error: app open failed" >&2
  cat "$WORK/open.log" >&2
  exit 1
}
ready=0
for _ in $(seq 1 40); do
  if "$BIN" pane list >"$WORK/panes.txt" 2>"$WORK/panes.err" && grep -q . "$WORK/panes.txt"; then
    ready=1
    break
  fi
  sleep 1
done
if [[ "$ready" != 1 ]]; then
  echo "error: chess pane did not appear" >&2
  cat "$WORK/open.log" "$WORK/panes.err" >&2 || true
  exit 1
fi

PANE="$(python3 - "$WORK/panes.txt" <<'PY'
import json, sys
panes = json.load(open(sys.argv[1]))
terms = [pane for pane in panes if pane.get("type") == "terminal"]
chosen = terms[0] if terms else panes[0]
print(chosen["id"])
PY
)"
echo "terminal pane $PANE"

run_in_pane() {
  local outfile="$1"
  local cmd="$2"
  rm -f "$outfile"
  "$BIN" pane command "$PANE" "$cmd" --enter >"$WORK/enter.log" 2>&1
  for _ in $(seq 1 30); do
    if [[ -s "$outfile" ]]; then
      return 0
    fi
    sleep 1
  done
  echo "error: pane command produced no output" >&2
  cat "$WORK/enter.log" >&2 || true
  return 1
}

echo "creating approval"
asked=0
for _ in $(seq 1 40); do
  if ! run_in_pane "$WORK/call1.out" "$BIN app call chess chess.play --json --input '$INPUT' > '$WORK/call1.out' 2>&1"; then
    sleep 1
    continue
  fi
  if python3 - "$WORK/call1.out" <<'PY'
import json, sys
text = open(sys.argv[1]).read()
start = text.find("{")
if start < 0:
    raise SystemExit(1)
body = json.loads(text[start:])
err = body.get("error") if isinstance(body.get("error"), dict) else {}
code = body.get("error_code") or err.get("code")
pending = body.get("pending_request_id") or err.get("pending_request_id")
if code == "permission_required" and pending:
    open(sys.argv[1] + ".id", "w").write(pending)
    raise SystemExit(0)
raise SystemExit(1)
PY
  then
    asked=1
    break
  fi
  sleep 1
done
if [[ "$asked" != 1 ]]; then
  echo "error: chess.play did not ask for approval" >&2
  cat "$WORK/call1.out" >&2 || true
  exit 1
fi
ID="$(cat "$WORK/call1.out.id")"
echo "pending $ID"

echo "listing needs-you"
LIST="$("$BIN" needs-you list --json)"
printf '%s\n' "$LIST" >"$WORK/list.json"
python3 - "$WORK/list.json" "$ID" <<'PY'
import json, sys
body = json.load(open(sys.argv[1]))
ids = [item["id"] for item in body.get("items", [])]
if sys.argv[2] not in ids:
    raise SystemExit(f"{sys.argv[2]} not in {body}")
item = next(item for item in body["items"] if item["id"] == sys.argv[2])
if item.get("kind") != "approval_click" or item.get("resolution") is not None:
    raise SystemExit(f"unexpected row {item}")
PY

echo "terminal resolve must not grant"
set +e
RESOLVE="$("$BIN" needs-you resolve "$ID" --approve 2>"$WORK/resolve.err")"
RESOLVE_CODE=$?
set -e
printf '%s\n' "$RESOLVE" >"$WORK/resolve.json"
python3 - "$WORK/resolve.json" "$RESOLVE_CODE" "$WORK/resolve.err" <<'PY'
import json, sys
text = open(sys.argv[1]).read()
code = int(sys.argv[2])
err = open(sys.argv[3]).read()
body = {}
start = text.find("{")
if start >= 0:
    try:
        body = json.loads(text[start:])
    except json.JSONDecodeError:
        body = {}
granted = code == 0 and body.get("ok") is True and body.get("resolution") == "approved"
if granted:
    raise SystemExit(f"terminal resolve granted: {text} {err}")
print("terminal resolve did not grant")
PY

echo "item still open"
LIST2="$("$BIN" needs-you list --json)"
python3 - "$LIST2" "$ID" <<'PY'
import json, sys
body = json.loads(sys.argv[1])
item = next((row for row in body.get("items", []) if row.get("id") == sys.argv[2]), None)
if item is None or item.get("resolution") is not None:
    raise SystemExit(f"pending item was settled from the terminal: {body}")
PY

echo "needs-you e2e passed"
