#!/usr/bin/env bash
# Installed-binary check: create a click approval, prove a terminal approve
# does not grant, click Allow once with HUMAN_APPROVE, and confirm the tool proceeds.
# The grant is a real XTEST click. This script never treats needs-you resolve as a grant.
set -euo pipefail

# A private session bus is where the unlocked login keyring lives. A nested
# bus cannot see that daemon, so this re-exec is the only session the script
# starts. The acceptance harness replaces dbus-run-session with a passthrough
# once its own preflight has unlocked the keyring.
if [[ "$(uname -s)" == "Linux" && -z "${NEEDS_YOU_E2E_INNER:-}" ]] && command -v dbus-run-session >/dev/null 2>&1; then
  exec dbus-run-session -- env NEEDS_YOU_E2E_INNER=1 "$0" "$@"
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

WORK="$(mktemp -d)"
trap 'if [[ -n "${HOST_PID:-}" ]]; then kill "$HOST_PID" 2>/dev/null || true; wait "$HOST_PID" 2>/dev/null || true; fi; if [[ -n "${XVFB_PID:-}" ]]; then kill "$XVFB_PID" 2>/dev/null || true; wait "$XVFB_PID" 2>/dev/null || true; fi; if [[ -n "${KEYRING_PID:-}" ]]; then kill "$KEYRING_PID" 2>/dev/null || true; fi; rm -rf "$WORK"' EXIT

ORIG_HOME="${HOME}"
export HOME="$WORK/home"
mkdir -p "$HOME/.plexi"
if [[ -d "$ORIG_HOME/.plexi/wasm-bundles" ]]; then
  ln -s "$ORIG_HOME/.plexi/wasm-bundles" "$HOME/.plexi/wasm-bundles"
fi
unset PLEXI_SOCKET PLEXI_PANE_ID PLEXI_CONTEXT_ID PLEXI_CONTEXT_ROOT PLEXI_RUNNING || true
export VK_DRIVER_FILES="${VK_DRIVER_FILES:-/usr/share/vulkan/icd.d/lvp_icd.json}"
export XDG_RUNTIME_DIR="$WORK/runtime"
mkdir -p "$XDG_RUNTIME_DIR/keyring"
chmod 700 "$XDG_RUNTIME_DIR" "$XDG_RUNTIME_DIR/keyring"

if [[ -n "${PLEXI_E2E_SHIM:-}" ]]; then
  # shellcheck disable=SC1090
  source "$PLEXI_E2E_SHIM"
fi
if [[ -z "${BIN:-}" ]]; then
  BIN="${PLEXI_BIN:-$ROOT/target/release/plexi}"
fi
if [[ ! -x "$BIN" ]]; then
  echo "error: missing binary $BIN (run just build first)" >&2
  exit 1
fi

# A channel-named binary (`plexi-alpha`, `plexi-pr-N`) ignores PLEXI_CHANNEL.
# The shim already set PROFILE from that name. Otherwise derive it here.
if [[ -z "${PROFILE:-}" ]]; then
  BIN_NAME="$(basename "$BIN")"
  BIN_NAME="${BIN_NAME%.exe}"
  if [[ "$BIN_NAME" == plexi-* ]]; then
    unset PLEXI_CHANNEL || true
    PROFILE="$HOME/.plexi-${BIN_NAME#plexi-}"
  else
    export PLEXI_CHANNEL="${PLEXI_CHANNEL:-needs-you-e2e}"
    PROFILE="$HOME/.plexi-$PLEXI_CHANNEL"
  fi
fi
SOCKET="$PROFILE/notify.sock"
INPUT='{"game_id":"game-1","expected_revision":0,"operation_id":"op-needs-you","move":"e2e4"}'
INPUT2='{"game_id":"game-1","expected_revision":1,"operation_id":"op-needs-you-2","move":"e7e5"}'

unlock_keyring() {
  if ! command -v gnome-keyring-daemon >/dev/null 2>&1 || ! command -v dbus-launch >/dev/null 2>&1; then
    echo "error: gnome-keyring or dbus-launch is not installed" >&2
    exit 1
  fi
  if ! python3 -c 'import dbus' >/dev/null 2>&1 || ! command -v secret-tool >/dev/null 2>&1; then
    echo "error: python3-dbus or secret-tool is not installed" >&2
    exit 1
  fi
  if [[ -z "${DBUS_SESSION_BUS_ADDRESS:-}" ]]; then
    local launch
    launch="$(dbus-launch --sh-syntax 2>"$WORK/dbus.log")" || {
      echo "error: dbus-launch failed" >&2
      cat "$WORK/dbus.log" >&2 || true
      exit 1
    }
    # shellcheck disable=SC1090
    eval "$launch"
    export DBUS_SESSION_BUS_ADDRESS
  fi
  local pass
  pass="$(openssl rand -hex 16 2>/dev/null || python3 -c 'import secrets; print(secrets.token_hex(16))')"
  printf '%s' "$pass" | gnome-keyring-daemon --unlock --components=secrets --daemonize >"$WORK/keyring.log" 2>&1 || true
  KEYRING_PID="$(pgrep -u "$(id -u)" -n -f '/usr/bin/gnome-keyring-daemon' || true)"
  local attempt code=1
  for attempt in 1 2 3 4 5 6 7 8; do
    sleep 0.3
    if python3 - >"$WORK/keyring-verify.txt" 2>>"$WORK/keyring.log" <<'PY'
import dbus, sys
bus = dbus.SessionBus()
proxy = bus.get_object("org.freedesktop.secrets", "/org/freedesktop/secrets")
svc = dbus.Interface(proxy, "org.freedesktop.Secret.Service")
default = str(svc.ReadAlias("default"))
print(f"default={default}")
if default in ("", "/"):
    sys.exit(2)
coll = bus.get_object("org.freedesktop.secrets", default)
locked = dbus.Interface(coll, "org.freedesktop.DBus.Properties").Get(
    "org.freedesktop.Secret.Collection", "Locked"
)
print(f"locked={int(bool(locked))}")
if int(bool(locked)) != 0:
    sys.exit(3)
PY
    then
      code=0
      break
    fi
  done
  if [[ "$code" -ne 0 ]]; then
    echo "error: gnome-keyring did not unlock a login collection" >&2
    cat "$WORK/keyring.log" "$WORK/keyring-verify.txt" >&2 || true
    exit 1
  fi
  if ! printf 'probe-value' | secret-tool store --label='needs-you-e2e-probe' service needs-you-e2e account probe >>"$WORK/keyring.log" 2>&1; then
    echo "error: secret-tool store failed on the unlocked keyring" >&2
    exit 1
  fi
  local got
  got="$(secret-tool lookup service needs-you-e2e account probe 2>>"$WORK/keyring.log" || true)"
  secret-tool clear service needs-you-e2e account probe >>"$WORK/keyring.log" 2>&1 || true
  if [[ "$got" != "probe-value" ]]; then
    echo "error: secret-tool lookup did not return the probe" >&2
    exit 1
  fi
  echo "secret service unlocked"
}

unlock_keyring

# Parent and host share one X server so xdotool can click Allow once.
display_n=$((80 + RANDOM % 40))
export DISPLAY=":$display_n"
Xvfb "$DISPLAY" -screen 0 1280x800x24 >"$WORK/xvfb.log" 2>&1 &
XVFB_PID=$!
for _ in $(seq 1 50); do
  if xdpyinfo -display "$DISPLAY" >/dev/null 2>&1; then
    break
  fi
  sleep 0.1
done
if ! xdpyinfo -display "$DISPLAY" >/dev/null 2>&1; then
  echo "error: Xvfb did not become ready on $DISPLAY" >&2
  cat "$WORK/xvfb.log" >&2 || true
  exit 1
fi

start_host() {
  "$BIN" >"$WORK/host.log" 2>&1 &
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

ask_approval() {
  local input="$1"
  local outfile="$2"
  local asked=0
  for _ in $(seq 1 40); do
    if ! run_in_pane "$outfile" "$BIN app call chess chess.play --json --input '$input' > '$outfile' 2>&1"; then
      sleep 1
      continue
    fi
    if python3 - "$outfile" <<'PY'
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
    cat "$outfile" >&2 || true
    return 1
  fi
  cat "$outfile.id"
}

echo "creating approval"
ID="$(ask_approval "$INPUT" "$WORK/call1.out")"
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
if body.get("error_code") != "permission_denied":
    raise SystemExit(f"terminal resolve was not permission_denied: {text} {err}")
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

AUDIT="$PROFILE/permission-audit.jsonl"
python3 - "$AUDIT" "$ID" <<'PY'
import json, sys
rows = [json.loads(line) for line in open(sys.argv[1]) if line.strip()]
refused = [
    row for row in rows
    if row.get("kind") == "refuse"
    and row.get("decision") == "refused_resolve"
    and row.get("call_id") == sys.argv[2]
]
if not refused:
    raise SystemExit(f"no refuse audit row for {sys.argv[2]}")
print("refuse audit row recorded")
PY

echo "clicking Allow once"
# shellcheck disable=SC1091
source "$ROOT/scripts/e2e/human.sh"
if ! HUMAN_APPROVE "$ID"; then
  echo "error: HUMAN_APPROVE did not grant $ID" >&2
  "$BIN" assistant permission list >"$WORK/buttons.json" 2>&1 || true
  cat "$WORK/buttons.json" >&2 || true
  exit 1
fi

echo "retrying the tool"
run_in_pane "$WORK/call2.out" "$BIN app call chess chess.play --json --input '$INPUT' > '$WORK/call2.out' 2>&1"
python3 - "$WORK/call2.out" <<'PY'
import json, sys
text = open(sys.argv[1]).read()
start = text.find("{")
if start < 0:
    raise SystemExit(f"tool did not proceed: {text}")
body = json.loads(text[start:])
if body.get("ok") is not True:
    raise SystemExit(f"tool did not proceed: {text}")
PY

echo "checking audit"
if [[ ! -f "$AUDIT" ]]; then
  echo "error: missing audit file $AUDIT" >&2
  exit 1
fi
python3 - "$AUDIT" "$ID" <<'PY'
import json, sys
rows = [json.loads(line) for line in open(sys.argv[1]) if line.strip()]
needs = [row for row in rows if row.get("kind") == "needs_you" and row.get("operation_id") == sys.argv[2] and row.get("decision") == "approved"]
grants = [row for row in rows if row.get("decision") == "allow_once"]
if not needs:
    raise SystemExit(f"no needs_you audit row for {sys.argv[2]}")
if not grants:
    raise SystemExit("no allow_once audit row")
print(f"allow_once rows {len(grants)}")
PY

echo "needs-you e2e passed"
