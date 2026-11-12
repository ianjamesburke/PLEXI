#!/usr/bin/env bash
# Installed-binary check: kill the host and edit the permission profile
# while it is down. The next start lists an integrity item in Needs you.
# This script does not resolve that item. A permission grant is HUMAN_APPROVE.
set -euo pipefail

if [[ "$(uname -s)" == "Linux" && -z "${HOST_TAMPER_E2E_INNER:-}" ]] && command -v dbus-run-session >/dev/null 2>&1; then
  exec dbus-run-session -- env HOST_TAMPER_E2E_INNER=1 "$0" "$@"
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${PLEXI_BIN:-$ROOT/target/release/plexi}"
if [[ ! -x "$BIN" ]]; then
  echo "error: missing binary $BIN (run just build first)" >&2
  exit 1
fi

WORK="$(mktemp -d)"
trap 'if [[ -n "${HOST_PID:-}" ]]; then kill -9 "$HOST_PID" 2>/dev/null || true; wait "$HOST_PID" 2>/dev/null || true; fi; if [[ -n "${KEYRING_PID:-}" ]]; then kill "$KEYRING_PID" 2>/dev/null || true; fi; rm -rf "$WORK"' EXIT

ORIG_HOME="${HOME}"
export HOME="$WORK/home"
mkdir -p "$HOME/.plexi"
if [[ -d "$ORIG_HOME/.plexi/wasm-bundles" ]]; then
  ln -s "$ORIG_HOME/.plexi/wasm-bundles" "$HOME/.plexi/wasm-bundles"
fi
export PLEXI_CHANNEL="host-tamper-e2e"
unset PLEXI_SOCKET PLEXI_PANE_ID PLEXI_CONTEXT_ID PLEXI_CONTEXT_ROOT PLEXI_RUNNING || true
export VK_DRIVER_FILES="${VK_DRIVER_FILES:-/usr/share/vulkan/icd.d/lvp_icd.json}"
export XDG_RUNTIME_DIR="$WORK/runtime"
mkdir -p "$XDG_RUNTIME_DIR/keyring"
chmod 700 "$XDG_RUNTIME_DIR" "$XDG_RUNTIME_DIR/keyring"

BIN_NAME="$(basename "$BIN")"
if [[ "$BIN_NAME" == plexi-* ]]; then
  PROFILE="$HOME/.plexi-${BIN_NAME#plexi-}"
else
  PROFILE="$HOME/.plexi-$PLEXI_CHANNEL"
fi
SOCKET="$PROFILE/notify.sock"
STAMP="$PROFILE/permission-integrity.toml"

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
  KEYRING_PID="$(pgrep -u "$(id -u)" -n -f '/usr/bin/gnome-keyring-daemon' || true)"
  local attempt
  for attempt in 1 2 3 4 5 6 7 8; do
    sleep 0.3
    if printf 'probe-value' | secret-tool store --label='host-tamper-e2e-probe' service host-tamper-e2e account probe >>"$WORK/keyring.log" 2>&1; then
      local got
      got="$(secret-tool lookup service host-tamper-e2e account probe 2>>"$WORK/keyring.log" || true)"
      secret-tool clear service host-tamper-e2e account probe >>"$WORK/keyring.log" 2>&1 || true
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

start_host() {
  unset DISPLAY
  if command -v xvfb-run >/dev/null 2>&1; then
    xvfb-run -a "$BIN" >"$WORK/host.log" 2>&1 &
  else
    "$BIN" >"$WORK/host.log" 2>&1 &
  fi
  HOST_PID=$!
}

wait_for_socket() {
  for _ in $(seq 1 90); do
    if [[ -S "$SOCKET" && -f "$STAMP" ]]; then
      return 0
    fi
    if ! kill -0 "$HOST_PID" 2>/dev/null; then
      echo "error: host exited before the socket and stamp appeared" >&2
      cat "$WORK/host.log" >&2 || true
      exit 1
    fi
    sleep 1
  done
  echo "error: notify socket or integrity stamp did not appear" >&2
  cat "$WORK/host.log" >&2 || true
  exit 1
}

listener_pid() {
  local inode pid
  inode="$(awk -v sock="$SOCKET" '$NF == sock { print $7; exit }' /proc/net/unix)"
  if [[ -z "${inode:-}" ]]; then
    return 1
  fi
  for pid in /proc/[0-9]*; do
    pid="${pid#/proc/}"
    if ls -l "/proc/$pid/fd" 2>/dev/null | grep -F -q "socket:[$inode]"; then
      echo "$pid"
      return 0
    fi
  done
  return 1
}

unlock_keyring

echo "starting host $BIN"
start_host
wait_for_socket
export PLEXI_SOCKET="$SOCKET"
echo "host is up; stamp is present"

LISTENER="$(listener_pid)"
echo "killing host pid $LISTENER"
kill -9 "$LISTENER"
kill -9 "$HOST_PID" 2>/dev/null || true
wait "$HOST_PID" 2>/dev/null || true
HOST_PID=
for _ in $(seq 1 30); do
  if ! kill -0 "$LISTENER" 2>/dev/null; then
    break
  fi
  sleep 0.2
done
if kill -0 "$LISTENER" 2>/dev/null; then
  echo "error: host pid $LISTENER survived SIGKILL" >&2
  exit 1
fi
rm -f "$SOCKET"

echo "editing grants.toml while the host is down"
printf '\n# tampered while the host was down\n' >> "$PROFILE/grants.toml"

echo "restarting host"
start_host
wait_for_socket
export PLEXI_SOCKET="$SOCKET"

echo "listing needs-you"
LIST="$("$BIN" needs-you list --json)"
printf '%s\n' "$LIST" >"$WORK/list.json"
python3 - "$WORK/list.json" <<'PY'
import json, sys
body = json.load(open(sys.argv[1]))
items = body.get("items", [])
integrity = [item for item in items if item.get("kind") == "integrity"]
changed = [
    item for item in integrity
    if "permission profile changed" in item.get("summary", "")
    and item.get("resolution") is None
]
if not changed:
    raise SystemExit(f"expected an unresolved profile-change item, got {body}")
print("integrity", changed[0]["id"], changed[0]["summary"])
PY

if ! grep -q "profile_changed=true" "$WORK/host.log" "$PROFILE/plexi.log"; then
  echo "error: host log has no profile_changed trace" >&2
  exit 1
fi

echo "host tamper marker: pass"
