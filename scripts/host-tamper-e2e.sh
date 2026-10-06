#!/usr/bin/env bash
# Installed-binary check: kill the host and edit the permission profile
# while it is down. The next start lists an integrity item in Needs you.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${PLEXI_BIN:-$ROOT/target/release/plexi}"
if [[ ! -x "$BIN" ]]; then
  echo "error: missing binary $BIN (run just build first)" >&2
  exit 1
fi

WORK="$(mktemp -d)"
trap 'if [[ -n "${HOST_PID:-}" ]]; then kill -9 "$HOST_PID" 2>/dev/null || true; wait "$HOST_PID" 2>/dev/null || true; fi; rm -rf "$WORK"' EXIT

ORIG_HOME="${HOME}"
export HOME="$WORK/home"
mkdir -p "$HOME/.plexi"
if [[ -d "$ORIG_HOME/.plexi/wasm-bundles" ]]; then
  ln -s "$ORIG_HOME/.plexi/wasm-bundles" "$HOME/.plexi/wasm-bundles"
fi
export PLEXI_CHANNEL="host-tamper-e2e"
unset PLEXI_SOCKET PLEXI_PANE_ID PLEXI_CONTEXT_ID PLEXI_CONTEXT_ROOT PLEXI_RUNNING || true
export VK_DRIVER_FILES="${VK_DRIVER_FILES:-/usr/share/vulkan/icd.d/lvp_icd.json}"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp/runtime-ubuntu}"
mkdir -p "$XDG_RUNTIME_DIR"

PROFILE="$HOME/.plexi-$PLEXI_CHANNEL"
SOCKET="$PROFILE/notify.sock"
STAMP="$PROFILE/permission-integrity.toml"

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

echo "starting host $BIN"
start_host
wait_for_socket
export PLEXI_SOCKET="$SOCKET"
echo "host is up; stamp is present"

LISTENER="$(listener_pid)"
echo "killing host pid $LISTENER"
kill -9 "$LISTENER"
# The wrapper must not run the clean-quit path either.
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
if len(integrity) != 1:
    raise SystemExit(f"expected one integrity item, got {body}")
summary = integrity[0].get("summary", "")
if "permission profile changed" not in summary:
    raise SystemExit(f"summary does not name the profile change: {integrity[0]}")
if integrity[0].get("resolution") is not None:
    raise SystemExit(f"integrity item is already resolved: {integrity[0]}")
print("integrity", integrity[0]["id"], summary)
PY

if ! grep -q "profile_changed=true" "$WORK/host.log" "$PROFILE/plexi.log"; then
  echo "error: host log has no profile_changed trace" >&2
  exit 1
fi

echo "host tamper marker: pass"
