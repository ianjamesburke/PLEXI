#!/usr/bin/env bash
# Installed-binary checks for the Agents API.
# Usage: scripts/e2e_agents_api_installed.sh <PR>
# With PLEXI_E2E_SHIM set, the harness binary is used instead of plexi-pr-<PR>.
# Profile dir is derived from that binary's name.
# Exits 0 only when create/list, start (spawn), stop (finish), tagged ledger
# claim, subset delegation, an ungranted-capability denial, and the existing
# permission list all pass.

set -euo pipefail

if [[ -z "${DISPLAY:-}" ]] && command -v xvfb-run >/dev/null 2>&1; then
  exec xvfb-run -a "$0" "$@"
fi

if [[ -n "${PLEXI_E2E_SHIM:-}" ]]; then
  # shellcheck disable=SC1090
  source "${PLEXI_E2E_SHIM:?}"
else
  PR="${1:?usage: scripts/e2e_agents_api_installed.sh <PR>}"
  BIN="plexi-pr-${PR}"
  if ! command -v "$BIN" >/dev/null 2>&1; then
    echo "FAIL: $BIN is not on PATH. Run: just pr-install ${PR}"
    exit 1
  fi
  BIN_PATH="$(command -v "$BIN")"
  BIN_NAME="$(basename "$BIN_PATH")"
  PROFILE="${HOME}/.${BIN_NAME}"
  CHANNEL_DIR=".${BIN_NAME}"
  unset PLEXI_CHANNEL
fi
unset PLEXI_SOCKET

WORKDIR="$(mktemp -d)"
HOST_STARTED=0

cleanup() {
  local status=$?
  if [[ "$HOST_STARTED" == 1 ]]; then
    "$BIN" host stop >/dev/null 2>&1 || true
  fi
  rm -rf "$WORKDIR"
  exit "$status"
}
trap cleanup EXIT

cd "$WORKDIR"
echo "workspace: $WORKDIR"
echo "binary: $BIN_PATH"
echo "profile: $PROFILE"

"$BIN" workspace init

# A mapped window is required: the ask-tier approval is a real pointer click.
if ! "$BIN" host start --ephemeral --timeout-secs 90 >"$WORKDIR/host-start.out" 2>"$WORKDIR/host-start.err"; then
  echo "FAIL: host start"
  cat "$WORKDIR/host-start.err" >&2 || true
  exit 1
fi
HOST_STARTED=1

python_json() {
  python3 -c "$1"
}

must_ok() {
  local label="$1"
  local body="$2"
  if ! python3 -c 'import json,sys; body=json.loads(sys.argv[1]); assert body.get("ok") is True' "$body"; then
    echo "FAIL: $label"
    echo "$body"
    exit 1
  fi
}

must_code() {
  local label="$1"
  local body="$2"
  local code="$3"
  if ! python3 -c 'import json,sys; body=json.loads(sys.argv[1]); assert body.get("error_code")==sys.argv[2]' "$body" "$code"; then
    echo "FAIL: $label (wanted $code)"
    echo "$body"
    exit 1
  fi
}

cli() {
  local out
  set +e
  out="$("$BIN" "$@" 2>/tmp/agents-api-e2e.err)"
  local status=$?
  set -e
  if [[ ! -s /tmp/agents-api-e2e.err ]]; then
    :
  else
    cat /tmp/agents-api-e2e.err >&2
  fi
  printf '%s' "$out"
  return "$status"
}

mcp() {
  local token="$1"
  local tool="$2"
  local args="$3"
  python3 - "$PORT" "$token" "$tool" "$args" <<'PY'
import json, sys, urllib.request, urllib.error
port, token, tool, args = sys.argv[1:5]
body = json.dumps({
    "jsonrpc": "2.0",
    "id": 1,
    "method": "tools/call",
    "params": {"name": tool, "arguments": json.loads(args)},
}).encode()
req = urllib.request.Request(
    f"http://127.0.0.1:{port}/mcp",
    data=body,
    headers={"Authorization": f"Bearer {token}", "Content-Type": "application/json"},
)
try:
    with urllib.request.urlopen(req, timeout=20) as resp:
        raw = resp.read().decode()
except urllib.error.HTTPError as exc:
    raw = exc.read().decode()
print(raw)
PY
}

mcp_text() {
  python3 -c 'import json,sys; body=json.loads(sys.stdin.read()); print(body["result"]["content"][0]["text"])'
}

echo "=== check 1: create and list heads via CLI and MCP ==="
CREATE="$(cli agent head create lead --display-name Lead --description 'Lead agent' \
  --grant agents.ping=allow --grant agents.review=ask --grant agents.list=allow --grant agents.create=allow --json)"
must_ok "create head" "$CREATE"
test -f "$WORKDIR/.plexi/agents/lead/AGENT.md"
test -f "$WORKDIR/.plexi/agents/lead/settings.toml"
test -f "$WORKDIR/.plexi/agents/lead/head.json"
if [[ -e "$WORKDIR/$CHANNEL_DIR/agents" ]]; then
  echo "FAIL: agent files landed in the channel dir"
  exit 1
fi
LIST="$(cli agent head list --json)"
must_ok "cli list" "$LIST"
python3 -c 'import json,sys; body=json.loads(sys.argv[1]); ids=[h["id"] for h in body["heads"]]; assert ids==["lead"]' "$LIST"

echo "=== check 2: spawn, idempotent claim, conflict, ledger tags ==="
SPAWN="$(cli agent run spawn --head lead --admission adm-1 --client-ref acme --kind output --input-tokens 11 --output-tokens 4 --json)"
must_ok "spawn" "$SPAWN"
python3 -c 'import json,sys; body=json.loads(sys.argv[1]); assert body["ledger"]["client_ref"]=="acme"; assert body["ledger"]["kind"]=="output"; assert body["ledger"]["input_tokens"]==11; assert body["ledger"]["output_tokens"]==4; assert body["idempotent"] is False' "$SPAWN"
RUN="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["run"]["id"])' "$SPAWN")"
TOKEN="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["run_token"])' "$SPAWN")"
PORT="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["mcp_port"])' "$SPAWN")"
test -n "$PORT" && test "$PORT" != "None" && test "$PORT" != "null"
AGAIN="$(cli agent run spawn --head lead --admission adm-1 --client-ref acme --kind output --input-tokens 11 --output-tokens 4 --json)"
must_ok "idempotent spawn" "$AGAIN"
python3 -c 'import json,sys; body=json.loads(sys.argv[1]); assert body["idempotent"] is True; assert body["run"]["id"]==sys.argv[2]' "$AGAIN" "$RUN"
set +e
CONFLICT="$(cli agent run spawn --head lead --admission adm-2 --json)"
set -e
must_code "assignment conflict" "$CONFLICT" "assignment_conflict"
LEDGER="$PROFILE/ai-ledger.jsonl"
test -f "$LEDGER"
python3 -c 'import json,sys; rows=[json.loads(line) for line in open(sys.argv[1]) if line.strip()]; tagged=[row for row in rows if row.get("run_id")==sys.argv[2]]; assert len(tagged)==1, tagged; row=tagged[0]; assert row.get("client_ref")=="acme"; assert row.get("kind")=="output"; assert row.get("input_tokens")==11; assert row.get("output_tokens")==4; assert row.get("agent_id")=="agent:lead"' "$LEDGER" "$RUN"

TOOLS="$(python3 - "$PORT" "$TOKEN" <<'PY'
import json, sys, urllib.request
port, token = sys.argv[1:3]
body = json.dumps({"jsonrpc":"2.0","id":1,"method":"tools/list"}).encode()
req = urllib.request.Request(
    f"http://127.0.0.1:{port}/mcp",
    data=body,
    headers={"Authorization": f"Bearer {token}", "Content-Type": "application/json"},
)
with urllib.request.urlopen(req, timeout=20) as resp:
    print(resp.read().decode())
PY
)"
python3 -c 'import json,sys; body=json.loads(sys.argv[1]); names={t["name"] for t in body["result"]["tools"]}; assert "agents.list" in names and "agents.create" in names' "$TOOLS"

LIST_MCP="$(mcp "$TOKEN" agents.list '{}')"
LIST_TEXT="$(printf '%s' "$LIST_MCP" | mcp_text)"
python3 -c 'import json,sys; body=json.loads(sys.argv[1]);
assert "heads" in body, "agents.list missing heads: " + sys.argv[1]
ids=[h["id"] for h in body["heads"]]; assert body["ok"] is True, body; assert ids==["lead"]' "$LIST_TEXT"

CREATE_MCP="$(mcp "$TOKEN" agents.create '{"name":"second","grants":["agents.ping=allow"]}')"
CREATE_TEXT="$(printf '%s' "$CREATE_MCP" | mcp_text)"
must_ok "mcp create second" "$CREATE_TEXT"
LIST2="$(cli agent head list --json)"
python3 -c 'import json,sys; ids=sorted(h["id"] for h in json.loads(sys.argv[1])["heads"]); assert ids==["lead","second"]' "$LIST2"

echo "=== check 3: delegation subset, child tool enforcement ==="
set +e
ROGUE="$(mcp "$TOKEN" agents.delegate '{"name":"rogue","parent_run":"'"$RUN"'","grants":["agents.admin=allow"]}')"
set -e
ROGUE_TEXT="$(printf '%s' "$ROGUE" | mcp_text)"
must_code "delegate admin" "$ROGUE_TEXT" "permission_denied"
test ! -d "$WORKDIR/.plexi/agents/rogue"

SCOUT="$(mcp "$TOKEN" agents.delegate '{"name":"scout","parent_run":"'"$RUN"'","grants":["agents.ping=allow","agents.review=ask"]}')"
SCOUT_TEXT="$(printf '%s' "$SCOUT" | mcp_text)"
must_ok "delegate scout" "$SCOUT_TEXT"
CHILD="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["run"]["id"])' "$SCOUT_TEXT")"
CHILD_TOKEN="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["run_token"])' "$SCOUT_TEXT")"
python3 -c 'import json,sys; card=json.load(open(sys.argv[1])); assert card["temporary"] is True; assert card["reports_to"]=="lead"; assert all(g["tool"]!="agents.admin" for g in card["grants"])' "$WORKDIR/.plexi/agents/scout/head.json"

PING="$(mcp "$CHILD_TOKEN" agents.ping '{}')"
PING_TEXT="$(printf '%s' "$PING" | mcp_text)"
python3 -c 'import json,sys; body=json.loads(sys.argv[1]); assert body["ok"] is True and body["actor_id"]=="agent:scout" and body["tool"]=="agents.ping"' "$PING_TEXT"

ADMIN="$(mcp "$CHILD_TOKEN" agents.admin '{}')"
ADMIN_TEXT="$(printf '%s' "$ADMIN" | mcp_text)"
must_code "child admin" "$ADMIN_TEXT" "permission_denied"

echo "=== check 4: child ask uses the existing permission list ==="
REVIEW="$(mcp "$CHILD_TOKEN" agents.review '{}')"
REVIEW_TEXT="$(printf '%s' "$REVIEW" | mcp_text)"
must_code "child review" "$REVIEW_TEXT" "permission_required"
PENDING="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["pending_request_id"])' "$REVIEW_TEXT")"
PERM="$(cli assistant permission list)"
python3 -c 'import json,sys; body=json.loads(sys.argv[1]); rows=body["pending"]; assert any(row.get("pending_request_id")==sys.argv[2] and row.get("actor_id")=="agent:scout" for row in rows)' "$PERM" "$PENDING"
# shellcheck disable=SC1091
source "$(cd "$(dirname "$0")" && pwd)/e2e/human.sh"
HUMAN_APPROVE "$PENDING"
REVIEW2="$(mcp "$CHILD_TOKEN" agents.review '{}')"
REVIEW2_TEXT="$(printf '%s' "$REVIEW2" | mcp_text)"
python3 -c 'import json,sys; body=json.loads(sys.argv[1]); assert body["ok"] is True and body["tool"]=="agents.review" and body["actor_id"]=="agent:scout"' "$REVIEW2_TEXT"

STANDING="$(cli agent head list --json)"
python3 -c 'import json,sys; ids=sorted(h["id"] for h in json.loads(sys.argv[1])["heads"]); assert "scout" not in ids; assert ids==["lead","second"]' "$STANDING"
ALL="$(cli agent head list --all --json)"
python3 -c 'import json,sys; ids={h["id"] for h in json.loads(sys.argv[1])["heads"]}; assert {"lead","second","scout"} <= ids' "$ALL"

AUDIT="$(cli assistant permission list)"
python3 -c 'import json,sys; body=json.loads(sys.argv[1]); assert any(row.get("decision")=="deny" and row.get("resource_id")=="agents.admin" for row in body["audit"])' "$AUDIT"

echo "=== check 5: stop the run, then a new admission can start ==="
FINISH="$(cli agent run finish "$RUN" --json)"
must_ok "stop run" "$FINISH"
python3 -c 'import json,sys; body=json.loads(sys.argv[1]); run=body["run"]; assert run["id"]==sys.argv[2]; assert run["state"]=="finished"; assert run["active"] is False' "$FINISH" "$RUN"
SHOW="$(cli agent run show "$RUN" --json)"
must_ok "show stopped run" "$SHOW"
python3 -c 'import json,sys; body=json.loads(sys.argv[1]); assert body["run"]["state"]=="finished" and body["run"]["active"] is False' "$SHOW"
NEXT="$(cli agent run spawn --head lead --admission adm-3 --client-ref acme --kind output --json)"
must_ok "start after stop" "$NEXT"
python3 -c 'import json,sys; body=json.loads(sys.argv[1]); assert body["idempotent"] is False; assert body["run"]["id"]!=sys.argv[2]; assert body["run"]["active"] is True and body["run"]["state"]=="running"' "$NEXT" "$RUN"
LIST_RUNS="$(cli agent run list --json)"
must_ok "list runs" "$LIST_RUNS"
python3 -c 'import json,sys; body=json.loads(sys.argv[1]); states={row["id"]: row["state"] for row in body["runs"]}; assert states[sys.argv[2]]=="finished"; assert any(row["state"]=="running" and row["head_id"]=="lead" for row in body["runs"])' "$LIST_RUNS" "$RUN"

echo "CHECK 1 create/list CLI+MCP: PASS"
echo "CHECK 2 start (spawn) ledger claim: PASS"
echo "CHECK 3 ungranted capability denied: PASS"
echo "CHECK 4 pending approval seam: PASS"
echo "CHECK 5 stop (finish) then start: PASS"
echo "e2e_agents_api_installed: PASS"
