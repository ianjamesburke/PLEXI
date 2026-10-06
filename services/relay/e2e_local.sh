#!/usr/bin/env bash
# Local loop: relay + `plexi relay connect` (echo assistant) + a curl phone.
# Proves pairing, confirm, a correlated turn, revoke, desktop-offline, and
# that a unique marker never appears in the relay log.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PORT="${RELAY_E2E_PORT:-8791}"
PLEXI_BIN="${PLEXI_BIN:-}"
if [[ -z "$PLEXI_BIN" ]]; then
  if [[ -x "$ROOT/target/debug/plexi" ]]; then
    PLEXI_BIN="$ROOT/target/debug/plexi"
  else
    PLEXI_BIN="plexi"
  fi
fi
LOG="$(mktemp)"
HOME_DIR="$(mktemp -d)"
MARKER="e2e-canary-$(date +%s)-$$-do-not-log"
BASE="http://127.0.0.1:${PORT}"
URL="ws://127.0.0.1:${PORT}/v1/desktop"
CONNECT_PID=""
RELAY_PID=""

cleanup() {
  if [[ -n "$CONNECT_PID" ]]; then kill "$CONNECT_PID" 2>/dev/null || true; fi
  if [[ -n "$RELAY_PID" ]]; then kill "$RELAY_PID" 2>/dev/null || true; fi
  rm -rf "$LOG" "$HOME_DIR" /tmp/relay-e2e-body /tmp/relay-e2e-headers
}
trap cleanup EXIT

export HOME="$HOME_DIR"
unset PLEXI_CHANNEL || true
export PLEXI_RELAY_ASSISTANT=echo
STATUS="$HOME_DIR/.plexi/relay-status.json"

python3 "$ROOT/services/relay/relay.py" --host 127.0.0.1 --port "$PORT" --public-origin "$BASE" >"$LOG" 2>&1 &
RELAY_PID=$!
for _ in $(seq 1 50); do
  curl -sf "$BASE/healthz" >/dev/null && break
  sleep 0.1
done
curl -sf "$BASE/healthz" >/dev/null

if ! command -v "$PLEXI_BIN" >/dev/null 2>&1 && [[ ! -x "$PLEXI_BIN" ]]; then
  echo "plexi binary not found ($PLEXI_BIN); build it before e2e" >&2
  exit 1
fi

"$PLEXI_BIN" relay connect --url "$URL" >"$HOME_DIR/connect.log" 2>&1 &
CONNECT_PID=$!

CODE=""
PAIRING=""
for _ in $(seq 1 50); do
  if [[ -f "$STATUS" ]]; then
    CODE="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("code") or "")' "$STATUS")"
    PAIRING="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("pairing_id") or "")' "$STATUS")"
    if [[ -n "$CODE" && -n "$PAIRING" ]]; then
      break
    fi
  fi
  sleep 0.1
done
test -n "$CODE"
test -n "$PAIRING"

REDEEM="$(curl -sf -X POST "$BASE/api/pair" -H 'content-type: application/json' -d "{\"code\":\"$CODE\",\"label\":\"curl-phone\"}")"
python3 -c 'import json,sys; body=json.loads(sys.argv[1]); raise SystemExit(0 if body.get("status")=="pending_desktop" else 1)' "$REDEEM"

for _ in $(seq 1 50); do
  PHASE="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("phase") or "")' "$STATUS" 2>/dev/null || true)"
  if [[ "$PHASE" == "pending_confirm" ]]; then
    break
  fi
  sleep 0.1
done

"$PLEXI_BIN" relay confirm "$PAIRING" >/dev/null

COOKIE=""
for _ in $(seq 1 50); do
  curl -sD /tmp/relay-e2e-headers -o /tmp/relay-e2e-body "$BASE/api/pair/$PAIRING" >/dev/null
  if python3 -c 'import json; raise SystemExit(0 if json.load(open("/tmp/relay-e2e-body")).get("status")=="confirmed" else 1)'; then
    COOKIE="$(python3 -c 'import pathlib; text=pathlib.Path("/tmp/relay-e2e-headers").read_text();
line=next((l for l in text.splitlines() if l.lower().startswith("set-cookie:")), "");
print(line.split(":",1)[1].split(";",1)[0].strip().split("=",1)[1] if "=" in line else "")')"
    if [[ -n "$COOKIE" ]]; then
      break
    fi
  fi
  sleep 0.1
done
test -n "$COOKIE"

TURN_CODE="$(curl -s -o /tmp/relay-e2e-body -w '%{http_code}' -X POST "$BASE/api/turns" \
  -H "cookie: plexi_phone=$COOKIE" -H 'content-type: application/json' \
  -d "{\"schema_version\":1,\"request_id\":\"req-e2e\",\"conversation_id\":\"not-authoritative\",\"content\":[{\"type\":\"text\",\"text\":\"$MARKER\"}]}")"
test "$TURN_CODE" = "202"

SAW=""
for _ in $(seq 1 50); do
  PAGE="$(curl -sf "$BASE/api/conversation?after=0" -H "cookie: plexi_phone=$COOKIE")"
  if python3 -c 'import json,sys; page=json.loads(sys.argv[1]); marker=sys.argv[2];
ok=any(e.get("kind")=="assistant_reply" and e.get("text")=="echo:"+marker and e.get("request_id")=="req-e2e" for e in page.get("events") or [])
raise SystemExit(0 if ok else 1)' "$PAGE" "$MARKER"; then
    SAW=1
    break
  fi
  sleep 0.1
done
test -n "$SAW"

DEVICE="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("device_id") or "")' "$STATUS")"
test -n "$DEVICE"
kill "$CONNECT_PID"
wait "$CONNECT_PID" 2>/dev/null || true
CONNECT_PID=""
sleep 0.4
OFFLINE="$(curl -s -o /tmp/relay-e2e-body -w '%{http_code}' -X POST "$BASE/api/turns" \
  -H "cookie: plexi_phone=$COOKIE" -H 'content-type: application/json' \
  -d "{\"schema_version\":1,\"request_id\":\"req-off\",\"content\":[{\"type\":\"text\",\"text\":\"$MARKER-offline\"}]}")"
test "$OFFLINE" = "409"
python3 -c 'import json; body=json.load(open("/tmp/relay-e2e-body")); raise SystemExit(0 if body.get("error")=="desktop_offline" and body.get("message")=="desktop offline" else 1)'

"$PLEXI_BIN" relay connect --url "$URL" >"$HOME_DIR/connect-again.log" 2>&1 &
CONNECT_PID=$!
READY=""
for _ in $(seq 1 50); do
  PHASE="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("phase") or "")' "$STATUS" 2>/dev/null || true)"
  if [[ "$PHASE" == "waiting_for_phone" || "$PHASE" == "pending_confirm" || "$PHASE" == "confirmed" ]]; then
    READY=1
    break
  fi
  sleep 0.1
done
test -n "$READY"
"$PLEXI_BIN" relay revoke "$DEVICE" >/dev/null
sleep 0.3
REVOKED="$(curl -s -o /dev/null -w '%{http_code}' "$BASE/api/status" -H "cookie: plexi_phone=$COOKIE")"
test "$REVOKED" = "401"

if grep -q "$MARKER" "$LOG"; then
  echo "canary leaked into relay log" >&2
  exit 1
fi
echo "e2e ok (pairing, echo turn, revoke, canary absent from relay log)"
