#!/usr/bin/env bash
# Installed-binary checks for the local house-agent runner.
# Usage: scripts/house-agent-e2e.sh <PR>
#
# Uses a temp HOME. The channel profile is ${HOME}/.<binary-name>. The script
# never calls a secret command and never opens a dialog. Docker must already
# be running; this script does not install or deploy a cloud host.

set -euo pipefail

PR="${1:?usage: scripts/house-agent-e2e.sh <PR>}"
BIN="plexi-pr-${PR}"
fail_all() {
  echo "FAIL start: ${1:-skipped}"
  echo "FAIL non-root: skipped"
  echo "FAIL unapproved: skipped"
  echo "FAIL reply: skipped"
  echo "FAIL denied: skipped"
  echo "FAIL no-host-secret: skipped"
  echo "FAIL vault: skipped"
  echo "FAIL retention: skipped"
  echo "FAIL egress: skipped"
  echo "FAIL stop: skipped"
  exit 1
}
if ! command -v "$BIN" >/dev/null 2>&1; then
  fail_all "$BIN is not on PATH. Run: just pr-install ${PR}"
fi
if ! command -v docker >/dev/null 2>&1; then
  fail_all "docker is not on PATH"
fi

BIN_PATH="$(command -v "$BIN")"
BIN_NAME="$(basename "$BIN_PATH")"
REAL_HOME="${HOME}"
TMP_HOME="$(mktemp -d "${TMPDIR:-/tmp}/plexi-house-e2e-home.XXXXXX")"
export HOME="${TMP_HOME}"
unset PLEXI_SOCKET
unset PLEXI_CHANNEL
CANARY="HOST-SECRET-CANARY-7c1e9a"
export PLEXI_HOST_SECRET="${CANARY}"
TENANT="e2e$(python3 -c 'import secrets; print(secrets.token_hex(3))')"
PROFILE="${HOME}/.${BIN_NAME}"
AGENT="plexi-house-agent-${TENANT}"
RELAY="plexi-house-relay-${TENANT}"
NETWORK="plexi-house-${TENANT}"
EDGE="plexi-house-edge-${TENANT}"
VOLUME="plexi-tenant-${TENANT}"
RELAY_VOLUME="plexi-relay-${TENANT}"
STARTED=0
START="FAIL"
NONROOT="FAIL"
UNAPPROVED="FAIL"
REPLY="FAIL"
DENIED="FAIL"
SECRET="FAIL"
VAULT="FAIL"
RETENTION="FAIL"
EGRESS="FAIL"
STOP="FAIL"

cleanup() {
  local status=$?
  if [[ "$STARTED" == 1 ]]; then
    "$BIN_PATH" cloud agent stop --tenant "$TENANT" >/dev/null 2>&1 || true
  fi
  docker rm -f "$AGENT" "$RELAY" >/dev/null 2>&1 || true
  docker network rm "$NETWORK" "$EDGE" >/dev/null 2>&1 || true
  docker volume rm "$VOLUME" "$RELAY_VOLUME" >/dev/null 2>&1 || true
  rm -rf "$TMP_HOME"
  exit "$status"
}
trap cleanup EXIT

if [[ "$PROFILE" == "$REAL_HOME"/* || "$PROFILE" == "$REAL_HOME" ]]; then
  echo "FAIL start: profile resolved inside the real home ($PROFILE)"
  exit 1
fi

mkdir -p "$HOME/.ssh" "$PROFILE"
printf '%s\n' "$CANARY" >"$HOME/.ssh/plexi-host-secret"
printf '%s\n' "$CANARY" >"$PROFILE/host-secret"
chmod 600 "$HOME/.ssh/plexi-host-secret" "$PROFILE/host-secret"

echo "binary: $BIN_PATH"
echo "profile: $PROFILE"
echo "tenant: $TENANT"

MODEL_KEY="$(python3 -c 'import secrets; print(secrets.token_hex(16), end="")')"
VAULT_SET="$(printf '%s\n' "$MODEL_KEY" | "$BIN_PATH" cloud agent vault set --tenant "$TENANT" || true)"
echo "vault-set: $VAULT_SET"
MODEL_FP="$(MODEL_KEY="$MODEL_KEY" python3 -c 'import hashlib,os; print(hashlib.sha256(os.environ["MODEL_KEY"].encode()).hexdigest()[:12])')"

if RUN_JSON="$("$BIN_PATH" cloud agent run --agent chess-opponent --tenant "$TENANT")"; then
  echo "run: $RUN_JSON"
  STARTED=1
else
  echo "run failed"
  RUN_JSON=""
fi
if [[ "$STARTED" == 1 ]]; then
read -r PORT CODE PAIR_ID < <(python3 -c 'import json,sys; body=json.loads(sys.argv[1]); print(body["relay_port"], body["pairing_code"], body["pairing_id"])' "$RUN_JSON")
if [[ -n "$PORT" && -n "$CODE" && -n "$PAIR_ID" ]]; then
  START="PASS"
fi

AGENT_USER="$(docker inspect -f '{{.Config.User}}' "$AGENT" 2>/dev/null || true)"
if [[ "$AGENT_USER" == "65532:65532" ]] && docker exec "$AGENT" python -c 'import os,sys
st = os.stat("/tenant")
sys.exit(0 if os.getuid() == 65532 and st.st_uid == 65532 else 1)'; then
  NONROOT="PASS"
else
  echo "tenant is not running as uid 65532 (user=${AGENT_USER})"
fi

cat >/tmp/house-unapproved.py <<'PY'
import json, sys, time, urllib.error, urllib.request
port, code, pairing_id = sys.argv[1:4]
base = f"http://127.0.0.1:{port}"

def http(method, url, body=None, cookie=None):
    data = None if body is None else json.dumps(body).encode()
    headers = {"Content-Type": "application/json"} if body is not None else {}
    if cookie:
        headers["Cookie"] = f"plexi_phone={cookie}"
    request = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(request, timeout=5) as response:
            raw = response.read().decode()
            status = response.status
    except urllib.error.HTTPError as exc:
        raw = exc.read().decode()
        status = exc.code
    try:
        parsed = json.loads(raw or "{}")
    except json.JSONDecodeError:
        parsed = {"raw": raw}
    return status, parsed

status, pending = http("POST", f"{base}/api/pair", {"code": code, "label": "house-e2e"})
if status != 202 or pending.get("status") != "pending_desktop":
    raise SystemExit(f"pair did not stay pending: {status} {pending}")
polled = {}
for _ in range(6):
    status, polled = http("GET", f"{base}/api/pair/{pairing_id}")
    if polled.get("status") != "pending_desktop":
        raise SystemExit(f"pair changed before approval: {status} {polled}")
    time.sleep(0.25)
turn_status, turn_body = http(
    "POST",
    f"{base}/api/turns",
    {
        "schema_version": 1,
        "request_id": "req-unapproved",
        "content": [{"type": "text", "text": "should-not-send"}],
    },
)
if turn_status == 202:
    raise SystemExit(f"unapproved turn was accepted: {turn_body}")
print(f"held status={polled.get('status')} turn={turn_status}")
PY
if UNAUTH_OUT="$(python3 /tmp/house-unapproved.py "$PORT" "$CODE" "$PAIR_ID")"; then
  echo "unapproved: $UNAUTH_OUT"
else
  echo "unapproved probe failed: ${UNAUTH_OUT:-}"
  UNAUTH_OUT=""
fi
PENDING_JSON=""
for _ in 1 2 3 4 5 6 7 8 9 10; do
  PENDING_JSON="$("$BIN_PATH" cloud agent pending --tenant "$TENANT" || true)"
  if [[ "$PENDING_JSON" == *"$PAIR_ID"* ]]; then
    break
  fi
  sleep 0.3
done
echo "pending: $PENDING_JSON"
if [[ "$UNAUTH_OUT" == held* && "$PENDING_JSON" == *"$PAIR_ID"* ]]; then
  UNAPPROVED="PASS"
else
  echo "unapproved pair was able to send, or it never reached Needs you"
fi

APPROVE_JSON="$("$BIN_PATH" cloud agent approve --tenant "$TENANT" --pairing-id "$PAIR_ID" || true)"
echo "approve: $APPROVE_JSON"

cat >/tmp/house-turns.py <<'PY'
import json, sys, time, urllib.error, urllib.request
port, code, pairing_id = sys.argv[1:4]
base = f"http://127.0.0.1:{port}"

def http(method, url, body=None, cookie=None):
    data = None if body is None else json.dumps(body).encode()
    headers = {"Content-Type": "application/json"} if body is not None else {}
    if cookie:
        headers["Cookie"] = f"plexi_phone={cookie}"
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
            if name == "plexi_phone" and value:
                token = value
    return status, json.loads(raw or "{}"), token

status, pending, _ = http("POST", f"{base}/api/pair", {"code": code, "label": "house-e2e"})
if status != 202:
    raise SystemExit(f"pair failed: {status} {pending}")
cookie = None
polled = {}
for _ in range(40):
    status, polled, found = http("GET", f"{base}/api/pair/{pairing_id}")
    if polled.get("status") == "confirmed" and found:
        cookie = found
        break
    time.sleep(0.25)
else:
    raise SystemExit(f"pair was not confirmed: {polled}")

def turn(request_id, text):
    status, queued, _ = http(
        "POST",
        f"{base}/api/turns",
        {
            "schema_version": 1,
            "request_id": request_id,
            "content": [{"type": "text", "text": text}],
        },
        cookie=cookie,
    )
    if status != 202:
        raise SystemExit(f"turn {request_id} failed: {status} {queued}")
    for _ in range(80):
        _status, page, _cookie = http("GET", f"{base}/api/conversation", cookie=cookie)
        texts = [
            event.get("text", "")
            for event in page.get("events", [])
            if event.get("request_id") == request_id
            and event.get("kind") == "assistant_reply"
            and event.get("text")
        ]
        if texts:
            return "\n".join(texts)
        time.sleep(0.25)
    raise SystemExit(f"no reply for {request_id}")

hello = turn("req-hello", json.dumps({"kind": "message", "text": "hello"}))
denied = turn("req-play", json.dumps({"kind": "tool", "name": "app.chess.play", "input": {}}))
model = turn("req-model", json.dumps({"kind": "model"}))
with open("/tmp/house-phone-cookie", "w", encoding="utf-8") as handle:
    handle.write(cookie or "")
print(hello)
print("---")
print(denied)
print("---")
print(model)
PY
if TURN_OUT="$(python3 /tmp/house-turns.py "$PORT" "$CODE" "$PAIR_ID")"; then
  echo "turns: $TURN_OUT"
else
  echo "turns failed: ${TURN_OUT:-}"
  TURN_OUT=""
fi
HELLO_TEXT="${TURN_OUT%%---*}"
REST_TEXT="${TURN_OUT#*---}"
DENIED_TEXT="${REST_TEXT%%---*}"
MODEL_TEXT="${REST_TEXT#*---}"
if [[ "$HELLO_TEXT" == *"Chess Opponent ready."* ]]; then
  REPLY="PASS"
fi
if [[ "$DENIED_TEXT" == *"blocked"* && "$DENIED_TEXT" == *"permission_denied"* ]]; then
  if docker exec "$AGENT" python -c 'import pathlib,sys; sys.exit(0 if not pathlib.Path("/tenant/effects/app.chess.play").exists() else 1)' \
    && docker exec "$AGENT" python -c 'import pathlib,sys; sys.exit(0 if pathlib.Path("/tenant/effects/agent.turn").is_file() else 1)'; then
    DENIED="PASS"
  else
    echo "effect files did not match the gate decision"
  fi
fi

if docker exec "$AGENT" python -c 'import os, pathlib, sys
roots = ["/tenant", "/opt/house", "/root", "/home", "/etc"]
for key, value in os.environ.items():
    sys.stdout.write(f"{key}={value}\n")
for root in roots:
    path = pathlib.Path(root)
    if not path.exists():
        continue
    try:
        items = list(path.rglob("*"))
    except PermissionError:
        continue
    for item in items:
        try:
            if not item.is_file() or item.stat().st_size > 2_000_000:
                continue
            sys.stdout.buffer.write(item.read_bytes())
            sys.stdout.buffer.write(b"\n")
        except PermissionError:
            continue
' > /tmp/house-dump.bin; then
  MOUNTS="$(docker inspect -f '{{range .Mounts}}{{.Type}} {{.Source}} {{.Destination}}{{println}}{{end}}' "$AGENT")"
  echo "mounts: $MOUNTS"
  if ! grep -a -q -F "$CANARY" /tmp/house-dump.bin \
    && [[ "$MOUNTS" != *"$TMP_HOME"* \
    && "$MOUNTS" != *"docker.sock"* \
    && "$MOUNTS" == *"volume"* \
    && "$MOUNTS" == *"/tenant"* \
    && "$MOUNTS" == *"/opt/plexi/plexi"* ]]; then
    SECRET="PASS"
  else
    echo "host secret or an unexpected mount was visible"
  fi
else
  echo "could not read the container filesystem"
fi

VAULT_OK=0
if [[ "$VAULT_SET" != *"$MODEL_KEY"* && "$MODEL_TEXT" == *"model ${MODEL_FP}"* ]]; then
  if printf '%s' "$MODEL_KEY" | docker exec -i "$AGENT" python -c 'import os, pathlib, sys
key = sys.stdin.read()
stored = pathlib.Path("/tenant/vault/model").read_text(encoding="utf-8")
visible = "\n".join(f"{name}={value}" for name, value in os.environ.items())
opt = pathlib.Path("/opt/house")
if opt.exists():
    for item in opt.rglob("*"):
        if item.is_file() and item.stat().st_size < 2_000_000:
            visible += "\n" + item.read_text(encoding="utf-8", errors="replace")
sys.exit(0 if stored == key and key not in visible else 1)'; then
    VAULT_OK=1
  else
    echo "model credential was baked into the image or missing from the volume"
  fi
else
  echo "vault set leaked the secret, or the model reply was not the fingerprint"
fi
REVOKE_JSON="$("$BIN_PATH" cloud agent vault revoke --tenant "$TENANT" || true)"
echo "vault-revoke: $REVOKE_JSON"
cat >/tmp/house-revoked.py <<'PY'
import json, pathlib, sys, time, urllib.error, urllib.request
port = sys.argv[1]
cookie = pathlib.Path("/tmp/house-phone-cookie").read_text(encoding="utf-8")
base = f"http://127.0.0.1:{port}"

def http(method, url, body=None):
    data = None if body is None else json.dumps(body).encode()
    headers = {"Content-Type": "application/json", "Cookie": f"plexi_phone={cookie}"}
    request = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(request, timeout=5) as response:
            raw = response.read().decode()
            status = response.status
    except urllib.error.HTTPError as exc:
        raw = exc.read().decode()
        status = exc.code
    return status, json.loads(raw or "{}")

request_id = "req-revoked"
status, queued = http(
    "POST",
    f"{base}/api/turns",
    {
        "schema_version": 1,
        "request_id": request_id,
        "content": [{"type": "text", "text": json.dumps({"kind": "model"})}],
    },
)
if status != 202:
    raise SystemExit(f"revoked turn was not queued: {status} {queued}")
for _ in range(80):
    _status, page = http("GET", f"{base}/api/conversation")
    texts = [
        event.get("text", "")
        for event in page.get("events", [])
        if event.get("request_id") == request_id
        and event.get("kind") == "assistant_reply"
        and event.get("text")
    ]
    if texts:
        print("\n".join(texts))
        raise SystemExit(0)
    time.sleep(0.25)
raise SystemExit("no reply for revoked model turn")
PY
REVOKED_TEXT=""
if [[ -f /tmp/house-phone-cookie ]]; then
  REVOKED_TEXT="$(python3 /tmp/house-revoked.py "$PORT" || true)"
fi
echo "revoked-turn: $REVOKED_TEXT"
if [[ "$VAULT_OK" == 1 && "$REVOKE_JSON" != *"$MODEL_KEY"* && "$REVOKED_TEXT" == *"vault_revoked"* ]] \
  && docker exec "$AGENT" python -c 'import pathlib,sys; sys.exit(0 if not pathlib.Path("/tenant/vault/model").exists() else 1)'; then
  VAULT="PASS"
else
  echo "revoked model credential was still usable"
fi

read -r OLD_DAY KEPT_DAY OLD_TS KEPT_TS < <(python3 - <<'PY'
from datetime import datetime, timedelta, timezone
today = datetime.now(timezone.utc).date()
def day(delta):
    return (today - timedelta(days=delta)).isoformat()
def ts(delta):
    return (datetime.now(timezone.utc) - timedelta(days=delta)).strftime("%Y-%m-%dT%H:%M:%SZ")
print(day(31), day(29), ts(31), ts(1))
PY
)
if docker exec -i "$AGENT" python - "$OLD_DAY" "$KEPT_DAY" "$OLD_TS" "$KEPT_TS" <<'PY'
import pathlib, sys
old_day, kept_day, old_ts, kept_ts = sys.argv[1:5]
profile = pathlib.Path("/tenant/profile")
profile.joinpath(f"plexi-{old_day}.log").write_text("old log\n", encoding="utf-8")
profile.joinpath(f"plexi-{kept_day}.log").write_text("kept log\n", encoding="utf-8")
profile.joinpath("ai-ledger.jsonl").write_text(
    '{"ts":"%s","marker":"old-ledger"}\n{"ts":"%s","marker":"kept-ledger"}\nnot-json-kept\n'
    % (old_ts, kept_ts),
    encoding="utf-8",
)
PY
then
  if docker exec "$AGENT" /opt/plexi/plexi cloud agent retain --tenant-profile /tenant/profile >/tmp/house-retain.out; then
    echo "retain: $(cat /tmp/house-retain.out)"
    if docker exec "$AGENT" python -c "import pathlib,sys
profile = pathlib.Path('/tenant/profile')
old = profile.joinpath('plexi-${OLD_DAY}.log')
kept = profile.joinpath('plexi-${KEPT_DAY}.log')
ledger = profile.joinpath('ai-ledger.jsonl').read_text(encoding='utf-8')
ok = (not old.exists()) and kept.is_file() and 'old-ledger' not in ledger and 'kept-ledger' in ledger and 'not-json-kept' in ledger
sys.exit(0 if ok else 1)"; then
      RETENTION="PASS"
    else
      echo "tenant retention did not apply"
    fi
  else
    echo "tenant retain command failed"
  fi
else
  echo "could not seed tenant retention files"
fi

AGENT_NETS="$(docker inspect -f '{{range $name, $net := .NetworkSettings.Networks}}{{$name}} {{end}}' "$AGENT")"
RELAY_NETS="$(docker inspect -f '{{range $name, $net := .NetworkSettings.Networks}}{{$name}} {{end}}' "$RELAY")"
INTERNAL="$(docker network inspect -f '{{.Internal}}' "$NETWORK" 2>/dev/null || echo false)"
MASQ="$(docker network inspect -f '{{index .Options "com.docker.network.bridge.enable_ip_masquerade"}}' "$EDGE" 2>/dev/null || echo missing)"
if [[ "$AGENT_NETS" == "$NETWORK " && "$INTERNAL" == "true" && "$RELAY_NETS" == *"$NETWORK"* && "$RELAY_NETS" == *"$EDGE"* && "$MASQ" == "false" ]]; then
  if docker exec "$AGENT" python -c 'import errno, socket, sys
try:
    socket.create_connection(("1.1.1.1", 443), 3)
except OSError as error:
    sys.exit(0 if error.errno == errno.ENETUNREACH else 1)
else:
    sys.exit(1)'; then
    if docker exec "$AGENT" python -c "import os,socket,sys
url = os.environ['RELAY_URL']
host = url.split('://', 1)[1].split(':')[0]
port = int(url.rsplit(':', 1)[1])
socket.create_connection((host, port), 3)
"; then
      EGRESS="PASS"
    else
      echo "agent could not reach the relay"
    fi
  else
    echo "agent has a route to the public internet"
  fi
else
  echo "network plan mismatch agent=[$AGENT_NETS] relay=[$RELAY_NETS] internal=$INTERNAL masquerade=$MASQ"
fi

if "$BIN_PATH" cloud agent stop --tenant "$TENANT" >/tmp/house-stop.out; then
  STARTED=0
else
  echo "stop command failed"
fi
STATUS_JSON="$("$BIN_PATH" cloud agent status --tenant "$TENANT" || true)"
echo "stop: $(cat /tmp/house-stop.out 2>/dev/null || true)"
echo "status: $STATUS_JSON"
RUNNING="True"
if [[ -n "$STATUS_JSON" ]]; then
  RUNNING="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1]).get("running"))' "$STATUS_JSON" || echo True)"
fi
if [[ "$RUNNING" == "False" ]] \
  && ! docker ps -a --format '{{.Names}}' | grep -qx "$AGENT" \
  && ! docker ps -a --format '{{.Names}}' | grep -qx "$RELAY"; then
  STOP="PASS"
fi
fi

echo "CHECK start: $START"
echo "CHECK non-root: $NONROOT"
echo "CHECK unapproved: $UNAPPROVED"
echo "CHECK reply: $REPLY"
echo "CHECK denied: $DENIED"
echo "CHECK no-host-secret: $SECRET"
echo "CHECK vault: $VAULT"
echo "CHECK retention: $RETENTION"
echo "CHECK egress: $EGRESS"
echo "CHECK stop: $STOP"

if [[ "$START" == PASS && "$NONROOT" == PASS && "$UNAPPROVED" == PASS && "$REPLY" == PASS && "$DENIED" == PASS && "$SECRET" == PASS && "$VAULT" == PASS && "$RETENTION" == PASS && "$EGRESS" == PASS && "$STOP" == PASS ]]; then
  exit 0
fi
exit 1
