#!/usr/bin/env bash
# Installed-binary check for the host seal key.
#
# `cat secrets.json`, `plexi secret get permission-mac` from a workspace whose
# id is "host", and a direct keychain/Secret Service read of the user-secret
# account must not yield the key. A forged grants.toml line is quarantined
# and audited. Linux without Secret Service refuses to seal and says why.
#
# Unit tests use an in-memory host-key mock. This script never writes the
# login keychain: on macOS it sets PLEXI_KEYCHAIN_PATH to a throwaway file.
set -euo pipefail

# A fresh session bus so the seal key never lands in the user's keyring.
# Phase A clears this bus for its host. Phase B restores it.
if [[ "$(uname -s)" == "Linux" && -z "${SEAL_E2E_INNER:-}" ]] && command -v dbus-run-session >/dev/null 2>&1; then
  exec dbus-run-session -- env SEAL_E2E_INNER=1 "$0" "$@"
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${PLEXI_BIN:-$ROOT/target/release/plexi}"
if [[ ! -x "$BIN" ]]; then
  echo "error: missing binary $BIN (run just build first)" >&2
  exit 1
fi

# A channel-named binary (`plexi-pr-2718`, `plexi-alpha`) ignores PLEXI_CHANNEL.
# A bare `plexi` adopts it. The profile dir has to match whichever one is running.
bin_base="$(basename "$BIN")"
bin_base="${bin_base%.exe}"
if [[ "$bin_base" == plexi-* ]]; then
  export PLEXI_CHANNEL="${bin_base#plexi-}"
else
  export PLEXI_CHANNEL="${PLEXI_CHANNEL:-seal-e2e}"
fi

WORK="$(mktemp -d)"
trap 'if [[ -n "${HOST_PID:-}" ]]; then kill -- "-$HOST_PID" 2>/dev/null || kill "$HOST_PID" 2>/dev/null || true; wait "$HOST_PID" 2>/dev/null || true; fi; rm -rf "$WORK"' EXIT

ORIG_HOME="${HOME}"
export HOME="$WORK/home"
mkdir -p "$HOME/.plexi"
if [[ -d "$ORIG_HOME/.plexi/wasm-bundles" ]]; then
  ln -s "$ORIG_HOME/.plexi/wasm-bundles" "$HOME/.plexi/wasm-bundles"
fi
unset PLEXI_SOCKET PLEXI_PANE_ID PLEXI_CONTEXT_ID PLEXI_CONTEXT_ROOT PLEXI_RUNNING || true
export VK_DRIVER_FILES="${VK_DRIVER_FILES:-/usr/share/vulkan/icd.d/lvp_icd.json}"
export XDG_RUNTIME_DIR="$WORK/runtime"
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_RUNTIME_DIR/keyring"
chmod 700 "$XDG_RUNTIME_DIR" "$XDG_RUNTIME_DIR/keyring"

PROFILE="$HOME/.plexi-$PLEXI_CHANNEL"
SOCKET="$PROFILE/notify.sock"
mkdir -p "$PROFILE"

if [[ "$(uname -s)" == "Darwin" ]]; then
  export PLEXI_KEYCHAIN_PATH="$WORK/throwaway.keychain"
  security create-keychain -p "" "$PLEXI_KEYCHAIN_PATH" >/dev/null
  security set-keychain-settings "$PLEXI_KEYCHAIN_PATH" >/dev/null
  security unlock-keychain -p "" "$PLEXI_KEYCHAIN_PATH" >/dev/null
fi

stop_host() {
  if [[ -n "${HOST_PID:-}" ]]; then
    kill -- "-$HOST_PID" 2>/dev/null || kill "$HOST_PID" 2>/dev/null || true
    wait "$HOST_PID" 2>/dev/null || true
    unset HOST_PID
  fi
  rm -f "$SOCKET"
}

start_host() {
  unset DISPLAY
  if command -v xvfb-run >/dev/null 2>&1; then
    setsid xvfb-run -a "$BIN" >"$WORK/host.log" 2>&1 &
  else
    setsid "$BIN" >"$WORK/host.log" 2>&1 &
  fi
  HOST_PID=$!
  for _ in $(seq 1 90); do
    if [[ -S "$SOCKET" ]]; then
      return 0
    fi
    if ! kill -0 "$HOST_PID" 2>/dev/null; then
      echo "error: host exited before the socket appeared" >&2
      cat "$WORK/host.log" >&2 || true
      return 1
    fi
    sleep 1
  done
  echo "error: notify socket did not appear at $SOCKET" >&2
  cat "$WORK/host.log" >&2 || true
  return 1
}

plant_forge() {
  cat >"$PROFILE/grants.toml" <<'EOF'
[[records]]
actor_id = "forged"
# plexi-mac:00
EOF
}

assert_secret_file_hides_key() {
  local secrets="$PROFILE/secrets.json"
  if [[ -f "$secrets" ]]; then
    if grep -E 'permission-mac|plexi:host:' "$secrets"; then
      echo "error: secrets.json contains the host seal key" >&2
      exit 1
    fi
  fi
  echo "secrets.json does not contain the seal key"
}

assert_secret_get_refuses() {
  local ws="$WORK/ws-host"
  mkdir -p "$ws/.plexi-$PLEXI_CHANNEL"
  printf 'id = "host"\n' >"$ws/.plexi-$PLEXI_CHANNEL/workspace.toml"
  local out="$WORK/secret-get.txt"
  if (cd "$ws" && "$BIN" secret get permission-mac >"$out" 2>&1); then
    echo "error: secret get printed a value from workspace id=host" >&2
    cat "$out" >&2
    exit 1
  fi
  if grep -E '^[0-9a-f]{64}$' "$out"; then
    echo "error: secret get yielded a key" >&2
    exit 1
  fi
  if ! grep -q 'reserved' "$out"; then
    echo "error: secret get did not say the workspace id is reserved" >&2
    cat "$out" >&2
    exit 1
  fi
  echo "secret get from workspace id=host refused"
}

assert_direct_read_fails() {
  if [[ "$(uname -s)" == "Darwin" ]]; then
    if security find-generic-password -s plexi -a 'plexi:host:permission-mac' -w >"$WORK/keychain.txt" 2>"$WORK/keychain.err"; then
      echo "error: user keychain service returned the seal account" >&2
      exit 1
    fi
    if security find-generic-password -s plexi-host-seal -a permission-mac -w >"$WORK/keychain-host.txt" 2>>"$WORK/keychain.err"; then
      echo "error: security printed the host seal item" >&2
      exit 1
    fi
    echo "direct keychain read did not yield the key"
    return 0
  fi
  python3 - <<'PY'
import os, sys
try:
    import dbus
except ImportError:
    print("dbus python module missing; skipping live Secret Service attribute probe")
    sys.exit(0)
addr = os.environ.get("DBUS_SESSION_BUS_ADDRESS")
if not addr:
    print("no session bus; user-secret lookup cannot yield a key")
    sys.exit(0)
bus = dbus.SessionBus()
proxy = bus.get_object("org.freedesktop.secrets", "/org/freedesktop/secrets")
service = dbus.Interface(proxy, "org.freedesktop.Secret.Service")
unlocked, locked = service.SearchItems({"service": "plexi", "account": "plexi:host:permission-mac"})
if list(unlocked) or list(locked):
    sys.exit("user secret attributes returned a seal item")
print("direct Secret Service lookup of the user-secret account is empty")
PY
}

SAVED_BUS="${DBUS_SESSION_BUS_ADDRESS:-}"
SAVED_PID="${DBUS_SESSION_BUS_PID:-}"

echo "phase A: Linux-style refusal when Secret Service is not on the bus"
unset DBUS_SESSION_BUS_ADDRESS DBUS_SESSION_BUS_PID || true
plant_forge
echo "starting host without a session bus"
start_host
export PLEXI_SOCKET="$SOCKET"
"$BIN" permissions list --json >"$WORK/list-a.json" 2>"$WORK/list-a.err" || true
"$BIN" needs-you list --json >"$WORK/needs-a.json"
python3 - "$WORK/needs-a.json" <<'PY'
import json, sys
rows = json.load(open(sys.argv[1]))
items = rows if isinstance(rows, list) else rows.get("items") or rows.get("needs_you") or []
blob = json.dumps(items)
if "integrity" not in blob:
    sys.exit(f"needs-you has no integrity row: {blob[:500]}")
if "Secret Service" not in blob and "bad mac" not in blob:
    sys.exit(f"needs-you did not explain the rejection: {blob[:500]}")
print("forged grants.toml filed an integrity needs-you")
PY
if compgen -G "$PROFILE/grants.toml.untrusted-*" >/dev/null; then
  echo "forged grants.toml was quarantined"
else
  echo "error: forged grants.toml was not quarantined" >&2
  ls -la "$PROFILE" >&2 || true
  exit 1
fi
assert_secret_file_hides_key
assert_secret_get_refuses
assert_direct_read_fails
if [[ -f "$PROFILE/permission-audit.jsonl" ]]; then
  if grep -q integrity "$PROFILE/permission-audit.jsonl"; then
    echo "integrity audit row present"
  else
    echo "note: audit file has no integrity row (seal key unavailable is allowed to say why)"
    if ! grep -q 'Secret Service' "$WORK/needs-a.json"; then
      echo "error: neither the audit nor needs-you explained the refusal" >&2
      exit 1
    fi
  fi
else
  if ! grep -q 'Secret Service' "$WORK/needs-a.json"; then
    echo "error: no audit file and needs-you did not name Secret Service" >&2
    cat "$WORK/needs-a.json" >&2
    exit 1
  fi
  echo "no audit file; needs-you names Secret Service"
fi
stop_host

if [[ -n "$SAVED_BUS" ]]; then
  echo "phase B: Secret Service stores the key outside secrets.json"
  export DBUS_SESSION_BUS_ADDRESS="$SAVED_BUS"
  if [[ -n "$SAVED_PID" ]]; then
    export DBUS_SESSION_BUS_PID="$SAVED_PID"
  fi
  rm -f "$PROFILE"/grants.toml "$PROFILE"/grants.toml.untrusted-* "$PROFILE"/permission-audit.jsonl
  plant_forge
  start_host
  export PLEXI_SOCKET="$SOCKET"
  "$BIN" needs-you list --json >"$WORK/needs-b.json"
  python3 - "$WORK/needs-b.json" <<'PY'
import json, sys
rows = json.load(open(sys.argv[1]))
items = rows if isinstance(rows, list) else rows.get("items") or rows.get("needs_you") or []
blob = json.dumps(items)
if "integrity" not in blob or "bad mac" not in blob:
    sys.exit(f"expected bad-mac integrity row: {blob[:800]}")
print("forged line quarantined with bad mac")
PY
  if ! grep -q integrity "$PROFILE/permission-audit.jsonl"; then
    echo "error: integrity fact was not audited" >&2
    cat "$PROFILE/permission-audit.jsonl" >&2 || true
    exit 1
  fi
  assert_secret_file_hides_key
  assert_secret_get_refuses
  assert_direct_read_fails
  echo "phase B passed"
  stop_host
fi

echo "permissions-seal-e2e passed"
