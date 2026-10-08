#!/usr/bin/env bash
# Installed-binary check: a pending approval survives a host restart with the
# same id. A terminal resolve, including one after restart, does not grant it
# and the row stays open. A file an agent writes into the profile is not a grant.
set -euo pipefail

# The journal HMAC is the host seal key. A private session bus is where that
# key is stored on Linux. Without it the queue file is never written.
if [[ "$(uname -s)" == "Linux" && -z "${NEEDS_YOU_PERSIST_INNER:-}" ]] && command -v dbus-run-session >/dev/null 2>&1; then
  exec dbus-run-session -- env NEEDS_YOU_PERSIST_INNER=1 "$0" "$@"
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
export PLEXI_CHANNEL="needs-you-persist-e2e"
unset PLEXI_SOCKET PLEXI_PANE_ID PLEXI_CONTEXT_ID PLEXI_CONTEXT_ROOT PLEXI_RUNNING || true
export VK_DRIVER_FILES="${VK_DRIVER_FILES:-/usr/share/vulkan/icd.d/lvp_icd.json}"
export XDG_RUNTIME_DIR="$WORK/runtime"
mkdir -p "$XDG_RUNTIME_DIR/keyring"
chmod 700 "$XDG_RUNTIME_DIR" "$XDG_RUNTIME_DIR/keyring"

# A channel-named binary (`plexi-pr-N`) ignores PLEXI_CHANNEL and uses its own
# profile. The bare binary follows PLEXI_CHANNEL.
BIN_NAME="$(basename "$BIN")"
if [[ "$BIN_NAME" == plexi-* ]]; then
  PROFILE="$HOME/.plexi-${BIN_NAME#plexi-}"
else
  PROFILE="$HOME/.plexi-$PLEXI_CHANNEL"
fi
SOCKET="$PROFILE/notify.sock"
HOST_DIR="$PROFILE/host"
INPUT='{"game_id":"game-1","expected_revision":0,"operation_id":"op-needs-you-persist","move":"e2e4"}'

start_host() {
  local log="$1"
  unset DISPLAY
  if command -v xvfb-run >/dev/null 2>&1; then
    xvfb-run -a "$BIN" >"$log" 2>&1 &
  else
    "$BIN" >"$log" 2>&1 &
  fi
  HOST_PID=$!
}

wait_for_socket() {
  local log="$1"
  for _ in $(seq 1 90); do
    if [[ -S "$SOCKET" ]]; then
      return 0
    fi
    if ! kill -0 "$HOST_PID" 2>/dev/null; then
      echo "error: host exited before the socket appeared" >&2
      cat "$log" >&2 || true
      exit 1
    fi
    sleep 1
  done
  echo "error: notify socket did not appear at $SOCKET" >&2
  cat "$log" >&2 || true
  exit 1
}

stop_host() {
  "$BIN" host stop >"$WORK/stop.log" 2>&1 || true
  if [[ -n "${HOST_PID:-}" ]]; then
    for _ in $(seq 1 20); do
      if ! kill -0 "$HOST_PID" 2>/dev/null; then
        break
      fi
      sleep 1
    done
    kill "$HOST_PID" 2>/dev/null || true
    wait "$HOST_PID" 2>/dev/null || true
  fi
  rm -f "$SOCKET"
  unset HOST_PID
}

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

unlock_keyring() {
  if ! command -v gnome-keyring-daemon >/dev/null 2>&1 || ! command -v secret-tool >/dev/null 2>&1; then
    echo "error: gnome-keyring or secret-tool is not installed" >&2
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
  local attempt
  for attempt in 1 2 3 4 5 6 7 8; do
    sleep 0.3
    if printf 'probe-value' | secret-tool store --label='needs-you-persist-probe' service needs-you-persist account probe >>"$WORK/keyring.log" 2>&1; then
      local got
      got="$(secret-tool lookup service needs-you-persist account probe 2>>"$WORK/keyring.log" || true)"
      secret-tool clear service needs-you-persist account probe >>"$WORK/keyring.log" 2>&1 || true
      if [[ "$got" == "probe-value" ]]; then
        echo "secret service unlocked"
        return 0
      fi
    fi
  done
  echo "error: gnome-keyring did not accept a secret" >&2
  cat "$WORK/keyring.log" >&2 || true
  exit 1
}

unlock_keyring

echo "starting host $BIN"
start_host "$WORK/host.log"
wait_for_socket "$WORK/host.log"
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

echo "checking host-only queue"
python3 - "$HOST_DIR" "$PROFILE" "$ID" <<'PY'
import os, stat, sys
host, profile, pending = sys.argv[1:]
mode = stat.S_IMODE(os.stat(host).st_mode)
if mode != 0o700:
    raise SystemExit(f"host dir mode is {oct(mode)}")
journal_path = os.path.join(host, "needs-you.json")
journal_mode = stat.S_IMODE(os.stat(journal_path).st_mode)
if journal_mode != 0o600:
    raise SystemExit(f"needs-you.json mode is {oct(journal_mode)}")
if os.path.exists(os.path.join(host, "seal.key")):
    raise SystemExit("seal key must live in the host seal store, not beside the journal")
if os.path.exists(os.path.join(profile, "needs-you.json")):
    raise SystemExit("queue was written at the profile root")
journal = open(journal_path, "rb").read()
if pending.encode() not in journal:
    raise SystemExit("journal does not contain the pending id")
PY

echo "planting a forged approval from the pane"
cat >"$WORK/forge.sh" <<EOF
#!/bin/sh
profile=\$(dirname "\$PLEXI_SOCKET")
printf '%s\n' '{"id":"req_forged","resolution":"approved"}' > "\$profile/needs-you.json"
mkdir -p "\$profile/host"
printf '%s\n' '{"id":"req_forged","resolution":"approved"}' > "\$profile/host/planted.json"
printf '%s\n' forged > "$WORK/forge.out"
EOF
chmod +x "$WORK/forge.sh"
run_in_pane "$WORK/forge.out" "sh '$WORK/forge.sh'"

echo "restarting host"
stop_host
start_host "$WORK/host-restart.log"
wait_for_socket "$WORK/host-restart.log"
export PLEXI_SOCKET="$SOCKET"

echo "listing needs-you after restart"
LIST="$("$BIN" needs-you list --json)"
printf '%s\n' "$LIST" >"$WORK/list.json"
python3 - "$WORK/list.json" "$ID" <<'PY'
import json, sys
body = json.load(open(sys.argv[1]))
ids = [item["id"] for item in body.get("items", [])]
if sys.argv[2] not in ids:
    raise SystemExit(f"{sys.argv[2]} not in {body}")
if "req_forged" in ids:
    raise SystemExit(f"forged id was listed: {body}")
item = next(item for item in body["items"] if item["id"] == sys.argv[2])
if item.get("kind") != "approval_click" or item.get("resolution") is not None:
    raise SystemExit(f"unexpected row {item}")
PY

echo "forged id is not resolvable"
set +e
FORGE_RESOLVE="$("$BIN" needs-you resolve req_forged --approve 2>"$WORK/forge-resolve.err")"
FORGE_CODE=$?
set -e
printf '%s\n' "$FORGE_RESOLVE" >"$WORK/forge-resolve.json"
python3 - "$WORK/forge-resolve.json" "$FORGE_CODE" <<'PY'
import json, sys
text, code = open(sys.argv[1]).read(), int(sys.argv[2])
if code == 0 and '"ok":true' in text.replace(" ", "") and "req_forged" in text:
    raise SystemExit(f"forged id resolved: {text}")
if text.strip():
    body = json.loads(text)
    if body.get("ok") is True and body.get("id") == "req_forged":
        raise SystemExit(f"forged id resolved: {body}")
PY

echo "terminal resolve after restart must not grant"
set +e
RESOLVE="$("$BIN" needs-you resolve "$ID" --approve 2>"$WORK/resolve.err")"
RESOLVE_CODE=$?
set -e
printf '%s\n' "$RESOLVE" >"$WORK/resolve.json"
python3 - "$WORK/resolve.json" "$RESOLVE_CODE" "$ID" <<'PY'
import json, sys
text, code, want = open(sys.argv[1]).read(), int(sys.argv[2]), sys.argv[3]
body = {}
start = text.find("{")
if start >= 0:
    try:
        body = json.loads(text[start:])
    except json.JSONDecodeError:
        body = {}
if code == 0 and body.get("ok") is True and body.get("resolution") == "approved" and body.get("id") == want:
    raise SystemExit(f"terminal resolve granted after restart: {text}")
print("terminal resolve did not grant")
PY

echo "item stays open after the refused resolve"
AFTER="$("$BIN" needs-you list --json)"
python3 - "$AFTER" "$ID" <<'PY'
import json, sys
body = json.loads(sys.argv[1])
item = next((row for row in body.get("items", []) if row.get("id") == sys.argv[2]), None)
if item is None or item.get("resolution") is not None:
    raise SystemExit(f"pending item was settled from the terminal: {body}")
PY

echo "overwriting the sealed queue from the pane"
cat >"$WORK/overwrite.sh" <<EOF
#!/bin/sh
profile=\$(dirname "\$PLEXI_SOCKET")
printf '%s\n' '{"schema":1,"items":[{"id":"req_preapproved","kind":"approval_click","actor":"agent:chess","resource":"game-1","summary":"forged","created_at":1,"origin_session":"sess-forged","resolution":"approved"}]}' > "\$profile/host/needs-you.json"
printf '%s\n' overwritten > "$WORK/overwrite.out"
EOF
chmod +x "$WORK/overwrite.sh"
# The restart dropped the old pane. Open a terminal by listing after a fresh
# chess open so the agent write happens inside a pane.
"$BIN" app open "$ROOT/apps/chess" >"$WORK/open2.log" 2>&1 || true
for _ in $(seq 1 40); do
  if "$BIN" pane list >"$WORK/panes2.txt" 2>/dev/null && grep -q . "$WORK/panes2.txt"; then
    break
  fi
  sleep 1
done
PANE="$(python3 - "$WORK/panes2.txt" <<'PY'
import json, sys
panes = json.load(open(sys.argv[1]))
terms = [pane for pane in panes if pane.get("type") == "terminal"]
chosen = terms[0] if terms else panes[0]
print(chosen["id"])
PY
)"
run_in_pane "$WORK/overwrite.out" "sh '$WORK/overwrite.sh'"

echo "restarting after the forged seal"
stop_host
start_host "$WORK/host-forged.log"
wait_for_socket "$WORK/host-forged.log"
export PLEXI_SOCKET="$SOCKET"
FORGED_LIST="$("$BIN" needs-you list --json)"
printf '%s\n' "$FORGED_LIST" >"$WORK/forged-list.json"
python3 - "$WORK/forged-list.json" <<'PY'
import json, sys
body = json.load(open(sys.argv[1]))
ids = [item["id"] for item in body.get("items", [])]
if "req_preapproved" in ids:
    raise SystemExit(f"pre-approved forge was loaded: {body}")
kinds = [item.get("kind") for item in body.get("items", [])]
if "integrity" not in kinds:
    raise SystemExit(f"tampered queue did not raise an integrity item: {body}")
PY

echo "needs-you persist e2e passed"
