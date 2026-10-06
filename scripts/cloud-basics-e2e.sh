#!/usr/bin/env bash
# Installed-binary checks for cloud basics.
# Usage: scripts/cloud-basics-e2e.sh <PR>
#
# Uses a temp HOME. The channel profile is ${HOME}/.<binary-name>, derived
# from the installed plexi-pr-<PR> binary, or from PLEXI_E2E_SHIM when the
# acceptance harness sets it. Never touches the OS keychain and never opens a
# secret dialog: it does not call secret commands, and the host is started
# with --background so it does not activate a window. A relay that rejects
# plaintext is checked with a sealed envelope that does not contain the canary.

set -euo pipefail

# An ambient DISPLAY can be an unauthorized socket (XOpenDisplayFailed) and
# would put a window on a real session. Always use a private Xvfb. The marker
# stops the re-exec from looping once xvfb-run has set DISPLAY.
if [[ -z "${PLEXI_CLOUD_E2E_XVFB:-}" ]] && command -v xvfb-run >/dev/null 2>&1; then
  export PLEXI_CLOUD_E2E_XVFB=1
  exec xvfb-run -a "$0" "$@"
fi

if [[ -n "${PLEXI_E2E_SHIM:-}" ]]; then
  # shellcheck disable=SC1090
  source "${PLEXI_E2E_SHIM:?}"
else
  PR="${1:?usage: scripts/cloud-basics-e2e.sh <PR>}"
  BIN="plexi-pr-${PR}"
  if ! command -v "$BIN" >/dev/null 2>&1; then
    echo "FAIL retention: $BIN is not on PATH. Run: just pr-install ${PR}"
    echo "FAIL relay-canary: skipped"
    echo "FAIL no-account: skipped"
    exit 1
  fi
  unset PLEXI_CHANNEL
fi

if [[ -z "${BIN_PATH:-}" ]]; then
  BIN_PATH="$(command -v "$BIN")"
fi
BIN_NAME="$(basename "$BIN_PATH")"
REAL_HOME="${HOME}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TMP_HOME="$(mktemp -d "${TMPDIR:-/tmp}/plexi-cloud-e2e-home.XXXXXX")"
export HOME="${TMP_HOME}"
unset PLEXI_SOCKET

PROFILE="${HOME}/.${BIN_NAME}"
CHANNEL_DIR=".${BIN_NAME}"
WORKDIR="$(mktemp -d "${TMPDIR:-/tmp}/plexi-cloud-e2e-work.XXXXXX")"
HOST_STARTED=0
RETENTION="FAIL"
CANARY="FAIL"
ACCOUNT="FAIL"

cleanup() {
  local status=$?
  if [[ "$HOST_STARTED" == 1 ]]; then
    "$BIN_PATH" host stop >/dev/null 2>&1 || true
  fi
  rm -rf "$TMP_HOME" "$WORKDIR"
  exit "$status"
}
trap cleanup EXIT

if [[ "$PROFILE" == "$REAL_HOME"/* || "$PROFILE" == "$REAL_HOME" ]]; then
  echo "FAIL retention: profile resolved inside the real home ($PROFILE)"
  echo "FAIL relay-canary: skipped"
  echo "FAIL no-account: skipped"
  exit 1
fi

echo "binary: $BIN_PATH"
echo "profile: $PROFILE"
echo "workspace: $WORKDIR"

# Dates are wall-clock, because the installed host prunes with its own clock.
# 31 days is outside the window even if startup takes a few minutes.
# 29 days stays inside it.
read -r OLD_DAY KEPT_DAY OLD_TS KEPT_TS < <(python3 - <<'PY'
from datetime import datetime, timedelta, timezone
local = datetime.now().astimezone()
utc = datetime.now(timezone.utc)
def day(delta):
    return (local.date() - timedelta(days=delta)).isoformat()
def ts(delta):
    return (utc - timedelta(days=delta)).strftime("%Y-%m-%dT%H:%M:%SZ")
print(day(31), day(29), ts(31), ts(29))
PY
)

mkdir -p "$PROFILE"
cat >"$PROFILE/config.toml" <<'EOF'
[cloud]
retain_local_history = true
EOF

cd "$WORKDIR"
"$BIN_PATH" workspace init >/tmp/cloud-e2e-init.out
echo "workspace init:"
cat /tmp/cloud-e2e-init.out

ASSISTANT="$WORKDIR/$CHANNEL_DIR/assistant"
mkdir -p "$ASSISTANT/history" "$ASSISTANT/conversations" "$ASSISTANT/checkpoints/old-conv"
printf '%s\n' "old log" >"$PROFILE/plexi-${OLD_DAY}.log"
printf '%s\n' "kept log" >"$PROFILE/plexi-${KEPT_DAY}.log"
printf '%s\n' "live log" >"$PROFILE/plexi.log"
cat >"$PROFILE/ai-ledger.jsonl" <<EOF
{"ts":"${OLD_TS}","backend":"e2e","billing":"metered","input_tokens":1,"output_tokens":1,"cost_cents":0,"marker":"old-ledger"}
{"ts":"${KEPT_TS}","backend":"e2e","billing":"metered","input_tokens":2,"output_tokens":2,"cost_cents":0,"marker":"kept-ledger"}
not-json-kept
EOF
printf '%s\n' "{\"updated_at\":\"${OLD_TS}\",\"checkpoints\":[],\"compactions\":[],\"interruptions\":[]}" \
  >"$ASSISTANT/history/old-conv.json"
printf '%s\n' "{\"updated_at\":\"${KEPT_TS}\",\"checkpoints\":[],\"compactions\":[],\"interruptions\":[]}" \
  >"$ASSISTANT/history/kept-conv.json"
printf '%s\n' '{"role":"User","text":"old turn","created_at":"'"${OLD_TS}"'"}' \
  >"$ASSISTANT/conversations/old-conv.jsonl"
printf '%s\n' '{"role":"User","text":"kept turn","created_at":"'"${KEPT_TS}"'"}' \
  >"$ASSISTANT/conversations/kept-conv.jsonl"
printf '%s\n' "old checkpoint" >"$ASSISTANT/checkpoints/old-conv/c1.jsonl"
cat >"$ASSISTANT/state.toml" <<'EOF'
show_thoughts = true

[contexts.1]
active_conversation = "old-conv"
EOF

echo "=== relay canary ==="
CANARY_MARK="CANARY-plexi-relay-9f3a2c1b"
RELAY_DIR="$(mktemp -d "${TMPDIR:-/tmp}/plexi-cloud-e2e-relay.XXXXXX")"
if python3 - "$ROOT" "$RELAY_DIR" "$CANARY_MARK" <<'PY'
import json, logging, os, socket, sqlite3, sys, threading, urllib.error, urllib.request
from pathlib import Path

root, relay_dir, canary = sys.argv[1:4]
sys.path.insert(0, str(Path(root) / "services" / "relay"))
import relay

log_path = Path(relay_dir) / "relay.log"
state_path = str(Path(relay_dir) / "registry.sqlite")
static = Path(root) / "clients" / "phone-web" / "static"
relay.install_log_guard()
handler = logging.FileHandler(log_path)
handler.setFormatter(logging.Formatter("%(message)s"))
relay.log.addHandler(handler)

box = relay.Relay(public_origin="http://127.0.0.1", state_path=state_path)
server = relay.build_server("127.0.0.1", 0, box, static)
thread = threading.Thread(target=server.serve_forever, daemon=True)
thread.start()
port = server.server_address[1]
base = f"http://127.0.0.1:{port}"

def exact(sock, n):
    buf = b""
    while len(buf) < n:
        chunk = sock.recv(n - len(buf))
        if not chunk:
            raise ConnectionError("socket closed")
        buf += chunk
    return buf

def ws_send(sock, payload, opcode=0x1):
    mask = os.urandom(4)
    masked = bytes(byte ^ mask[i % 4] for i, byte in enumerate(payload))
    length = len(payload)
    if length < 126:
        header = bytes([0x80 | opcode, 0x80 | length])
    elif length < 65536:
        header = bytes([0x80 | opcode, 0x80 | 126]) + length.to_bytes(2, "big")
    else:
        header = bytes([0x80 | opcode, 0x80 | 127]) + length.to_bytes(8, "big")
    sock.sendall(header + mask + masked)

def ws_recv(sock):
    head = exact(sock, 2)
    opcode = head[0] & 0x0F
    length = head[1] & 0x7F
    if length == 126:
        length = int.from_bytes(exact(sock, 2), "big")
    elif length == 127:
        length = int.from_bytes(exact(sock, 8), "big")
    payload = exact(sock, length) if length else b""
    return opcode, payload

def http(method, url, body=None, cookie=None):
    data = None if body is None else json.dumps(body).encode()
    headers = {"Content-Type": "application/json"} if body is not None else {}
    if cookie:
        headers["Cookie"] = f"{relay.COOKIE}={cookie}"
    request = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(request, timeout=5) as response:
            raw = response.read().decode()
            set_cookie = response.headers.get("Set-Cookie")
            status = response.status
    except urllib.error.HTTPError as exc:
        raw = exc.read().decode()
        set_cookie = exc.headers.get("Set-Cookie") if exc.headers else None
        status = exc.code
    token = None
    if set_cookie:
        for part in set_cookie.split(";"):
            name, _, value = part.strip().partition("=")
            if name == relay.COOKIE and value:
                token = value
    return status, json.loads(raw or "{}"), token

try:
    sock = socket.create_connection(("127.0.0.1", port), timeout=5)
    key = __import__("base64").b64encode(os.urandom(16)).decode()
    sock.sendall(
        (
            "GET /v1/desktop HTTP/1.1\r\n"
            f"Host: 127.0.0.1:{port}\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\n"
            "Sec-WebSocket-Version: 13\r\n\r\n"
        ).encode()
    )
    raw = b""
    while b"\r\n\r\n" not in raw:
        raw += sock.recv(4096)
    if b"101" not in raw.split(b"\r\n", 1)[0]:
        raise SystemExit(f"websocket upgrade failed: {raw[:200]!r}")
    ws_send(sock, json.dumps({
        "type": "hello",
        "host_id": "host-e2e",
        "host_token": "host-token-e2e-not-logged",
        "host_label": "desk",
        "protocol": 1,
    }).encode())
    opcode, payload = ws_recv(sock)
    hello = json.loads(payload.decode())
    if hello.get("type") != "hello_ok":
        raise SystemExit(f"hello failed: {hello}")
    ws_send(sock, json.dumps({"type": "pair_start"}).encode())
    issued = json.loads(ws_recv(sock)[1].decode())
    status, pending, _ = http("POST", f"{base}/api/pair", {"code": issued["code"], "label": "pixel"})
    if status != 202:
        raise SystemExit(f"pair failed: {status} {pending}")
    if json.loads(ws_recv(sock)[1].decode()).get("type") != "pair_pending":
        raise SystemExit("missing pair_pending")
    ws_send(sock, json.dumps({"type": "pair_confirm", "pairing_id": issued["pairing_id"]}).encode())
    if json.loads(ws_recv(sock)[1].decode()).get("type") != "pair_confirmed":
        raise SystemExit("pair was not confirmed")
    status, polled, cookie = http("GET", f"{base}/api/pair/{issued['pairing_id']}")
    if polled.get("status") != "confirmed" or not cookie:
        raise SystemExit(f"pair poll failed: {status} {polled}")
    # V1-13: the canary string is absent from relay logs. A relay that still
    # accepts type=text is checked with that plaintext turn. A relay that
    # rejects plaintext (the encrypted relay is right to) is checked with a
    # sealed envelope whose body does not contain the canary.
    plaintext = {
        "schema_version": 1,
        "request_id": "req-canary",
        "content": [{"type": "text", "text": canary}],
    }
    if relay.validate_envelope(plaintext) == "plaintext_rejected":
        try:
            import phone_crypto
            sealed = phone_crypto.opaque_body(canary)
        except Exception:
            sealed = __import__("base64").urlsafe_b64encode(os.urandom(48)).decode().rstrip("=")
        if canary in sealed:
            raise SystemExit("sealed body leaked the canary")
        turn = {
            "schema_version": 1,
            "request_id": "req-canary",
            "content": [{"type": "sealed", "body": sealed}],
        }
        reply_text = sealed
        print("relay rejected plaintext; posted a sealed envelope", file=sys.stderr)
    else:
        turn = plaintext
        reply_text = canary
    status, queued, _ = http(
        "POST",
        f"{base}/api/turns",
        turn,
        cookie=cookie,
    )
    if status != 202:
        raise SystemExit(f"turn failed: {status} {queued}")
    delivered = json.loads(ws_recv(sock)[1].decode())
    ws_send(sock, json.dumps({
        "type": "reply",
        "delivery_id": delivered["delivery_id"],
        "request_id": "req-canary",
        "state": "succeeded",
        "reply": reply_text,
    }).encode())
finally:
    server.shutdown()
    server.server_close()
    handler.flush()

blob = log_path.read_bytes()
for candidate in Path(relay_dir).iterdir():
    if candidate.is_file():
        blob += b"\n" + candidate.read_bytes()
if canary.encode() in blob:
    raise SystemExit("canary appeared in a relay log or database file")
conn = sqlite3.connect(state_path)
try:
    tables = [row[0] for row in conn.execute("SELECT name FROM sqlite_master WHERE type='table'")]
    for table in tables:
        for row in conn.execute(f"SELECT * FROM {table}"):
            rendered = " ".join("" if value is None else str(value) for value in row)
            if canary in rendered:
                raise SystemExit(f"canary stored in {table}")
        columns = [info[1] for info in conn.execute(f"PRAGMA table_info({table})")]
        if "body" in columns or "content" in columns or "text" in columns:
            raise SystemExit(f"{table} has a content column: {columns}")
finally:
    conn.close()
print("relay canary absent from logs and database rows")
PY
then
  CANARY="PASS"
else
  CANARY="FAIL"
fi
rm -rf "$RELAY_DIR"

echo "=== host retention and no-account ==="
if "$BIN_PATH" host start --background --ephemeral --timeout-secs 90; then
  HOST_STARTED=1
else
  echo "host start failed"
fi

if [[ "$HOST_STARTED" == 1 ]]; then
  STATUS="$("$BIN_PATH" host status --json)"
  echo "host status: $STATUS"
  ACCOUNT_OUT="$("$BIN_PATH" account status)"
  echo "account status: $ACCOUNT_OUT"
  python3 - "$STATUS" <<'PY' && READY=1 || READY=0
import json, sys
body = json.loads(sys.argv[1])
raise SystemExit(0 if body.get("ready") is True else 1)
PY
  missing=0
  present=0
  [[ ! -f "$PROFILE/plexi-${OLD_DAY}.log" ]] && missing=$((missing + 1)) || echo "old log still present"
  [[ -f "$PROFILE/plexi-${KEPT_DAY}.log" ]] && present=$((present + 1)) || echo "kept log missing"
  [[ -f "$PROFILE/plexi.log" ]] && present=$((present + 1)) || echo "live log missing"
  [[ -f "$PROFILE/config.toml" ]] && present=$((present + 1)) || echo "config missing"
  if grep -q 'old-ledger' "$PROFILE/ai-ledger.jsonl"; then
    echo "old ledger row still present"
  else
    missing=$((missing + 1))
  fi
  grep -q 'kept-ledger' "$PROFILE/ai-ledger.jsonl" && present=$((present + 1)) || echo "kept ledger row missing"
  grep -q 'not-json-kept' "$PROFILE/ai-ledger.jsonl" && present=$((present + 1)) || echo "unparsed ledger line missing"
  [[ ! -e "$ASSISTANT/history/old-conv.json" ]] && missing=$((missing + 1)) || echo "old history still present"
  [[ ! -e "$ASSISTANT/conversations/old-conv.jsonl" ]] && missing=$((missing + 1)) || echo "old transcript still present"
  [[ ! -e "$ASSISTANT/checkpoints/old-conv" ]] && missing=$((missing + 1)) || echo "old checkpoint still present"
  [[ -f "$ASSISTANT/history/kept-conv.json" ]] && present=$((present + 1)) || echo "kept history missing"
  [[ -f "$ASSISTANT/conversations/kept-conv.jsonl" ]] && present=$((present + 1)) || echo "kept transcript missing"
  if grep -q 'old-conv' "$ASSISTANT/state.toml"; then
    echo "state still points at the old conversation"
  else
    present=$((present + 1))
  fi
  grep -q 'show_thoughts' "$ASSISTANT/state.toml" && present=$((present + 1)) || echo "show_thoughts missing"
  if grep -R -q 'cloud retention:' "$PROFILE" --include='*.log' && [[ "$missing" == 5 && "$present" == 9 ]]; then
    RETENTION="PASS"
  else
    echo "retention counts missing=$missing present=$present (want 5 and 9)"
    echo "--- profile logs ---"
    grep -R -n 'cloud retention:' "$PROFILE" --include='*.log' || true
  fi
  ACCOUNT_FILE="$(find "$TMP_HOME" -name account.toml -print -quit || true)"
  if [[ "$READY" == 1 ]] \
    && printf '%s' "$ACCOUNT_OUT" | grep -q 'does not need an account' \
    && [[ -z "$ACCOUNT_FILE" ]]; then
    ACCOUNT="PASS"
  else
    echo "ready=$READY account_file=${ACCOUNT_FILE:-absent}"
  fi
fi

echo "CHECK retention: $RETENTION"
echo "CHECK relay-canary: $CANARY"
echo "CHECK no-account: $ACCOUNT"

if [[ "$RETENTION" == PASS && "$CANARY" == PASS && "$ACCOUNT" == PASS ]]; then
  exit 0
fi
exit 1
