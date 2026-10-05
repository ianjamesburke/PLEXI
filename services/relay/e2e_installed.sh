#!/usr/bin/env bash
# Installed-build check for the phone relay.
# The relay process is the Docker image. The desktop is plexi-pr-<PR>.
# Records PASS/FAIL per check. Does not deploy anywhere.
set -u

PR="${1:-2685}"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="${PLEXI_BIN:-plexi-pr-${PR}}"
PORT="${RELAY_E2E_PORT:-8792}"
MOCK_PORT="${MOCK_PORT:-8766}"
TTL="${RELAY_UNDELIVERED_TTL:-8}"
CANARY="relay-canary-$(date +%s)-$$-do-not-log"
PROFILE="$HOME/.plexi-pr-${PR}"
BASE="http://127.0.0.1:${PORT}"
URL="ws://127.0.0.1:${PORT}/v1/desktop"
LOG="$(mktemp)"
RESULTS=()
CONNECT_PID=""
MOCK_PID=""
CONTAINER=""

pass() { echo "PASS  $1"; RESULTS+=("PASS  $1"); }
fail() { echo "FAIL  $1 — $2"; RESULTS+=("FAIL  $1 — $2"); }

cleanup() {
  if [[ -n "$CONNECT_PID" ]]; then kill "$CONNECT_PID" 2>/dev/null || true; fi
  if [[ -n "$CONTAINER" ]]; then docker rm -f "$CONTAINER" >/dev/null 2>&1 || true; fi
  if [[ -n "$MOCK_PID" ]]; then kill "$MOCK_PID" 2>/dev/null || true; fi
  if command -v "$BIN" >/dev/null 2>&1; then
    "$BIN" host stop >/dev/null 2>&1 || true
  fi
  rm -f "$LOG" /tmp/relay-e2e-body /tmp/relay-e2e-headers
}
trap cleanup EXIT

need() {
  if ! command -v "$1" >/dev/null 2>&1; then
    fail "prerequisite" "$1 is not installed"
    printf '%s\n' "${RESULTS[@]}"
    exit 1
  fi
}

need docker
need "$BIN"
need python3
need curl

python3 "$ROOT/services/relay/mock_openai.py" >"$LOG.mock" 2>&1 &
MOCK_PID=$!
for _ in $(seq 1 30); do
  curl -sf "http://127.0.0.1:${MOCK_PORT}/" >/dev/null && break
  sleep 0.1
done

mkdir -p "$PROFILE"
python3 - "$PROFILE/config.toml" "$MOCK_PORT" <<'PY'
import pathlib, re, sys
path, port = sys.argv[1], sys.argv[2]
file = pathlib.Path(path)
text = file.read_text() if file.exists() else "[ai]\nbackend = \"openrouter\"\n"
text = text.replace('backend = "openrouter"', 'backend = "local"', 1)
text = re.sub(r"(?ms)^\[ai\.local\]\n.*?(?=^\[|\Z)", "", text)
block = (
    "[ai.local]\n"
    f'base_url = "http://127.0.0.1:{port}"\n'
    'model_low = "mock"\n'
    'model_medium = "mock"\n'
    'model_high = "mock"\n'
)
file.write_text(text.rstrip() + "\n\n" + block + "\n")
PY

export HOME
unset PLEXI_PANE_ID PLEXI_CONTEXT_ID PLEXI_CONTEXT_ROOT PLEXI_RUNNING PLEXI_SOCKET || true
export PLEXI_CHANNEL="pr-${PR}"

"$BIN" host stop >/dev/null 2>&1 || true
if "$BIN" host start --pane 'cwd=/tmp' --timeout-secs 30 >"$LOG.host" 2>&1; then
  pass "host start"
else
  fail "host start" "see $LOG.host"
fi
if "$BIN" host status --json | python3 -c 'import json,sys; body=json.load(sys.stdin); raise SystemExit(0 if body.get("ready") else 1)'; then
  pass "host ready"
else
  fail "host ready" "$("$BIN" host status --json 2>&1 | head -c 300)"
fi
if "$BIN" app open assistant >"$LOG.open" 2>&1; then
  pass "assistant pane"
else
  fail "assistant pane" "$(tail -c 300 "$LOG.open")"
fi

IMAGE="plexi-relay-e2e:${PR}"
if docker build -f "$ROOT/services/relay/Dockerfile" -t "$IMAGE" "$ROOT" >"$LOG.docker-build" 2>&1; then
  pass "docker build"
else
  fail "docker build" "see $LOG.docker-build"
  printf '%s\n' "${RESULTS[@]}"
  exit 1
fi
CONTAINER="plexi-relay-e2e-${PR}"
docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
if docker run -d --name "$CONTAINER" --network host \
  -e "RELAY_UNDELIVERED_TTL=${TTL}" -e PORT="$PORT" -e RELAY_HOST=0.0.0.0 \
  "$IMAGE" >/dev/null; then
  pass "docker run"
else
  fail "docker run" "container did not start"
  printf '%s\n' "${RESULTS[@]}"
  exit 1
fi
for _ in $(seq 1 40); do
  curl -sf "$BASE/healthz" >/dev/null && break
  sleep 0.2
done
if curl -sf "$BASE/healthz" >/dev/null; then
  pass "relay health"
else
  fail "relay health" "healthz did not answer"
fi
if curl -sf "$BASE/" | grep -q "Pairing code" && curl -sf "$BASE/app.js" | grep -q "waiting on desktop"; then
  pass "phone page served"
else
  fail "phone page served" "pair page or app.js missing"
fi

export PLEXI_RELAY_URL="$URL"
"$BIN" relay connect --url "$URL" >"$LOG.connect" 2>&1 &
CONNECT_PID=$!
STATUS="$PROFILE/relay-status.json"
CODE=""
PAIRING=""
for _ in $(seq 1 80); do
  if [[ -f "$STATUS" ]]; then
    CODE="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("code") or "")' "$STATUS")"
    PAIRING="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("pairing_id") or "")' "$STATUS")"
    if [[ -n "$CODE" && -n "$PAIRING" ]]; then break; fi
  fi
  sleep 0.1
done
if [[ -n "$CODE" ]]; then pass "pairing code"; else fail "pairing code" "status never showed a code"; fi

REDEEM="$(curl -sf -X POST "$BASE/api/pair" -H 'content-type: application/json' -d "{\"code\":\"$CODE\",\"label\":\"curl-phone\"}" || true)"
if python3 -c 'import json,sys; raise SystemExit(0 if json.loads(sys.argv[1]).get("status")=="pending_desktop" else 1)' "$REDEEM"; then
  pass "phone redeem"
else
  fail "phone redeem" "$REDEEM"
fi
for _ in $(seq 1 50); do
  PHASE="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("phase") or "")' "$STATUS" 2>/dev/null || true)"
  [[ "$PHASE" == "pending_confirm" ]] && break
  sleep 0.1
done
if "$BIN" relay confirm "$PAIRING" >/dev/null; then
  pass "desktop confirm"
else
  fail "desktop confirm" "confirm command failed"
fi
COOKIE=""
for _ in $(seq 1 50); do
  curl -sD /tmp/relay-e2e-headers -o /tmp/relay-e2e-body "$BASE/api/pair/$PAIRING" >/dev/null || true
  if python3 -c 'import json; raise SystemExit(0 if json.load(open("/tmp/relay-e2e-body")).get("status")=="confirmed" else 1)'; then
    COOKIE="$(python3 -c 'import pathlib; text=pathlib.Path("/tmp/relay-e2e-headers").read_text();
line=next((l for l in text.splitlines() if l.lower().startswith("set-cookie:")), "");
print(line.split(":",1)[1].split(";",1)[0].strip().split("=",1)[1] if "=" in line else "")')"
    [[ -n "$COOKIE" ]] && break
  fi
  sleep 0.1
done
if [[ -n "$COOKIE" ]]; then pass "phone session"; else fail "phone session" "no cookie after confirm"; fi
SAVED_DEVICE="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("device_id") or "")' "$STATUS" 2>/dev/null || true)"

post_turn() {
  local id="$1" text="$2"
  curl -s -o /tmp/relay-e2e-body -w '%{http_code}' -X POST "$BASE/api/turns" \
    -H "cookie: plexi_phone=$COOKIE" -H 'content-type: application/json' \
    -d "{\"schema_version\":1,\"request_id\":\"$id\",\"content\":[{\"type\":\"text\",\"text\":$(python3 -c 'import json,sys; print(json.dumps(sys.argv[1]))' "$text")}]}"
}

saw_reply() {
  local page="$1" id="$2" needle="$3"
  python3 -c 'import json,sys; page=json.loads(sys.argv[1]); rid, needle = sys.argv[2], sys.argv[3];
raise SystemExit(0 if any(e.get("kind")=="assistant_reply" and e.get("request_id")==rid and needle in (e.get("text") or "") for e in page.get("events") or []) else 1)' "$page" "$id" "$needle"
}

CODE_TURN="$(post_turn req-round "say hello $CANARY")"
if [[ "$CODE_TURN" == "202" ]]; then pass "turn accepted"; else fail "turn accepted" "http $CODE_TURN $(cat /tmp/relay-e2e-body)"; fi
ROUND_OK=""
for _ in $(seq 1 160); do
  PAGE="$(curl -sf "$BASE/api/conversation?after=0" -H "cookie: plexi_phone=$COOKIE" || true)"
  if saw_reply "$PAGE" req-round "mock-reply"; then ROUND_OK=1; break; fi
  sleep 0.25
done
if [[ -n "$ROUND_OK" ]]; then pass "round trip"; else fail "round trip" "mock reply did not arrive: ${PAGE:0:400}"; fi

CODE_TOOL="$(post_turn req-approve "APPROVAL-TOOL $CANARY")"
if [[ "$CODE_TOOL" == "202" ]]; then pass "approval turn accepted"; else fail "approval turn accepted" "http $CODE_TOOL"; fi
WAIT_OK=""
for _ in $(seq 1 160); do
  PAGE="$(curl -sf "$BASE/api/conversation?after=0" -H "cookie: plexi_phone=$COOKIE" || true)"
  if python3 -c 'import json,sys; page=json.loads(sys.argv[1]);
raise SystemExit(0 if any(e.get("request_id")=="req-approve" and e.get("state")=="waiting_for_permission" and "waiting on desktop" in (e.get("status") or "") for e in page.get("events") or []) else 1)' "$PAGE"; then
    WAIT_OK=1
    break
  fi
  sleep 0.25
done
if [[ -n "$WAIT_OK" ]]; then pass "waiting on desktop"; else fail "waiting on desktop" "receipt did not appear"; fi
APPROVE_CODE="$(curl -s -o /tmp/relay-e2e-body -w '%{http_code}' -X POST "$BASE/api/approvals" -H "cookie: plexi_phone=$COOKIE" -H 'content-type: application/json' -d '{"request_id":"req-approve"}')"
if [[ "$APPROVE_CODE" == "403" ]] && grep -q "waiting on desktop" /tmp/relay-e2e-body; then
  pass "phone cannot approve"
else
  fail "phone cannot approve" "http $APPROVE_CODE $(cat /tmp/relay-e2e-body)"
fi

CODE_HOLD="$(post_turn req-hold "HOLD-FOR-TTL $CANARY")"
if [[ "$CODE_HOLD" == "202" ]]; then pass "held turn accepted"; else fail "held turn accepted" "http $CODE_HOLD"; fi
sleep 1
if "$BIN" host stop >"$LOG.stop" 2>&1; then
  pass "host stop"
else
  fail "host stop" "$(tail -c 200 "$LOG.stop")"
fi
OFFLINE=""
for _ in $(seq 1 40); do
  STATUS_JSON="$(curl -sf "$BASE/api/status" -H "cookie: plexi_phone=$COOKIE" || true)"
  if python3 -c 'import json,sys; body=json.loads(sys.argv[1]); raise SystemExit(0 if body.get("host")=="desktop_offline" and body.get("message")=="desktop offline" else 1)' "$STATUS_JSON"; then
    OFFLINE=1
    break
  fi
  sleep 0.25
done
if [[ -n "$OFFLINE" ]]; then pass "desktop offline"; else fail "desktop offline" "$STATUS_JSON"; fi
EXPIRED=""
for _ in $(seq 1 40); do
  PAGE="$(curl -sf "$BASE/api/conversation?after=0" -H "cookie: plexi_phone=$COOKIE" || true)"
  if python3 -c 'import json,sys; page=json.loads(sys.argv[1]);
raise SystemExit(0 if any(e.get("request_id")=="req-hold" and e.get("state")=="expired" for e in page.get("events") or []) else 1)' "$PAGE"; then
    EXPIRED=1
    break
  fi
  sleep 0.5
done
if [[ -n "$EXPIRED" ]]; then pass "ttl purge"; else fail "ttl purge" "held turn did not expire"; fi

# Desktop is back only so revoke can be sent on the socket.
"$BIN" host start --pane 'cwd=/tmp' --timeout-secs 30 >"$LOG.host2" 2>&1 || true
"$BIN" app open assistant >"$LOG.open2" 2>&1 || true
"$BIN" relay connect --url "$URL" >"$LOG.connect2" 2>&1 &
CONNECT_PID=$!
READY=""
for _ in $(seq 1 80); do
  PHASE="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("phase") or "")' "$STATUS" 2>/dev/null || true)"
  if [[ "$PHASE" == "waiting_for_phone" || "$PHASE" == "pending_confirm" || "$PHASE" == "confirmed" ]]; then READY=1; break; fi
  sleep 0.1
done
DEVICE="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("device_id") or "")' "$STATUS" 2>/dev/null || true)"
if [[ -z "$DEVICE" ]]; then
  # The restarted client has not seen the old device id. Revoke the id from the earlier status if we saved it.
  DEVICE="${SAVED_DEVICE:-}"
fi
if [[ -n "$READY" && -n "$DEVICE" ]]; then
  if "$BIN" relay revoke "$DEVICE" >/dev/null; then
    pass "revoke sent"
  else
    fail "revoke sent" "revoke command failed"
  fi
else
  fail "revoke sent" "desktop did not reconnect or device id missing ($DEVICE)"
fi
sleep 0.4
REVOKED="$(curl -s -o /tmp/relay-e2e-body -w '%{http_code}' "$BASE/api/status" -H "cookie: plexi_phone=$COOKIE")"
if [[ "$REVOKED" == "401" ]]; then pass "revoked phone rejected"; else fail "revoked phone rejected" "http $REVOKED $(cat /tmp/relay-e2e-body)"; fi

if docker logs "$CONTAINER" 2>&1 | grep -q "$CANARY"; then
  fail "canary absent from relay logs" "marker leaked"
else
  pass "canary absent from relay logs"
fi

echo
echo "---- summary ----"
printf '%s\n' "${RESULTS[@]}"
if printf '%s\n' "${RESULTS[@]}" | grep -q '^FAIL'; then
  exit 1
fi
