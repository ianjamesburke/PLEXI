#!/usr/bin/env bash
# Installed-binary check: package a sample app, install it, and confirm the
# one grant store. A declared non-sensitive capability is auto-granted and
# listed. A sensitive capability is absent until a human grants it. Deleting
# permissions.toml does not change the list.
set -euo pipefail

# Auto-grant has to seal grants.toml. Linux can do that only with Secret Service.
if [[ "$(uname -s)" == "Linux" && -z "${APP_SHARE_E2E_INNER:-}" ]] && command -v dbus-run-session >/dev/null 2>&1; then
  exec dbus-run-session -- env APP_SHARE_E2E_INNER=1 "$0" "$@"
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

# A channel-named binary ignores PLEXI_CHANNEL. Match its profile dir.
bin_base="$(basename "$BIN")"
bin_base="${bin_base%.exe}"
if [[ "$bin_base" == plexi-* ]]; then
  export PLEXI_CHANNEL="${bin_base#plexi-}"
else
  export PLEXI_CHANNEL="${PLEXI_CHANNEL:-app-share-e2e}"
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
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR" || true

PROFILE="$HOME/.plexi-$PLEXI_CHANNEL"
SOCKET="$PROFILE/notify.sock"
APP_ID="sample-share"

start_display() {
  if [[ -n "${DISPLAY:-}" && -n "${XVFB_PID:-}" ]]; then
    return 0
  fi
  local display=":157"
  local n
  for n in $(seq 157 180); do
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
      return 0
    fi
    sleep 0.1
  done
  echo "error: Xvfb $display did not come up" >&2
  exit 1
}

start_host() {
  start_display
  setsid "$BIN" >"$WORK/host.log" 2>&1 &
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

stop_host() {
  if [[ -n "${HOST_PID:-}" ]]; then
    kill -- "-$HOST_PID" 2>/dev/null || kill "$HOST_PID" 2>/dev/null || true
    wait "$HOST_PID" 2>/dev/null || true
    unset HOST_PID
  fi
  rm -f "$SOCKET"
}

echo "starting host $BIN"
start_host
export PLEXI_SOCKET="$SOCKET"

echo "scaffolding $APP_ID"
"$BIN" app init "$APP_ID" --global >"$WORK/init.log" 2>&1
APP_DIR="$PROFILE/apps/$APP_ID"
if [[ ! -f "$APP_DIR/manifest.toml" ]]; then
  echo "error: scaffold did not write $APP_DIR/manifest.toml" >&2
  cat "$WORK/init.log" >&2
  exit 1
fi
python3 - "$APP_DIR/manifest.toml" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
text = path.read_text()
old = 'capabilities = ["timer"]'
new = 'capabilities = ["timer", "permissions.manage"]'
if old not in text:
    sys.exit(f"scaffold manifest has no timer capability list:\n{text}")
path.write_text(text.replace(old, new, 1))
PY

echo "validating and packaging"
"$BIN" app validate "$APP_DIR" >"$WORK/validate.log" 2>&1
"$BIN" app package "$APP_DIR" --out "$WORK/$APP_ID.plexipkg" >"$WORK/package.log" 2>&1
test -f "$WORK/$APP_ID.plexipkg"

echo "installing package"
"$BIN" app install "$WORK/$APP_ID.plexipkg" --yes >"$WORK/install.log" 2>&1
if ! grep -q timer "$WORK/install.log"; then
  echo "error: install trust sheet did not name timer" >&2
  cat "$WORK/install.log" >&2
  exit 1
fi

echo "opening app"
if ! "$BIN" app open "$APP_ID" >"$WORK/open.log" 2>&1; then
  echo "warning: app open returned non-zero; the grant check still decides" >&2
  cat "$WORK/open.log" >&2 || true
fi

echo "waiting for permission rows"
ready=0
for _ in $(seq 1 40); do
  if "$BIN" permissions list --json >"$WORK/list.json" 2>"$WORK/list.err"; then
    if python3 - "$WORK/list.json" <<'PY'
import json, sys
rows = json.load(open(sys.argv[1]))
items = rows if isinstance(rows, list) else rows.get("entries") or rows.get("permissions") or []
blob = json.dumps(items)
ok = "timer" in blob and "permissions.manage" not in blob
sys.exit(0 if ok else 1)
PY
    then
      ready=1
      break
    fi
  fi
  sleep 1
done
if [[ "$ready" != 1 ]]; then
  echo "error: permissions list did not show timer without permissions.manage" >&2
  cat "$WORK/list.json" "$WORK/list.err" "$WORK/open.log" >&2 || true
  exit 1
fi
cp "$WORK/list.json" "$WORK/list-before.json"
echo "list shows the auto-granted timer row and withholds permissions.manage"

echo "removing permissions.toml"
rm -f "$PROFILE/permissions.toml"
stop_host
start_host
export PLEXI_SOCKET="$SOCKET"
ready=0
for _ in $(seq 1 40); do
  if "$BIN" permissions list --json >"$WORK/list-after.json" 2>"$WORK/list-after.err"; then
    if grep -q timer "$WORK/list-after.json"; then
      ready=1
      break
    fi
  fi
  sleep 1
done
python3 - "$WORK/list-before.json" "$WORK/list-after.json" <<'PY'
import json, sys
def items(path):
    rows = json.load(open(path))
    return rows if isinstance(rows, list) else rows.get("entries") or rows.get("permissions") or []
before = json.dumps(items(sys.argv[1]), sort_keys=True)
after = json.dumps(items(sys.argv[2]), sort_keys=True)
if "timer" not in after or "permissions.manage" in after:
    sys.exit(f"enforcement changed after deleting permissions.toml:\n{after[:800]}")
if before != after:
    sys.exit(f"permission list changed after deleting permissions.toml\nbefore={before[:400]}\nafter={after[:400]}")
print("deleting permissions.toml changed nothing")
PY

if [[ -f "$PROFILE/grants.toml" ]]; then
  if ! grep -q timer "$PROFILE/grants.toml"; then
    echo "error: grants.toml has no timer row" >&2
    exit 1
  fi
  echo "grants.toml contains the timer grant"
else
  echo "error: grants.toml was not written" >&2
  exit 1
fi

# The sensitive capability stays withheld. The app tool asks; a real click
# lets that call run once. This never auto-grants permissions.manage.
approve_sensitive_tool() {
  if ! command -v xdotool >/dev/null 2>&1; then
    echo "error: xdotool is required for HUMAN_APPROVE" >&2
    exit 1
  fi
  local port=18766
  export MOCK_CONTROL="$WORK/mock.json"
  export MOCK_PORT="$port"
  python3 - "$MOCK_CONTROL" <<'PY'
import json, sys
open(sys.argv[1], "w").write(json.dumps({
    "tool_substr": "demo.greet",
    "arguments": {"name": "Ada"},
}))
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
            chunks = [
                {"choices": [{"index": 0, "delta": {"content": "done"}}]},
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
                names = [(t.get("function") or {}).get("name") for t in tools]
                chunks = [
                    {"choices": [{"index": 0, "delta": {"content": "no tool matching %s in %s" % (want, names)}}]},
                    {"choices": [{"index": 0, "finish_reason": "stop"}]},
                ]
            else:
                CALLS["n"] += 1
                arguments = json.dumps(spec.get("arguments") or {})
                cid = f"call_share_{CALLS['n']}"
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
marker = "\n# app-share e2e local mock\n"
if marker not in text:
    text += f"""{marker}
[ai.local]
base_url = "http://127.0.0.1:{port}"
model_low = "mock-share"
model_medium = "mock-share"
model_high = "mock-share"
"""
path.write_text(text)
PY
  echo "restarting host so the assistant uses the mock"
  stop_host
  start_host
  export PLEXI_SOCKET="$SOCKET"
  local ws="$WORK/ws-share"
  mkdir -p "$ws"
  "$BIN" context set-root "$ws" >"$WORK/set-root.out" 2>"$WORK/set-root.err" || {
    echo "error: context set-root failed" >&2
    cat "$WORK/set-root.err" >&2
    exit 1
  }
  "$BIN" app open "$APP_ID" >"$WORK/reopen.log" 2>&1 || true
  local ready=0
  for _ in $(seq 1 40); do
    "$BIN" pane list >"$WORK/app-panes.json" 2>/dev/null || true
    if python3 - "$WORK/app-panes.json" "$APP_ID" <<'PY'
import json, sys
rows=json.load(open(sys.argv[1]))
want=sys.argv[2]
raise SystemExit(0 if any(want in str(r.get("title","")) or want in str(r.get("app_id","")) for r in rows) else 1)
PY
    then
      ready=1
      break
    fi
    sleep 1
  done
  if [[ "$ready" != 1 ]]; then
    echo "error: sample app did not reopen" >&2
    cat "$WORK/reopen.log" "$WORK/app-panes.json" >&2 || true
    exit 1
  fi
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
    exit 1
  fi
  "$BIN" pane focus "$assist" >/dev/null 2>&1 || true
  "$BIN" permissions list --json >"$WORK/before-click.json"
  if grep -q 'permissions.manage' "$WORK/before-click.json"; then
    echo "error: permissions.manage was auto-granted" >&2
    cat "$WORK/before-click.json" >&2
    exit 1
  fi
  echo "permissions.manage is not auto-granted"
  "$BIN" assistant send --pane-id "$assist" --text "Greet Ada with the sample app." --request-id share-greet --json >"$WORK/greet-send.json" 2>"$WORK/greet-send.err" &
  local send_pid=$!
  local pending=""
  for _ in $(seq 1 45); do
    "$BIN" assistant permission list >"$WORK/greet-pending.json" 2>/dev/null || true
    pending="$(python3 - "$WORK/greet-pending.json" <<'PY'
import json, sys
try:
    data=json.load(open(sys.argv[1]))
except Exception:
    print(""); raise SystemExit
for row in data.get("pending") or []:
    blob=json.dumps(row)
    if "greet" in blob or "sample-share" in blob:
        print(row.get("pending_request_id",""))
        raise SystemExit
print("")
PY
)"
    [[ -n "$pending" ]] && break
    sleep 1
  done
  if [[ -z "$pending" ]]; then
    echo "error: sensitive tool did not ask" >&2
    cat "$WORK/greet-pending.json" "$WORK/greet-send.err" "$WORK/mock.log" >&2 || true
    wait "$send_pid" || true
    exit 1
  fi
  echo "HUMAN_APPROVE sensitive tool pending $pending"
  if ! HUMAN_APPROVE "$pending" once; then
    echo "error: HUMAN_APPROVE did not resolve $pending" >&2
    wait "$send_pid" || true
    exit 1
  fi
  wait "$send_pid" || true
  if ! python3 - "$WORK/greet-send.json" <<'PY'
import json, sys
outer=json.load(open(sys.argv[1]))
raise SystemExit(0 if outer.get("state")=="succeeded" else 1)
PY
  then
    echo "error: the approved tool call did not succeed" >&2
    cat "$WORK/greet-send.json" "$WORK/greet-send.err" >&2 || true
    exit 1
  fi
  "$BIN" permissions list --json >"$WORK/after-click.json"
  if grep -q 'permissions.manage' "$WORK/after-click.json"; then
    echo "error: permissions.manage appeared without its own human grant" >&2
    cat "$WORK/after-click.json" >&2
    exit 1
  fi
  echo "HUMAN_APPROVE let the tool run once; permissions.manage stays withheld"
}

approve_sensitive_tool

echo "app-share-e2e passed"
