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
export BIN
# shellcheck disable=SC1091
source "$ROOT/scripts/e2e/human.sh"

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
trap 'if [[ -n "${HOST_PID:-}" ]]; then kill -- "-$HOST_PID" 2>/dev/null || kill "$HOST_PID" 2>/dev/null || true; wait "$HOST_PID" 2>/dev/null || true; fi; if [[ -n "${MOCK_PID:-}" ]]; then kill "$MOCK_PID" 2>/dev/null || true; fi; if [[ -n "${XVFB_PID:-}" ]]; then kill "$XVFB_PID" 2>/dev/null || true; fi; "$BIN" host stop >/dev/null 2>&1 || true; rm -rf "$WORK"' EXIT

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
  if [[ -z "${DISPLAY:-}" ]] && command -v xvfb-run >/dev/null 2>&1; then
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
unset DBUS_SESSION_BUS_ADDRESS DBUS_SESSION_BUS_PID DISPLAY || true
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

# Contract step 4: a real pointer click grants, the Permissions app and
# `permissions list` show that row, then a real click revokes it. The next
# call asks. This never calls `permissions allow`.
click_grant_and_revoke() {
  if ! command -v xdotool >/dev/null 2>&1; then
    echo "error: xdotool is required for HUMAN_APPROVE" >&2
    exit 1
  fi
  local display=":147"
  local n
  for n in $(seq 147 170); do
    if [[ ! -e "/tmp/.X${n}-lock" ]]; then
      display=":$n"
      break
    fi
  done
  Xvfb "$display" -screen 0 1400x900x24 >"$WORK/xvfb.log" 2>&1 &
  XVFB_PID=$!
  export DISPLAY="$display"
  local i
  for i in $(seq 1 50); do
    if [[ -S "/tmp/.X11-unix/X${display#:}" ]]; then
      break
    fi
    sleep 0.1
  done
  local port=18765
  export MOCK_CONTROL="$WORK/mock.json"
  export MOCK_PORT="$port"
  python3 - "$MOCK_CONTROL" 'panes.list' '{}' <<'PY'
import json, sys
open(sys.argv[1], "w").write(json.dumps({"tool_substr": sys.argv[2], "arguments": json.loads(sys.argv[3])}))
PY
  cat >"$WORK/mock_model.py" <<'PY'
import json, os
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
CONTROL = os.environ["MOCK_CONTROL"]
PORT = int(os.environ["MOCK_PORT"])
CALLS = {"n": 0}

def control():
    try:
        return json.load(open(CONTROL, encoding="utf-8"))
    except OSError:
        return {}

def sse(payload):
    return f"data: {json.dumps(payload)}\n\n".encode()

class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def log_message(self, fmt, *args):
        return
    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(length)
        try:
            body = json.loads(raw.decode() or "{}")
        except json.JSONDecodeError:
            body = {}
        messages = body.get("messages") or []
        tools = body.get("tools") or []
        if any(m.get("role") == "tool" for m in messages):
            content = "ok"
            chunks = [
                {"choices": [{"index": 0, "delta": {"content": content}}]},
                {"choices": [{"index": 0, "finish_reason": "stop"}]},
            ]
        else:
            spec = control()
            want = spec.get("tool_substr", "")
            chosen = None
            for tool in tools:
                fn = (tool.get("function") or {}).get("name") or ""
                if want and want in fn:
                    chosen = fn
                    break
            if chosen is None:
                chunks = [
                    {"choices": [{"index": 0, "delta": {"content": f"no tool matching {want}"}}]},
                    {"choices": [{"index": 0, "finish_reason": "stop"}]},
                ]
            else:
                CALLS["n"] += 1
                arguments = json.dumps(spec.get("arguments") or {})
                cid = f"call_seal_{CALLS['n']}"
                chunks = [
                    {"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "id": cid, "function": {"name": chosen, "arguments": ""}}]}}]},
                    {"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "function": {"arguments": arguments}}]}}]},
                    {"choices": [{"index": 0, "finish_reason": "tool_calls"}]},
                ]
        blob = b"".join(sse(chunk) for chunk in chunks) + b"data: [DONE]\n\n"
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Content-Length", str(len(blob)))
        self.end_headers()
        self.wfile.write(blob)

ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
PY
  python3 "$WORK/mock_model.py" >"$WORK/mock.log" 2>&1 &
  MOCK_PID=$!
  sleep 0.2
  python3 - "$PROFILE/config.toml" "$port" <<'PY'
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
marker = "\n# seal e2e local mock\n"
if marker not in text:
    text += f"""{marker}
[ai.local]
base_url = "http://127.0.0.1:{port}"
model_low = "mock-seal"
model_medium = "mock-seal"
model_high = "mock-seal"
"""
path.write_text(text)
PY
  local ws="$WORK/ws-click"
  mkdir -p "$ws"
  echo "starting host on $DISPLAY for the permission click"
  "$BIN" host stop >/dev/null 2>&1 || true
  rm -f "$SOCKET"
  if ! "$BIN" host start --ephemeral --timeout-secs 90 --pane "cwd=$ws" >"$WORK/host-start.out" 2>"$WORK/host-start.err"; then
    echo "error: host start failed" >&2
    cat "$WORK/host-start.err" >&2 || true
    exit 1
  fi
  "$BIN" context set-root "$ws" >"$WORK/set-root.out" 2>"$WORK/set-root.err" || {
    echo "error: context set-root failed" >&2
    cat "$WORK/set-root.err" >&2
    exit 1
  }
  "$BIN" app install "$ROOT/apps/permissions" --yes >"$WORK/perm-install.out" 2>"$WORK/perm-install.err" || {
    echo "error: Permissions app install failed" >&2
    cat "$WORK/perm-install.err" >&2
    exit 1
  }
  "$BIN" app open assistant >"$WORK/open-assistant.out" 2>"$WORK/open-assistant.err" || true
  local assist=""
  for _ in $(seq 1 20); do
    "$BIN" pane list >"$WORK/panes.json" 2>/dev/null || true
    assist="$(python3 - "$WORK/panes.json" <<'PY'
import json, sys
rows=json.load(open(sys.argv[1]))
hits=[str(r["id"]) for r in rows if r.get("type")=="app" and "assistant" in str(r.get("title","")).lower()]
print(hits[-1] if hits else "")
PY
)"
    [[ -n "$assist" ]] && break
    sleep 1
  done
  if [[ -z "$assist" ]]; then
    echo "error: assistant pane did not open" >&2
    cat "$WORK/open-assistant.err" "$WORK/panes.json" >&2 || true
    exit 1
  fi
  "$BIN" pane focus "$assist" >/dev/null 2>&1 || true
  "$BIN" assistant send --pane-id "$assist" --text "List panes." --request-id seal-grant --json >"$WORK/grant-send.json" 2>"$WORK/grant-send.err" &
  local send_pid=$!
  local pending=""
  for _ in $(seq 1 45); do
    "$BIN" assistant permission list >"$WORK/grant-pending.json" 2>/dev/null || true
    pending="$(python3 - "$WORK/grant-pending.json" <<'PY'
import json, sys
try:
    data=json.load(open(sys.argv[1]))
except Exception:
    print(""); raise SystemExit
for row in data.get("pending") or []:
    blob=json.dumps(row)
    if "panes.list" in blob:
        print(row.get("pending_request_id",""))
        raise SystemExit
print("")
PY
)"
    [[ -n "$pending" ]] && break
    sleep 1
  done
  if [[ -z "$pending" ]]; then
    echo "error: no pending grant for the Permissions app" >&2
    cat "$WORK/grant-pending.json" "$WORK/grant-send.err" >&2 || true
    wait "$send_pid" || true
    exit 1
  fi
  echo "HUMAN_APPROVE grant pending $pending"
  if ! HUMAN_APPROVE "$pending" always; then
    echo "error: HUMAN_APPROVE did not grant $pending" >&2
    wait "$send_pid" || true
    exit 1
  fi
  wait "$send_pid" || true
  local grant_id=""
  for _ in $(seq 1 20); do
    "$BIN" permissions list --json >"$WORK/grant-list.json" 2>/dev/null || true
    grant_id="$(python3 - "$WORK/grant-list.json" <<'PY'
import json, sys
rows=json.load(open(sys.argv[1]))
items=rows if isinstance(rows, list) else rows.get("entries") or rows.get("permissions") or []
for row in items:
    if row.get("kind")=="allow" and "panes.list" in json.dumps(row):
        print(row.get("id",""))
        raise SystemExit
print("")
PY
)"
    [[ -n "$grant_id" ]] && break
    sleep 0.5
  done
  if [[ -z "$grant_id" ]]; then
    echo "error: permissions list has no allow row after the click" >&2
    cat "$WORK/grant-list.json" >&2 || true
    exit 1
  fi
  "$BIN" app open permissions >"$WORK/open-permissions.out" 2>"$WORK/open-permissions.err" || true
  local saw_app=0
  for _ in $(seq 1 30); do
    "$BIN" pane list >"$WORK/perm-panes.json" 2>/dev/null || true
    local perm_pane
    perm_pane="$(python3 - "$WORK/perm-panes.json" <<'PY'
import json, sys
rows=json.load(open(sys.argv[1]))
hits=[str(r["id"]) for r in rows if r.get("type")=="app" and "permission" in str(r.get("title","")).lower()]
print(hits[-1] if hits else "")
PY
)"
    if [[ -n "$perm_pane" ]]; then
      "$BIN" pane state "$perm_pane" >"$WORK/perm-state.json" 2>/dev/null || true
      if grep -q "$grant_id" "$WORK/perm-state.json"; then
        saw_app=1
        break
      fi
    fi
    sleep 1
  done
  if [[ "$saw_app" != 1 ]]; then
    echo "error: Permissions app did not show grant $grant_id" >&2
    cat "$WORK/open-permissions.err" "$WORK/perm-state.json" >&2 || true
    exit 1
  fi
  echo "permissions list and the Permissions app show $grant_id"
  python3 - "$MOCK_CONTROL" 'permissions.revoke' "$(python3 -c 'import json,sys; print(json.dumps({"id": sys.argv[1]}))' "$grant_id")" <<'PY'
import json, sys
open(sys.argv[1], "w").write(json.dumps({"tool_substr": sys.argv[2], "arguments": json.loads(sys.argv[3])}))
PY
  "$BIN" assistant send --pane-id "$assist" --text "Revoke that grant." --request-id seal-revoke --json >"$WORK/revoke-send.json" 2>"$WORK/revoke-send.err" &
  send_pid=$!
  local revoke_pending=""
  for _ in $(seq 1 45); do
    "$BIN" assistant permission list >"$WORK/revoke-pending.json" 2>/dev/null || true
    revoke_pending="$(python3 - "$WORK/revoke-pending.json" <<'PY'
import json, sys
try:
    data=json.load(open(sys.argv[1]))
except Exception:
    print(""); raise SystemExit
for row in data.get("pending") or []:
    blob=json.dumps(row)
    if "permissions.revoke" in blob:
        print(row.get("pending_request_id",""))
        raise SystemExit
print("")
PY
)"
    [[ -n "$revoke_pending" ]] && break
    sleep 1
  done
  if [[ -z "$revoke_pending" ]]; then
    echo "error: no pending revoke" >&2
    cat "$WORK/revoke-pending.json" "$WORK/revoke-send.err" >&2 || true
    wait "$send_pid" || true
    exit 1
  fi
  echo "HUMAN_APPROVE revoke pending $revoke_pending"
  if ! HUMAN_APPROVE "$revoke_pending" once; then
    echo "error: HUMAN_APPROVE did not revoke $revoke_pending" >&2
    wait "$send_pid" || true
    exit 1
  fi
  wait "$send_pid" || true
  "$BIN" permissions list --json >"$WORK/after-revoke.json"
  if grep -q "$grant_id" "$WORK/after-revoke.json"; then
    echo "error: grant $grant_id still listed after revoke" >&2
    cat "$WORK/after-revoke.json" >&2
    exit 1
  fi
  python3 - "$MOCK_CONTROL" <<'PY'
import json, sys
open(sys.argv[1], "w").write(json.dumps({"tool_substr": "panes.list", "arguments": {}}))
PY
  "$BIN" assistant send --pane-id "$assist" --text "List panes again." --request-id seal-ask-again --json >"$WORK/again-send.json" 2>"$WORK/again-send.err" &
  send_pid=$!
  local again=""
  for _ in $(seq 1 30); do
    "$BIN" assistant permission list >"$WORK/again-pending.json" 2>/dev/null || true
    again="$(python3 - "$WORK/again-pending.json" <<'PY'
import json, sys
try:
    data=json.load(open(sys.argv[1]))
except Exception:
    print(""); raise SystemExit
for row in data.get("pending") or []:
    if "panes.list" in json.dumps(row):
        print(row.get("pending_request_id",""))
        raise SystemExit
print("")
PY
)"
    [[ -n "$again" ]] && break
    sleep 1
  done
  if [[ -z "$again" ]]; then
    echo "error: the next call did not ask" >&2
    cat "$WORK/again-pending.json" "$WORK/again-send.err" >&2 || true
    wait "$send_pid" || true
    exit 1
  fi
  echo "next call asks again ($again)"
  # Leave the ask pending. A deny click closes the turn without granting.
  HUMAN_DENY "$again" || true
  wait "$send_pid" || true
  "$BIN" host stop >/dev/null 2>&1 || true
  echo "grant and revoke were HUMAN_APPROVE clicks"
}

if [[ -n "${SAVED_BUS:-}" || "$(uname -s)" != "Linux" ]]; then
  click_grant_and_revoke
fi

echo "permissions-seal-e2e passed"
