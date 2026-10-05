#!/usr/bin/env bash
# Installed-build check for the Assistant AI ledger.
#
# Starts scripts/e2e/ledger/mock_openrouter.py, points a running
# `plexi-pr-<N>` host at it with PLEXI_OPENROUTER_BASE_URL, and drives real
# `assistant send` turns. Asserts ai-ledger.jsonl rows and `ledger summary`.
#
# Usage:
#   scripts/e2e/ledger/run.sh
#   PLEXI_BIN=plexi-pr-2683 scripts/e2e/ledger/run.sh
#
# The channel profile is derived from the binary name (`plexi-pr-2683` →
# ~/.plexi-pr-2683). The script truncates that profile's ai-ledger.jsonl so
# the summary counts only this run. It stops the host on exit.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
BIN="${PLEXI_BIN:-plexi-pr-2683}"
if ! command -v "$BIN" >/dev/null 2>&1; then
  echo "FAIL: $BIN is not on PATH. Install the PR build first (just pr-install)." >&2
  exit 1
fi

name="$(basename "$BIN")"
channel="${name#plexi-}"
if [[ "$channel" == "$name" || -z "$channel" ]]; then
  PROFILE="${HOME}/.plexi"
else
  PROFILE="${HOME}/.plexi-${channel}"
fi
LEDGER="${PROFILE}/ai-ledger.jsonl"
CONFIG="${PROFILE}/config.toml"
SOCK="${PROFILE}/notify.sock"
MOCK_PY="${ROOT}/scripts/e2e/ledger/mock_openrouter.py"
LOG="${PROFILE}/e2e-ledger.log"
mkdir -p "$PROFILE"

# Drive commands must not inherit another pane's socket or identity.
unset PLEXI_SOCKET PLEXI_PANE_ID PLEXI_CONTEXT_ID PLEXI_CONTEXT_ROOT PLEXI_RUNNING || true

if [[ ! -f "$CONFIG" ]] || ! grep -q 'backend = "openrouter"' "$CONFIG"; then
  echo "FAIL: ${CONFIG} is missing an [ai] openrouter backend. Install the channel so default-config.toml is seeded." >&2
  exit 1
fi

cleanup() {
  if [[ -n "${HOST_STARTED:-}" ]]; then
    "$BIN" host stop >/dev/null 2>&1 || true
  fi
  if [[ -n "${MOCK_PID:-}" ]]; then
    kill "$MOCK_PID" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

MOCK_OUT="$(mktemp)"
python3 "$MOCK_PY" >"$MOCK_OUT" &
MOCK_PID=$!
for _ in $(seq 1 50); do
  if grep -q '^PORT=' "$MOCK_OUT" 2>/dev/null; then
    break
  fi
  if ! kill -0 "$MOCK_PID" 2>/dev/null; then
    echo "FAIL: mock server exited" >&2
    cat "$MOCK_OUT" >&2 || true
    exit 1
  fi
  sleep 0.1
done
PORT="$(sed -n 's/^PORT=//p' "$MOCK_OUT" | head -n 1)"
if [[ -z "${PORT}" ]]; then
  echo "FAIL: mock did not print PORT" >&2
  exit 1
fi
export OPENROUTER_API_KEY="${OPENROUTER_API_KEY:-sk-e2e-ledger}"
export PLEXI_OPENROUTER_BASE_URL="http://127.0.0.1:${PORT}/v1"
echo "mock ${PLEXI_OPENROUTER_BASE_URL}"

# A live host keeps the ledger file open. Stop it before truncating.
"$BIN" host stop >/dev/null 2>&1 || true
: >"$LEDGER"

echo "starting ${BIN} host (log ${LOG})"
# Inherit the mock base URL. host start forwards the parent environment
# except the stripped PLEXI routing vars.
if ! "$BIN" host start --background >"$LOG" 2>&1; then
  echo "FAIL: host start" >&2
  cat "$LOG" >&2 || true
  exit 1
fi
HOST_STARTED=1

ready=0
for _ in $(seq 1 40); do
  if "$BIN" host status --json 2>/dev/null | grep -q '"ready":true'; then
    ready=1
    break
  fi
  sleep 0.5
done
if [[ "$ready" != 1 ]]; then
  echo "FAIL: host did not become ready" >&2
  "$BIN" host status --json >&2 || true
  tail -n 80 "$LOG" >&2 || true
  exit 1
fi

echo "opening assistant"
if ! "$BIN" app open assistant >"${PROFILE}/e2e-open-assistant.log" 2>&1; then
  echo "FAIL: app open assistant" >&2
  cat "${PROFILE}/e2e-open-assistant.log" >&2 || true
  exit 1
fi

send() {
  local label="$1"
  shift
  echo "send ${label}"
  if ! "$BIN" assistant send "$@" --json >"${PROFILE}/e2e-send-${label}.json"; then
    echo "FAIL: assistant send ${label}" >&2
    cat "${PROFILE}/e2e-send-${label}.json" >&2 || true
    exit 1
  fi
}

send stream-a --text "LEDGER_STREAM_A" --client narrative
send stream-b --text "LEDGER_STREAM_B" --client narrative
send system --text "LEDGER_SYSTEM" --client narrative --kind system
send none --text "LEDGER_NONE" --client narrative
send json --text "LEDGER_JSON" --client du

export LEDGER
python3 - <<'PY'
import json, os, sys
path = os.environ["LEDGER"]
rows = []
with open(path) as fh:
    for line in fh:
        line = line.strip()
        if line:
            rows.append(json.loads(line))
print("LEDGER_ROWS", json.dumps(rows, indent=2))
expected = [
    ("narrative", "output", 194, 12),
    ("narrative", "output", 10, 4),
    ("narrative", "system", 80, 15),
    ("narrative", "output", None, None),
    ("du", "output", 50, 7),
]
if len(rows) != len(expected):
    sys.exit(f"FAIL: expected {len(expected)} ledger rows, got {len(rows)}")
for row, (client, kind, inp, out) in zip(rows, expected):
    if row.get("client") != client or row.get("kind") != kind:
        sys.exit(f"FAIL: tags {row.get('client')}/{row.get('kind')} != {client}/{kind}")
    if row.get("input_tokens") != inp or row.get("output_tokens") != out:
        sys.exit(
            f"FAIL: tokens {row.get('input_tokens')}/{row.get('output_tokens')} != {inp}/{out}"
        )
    if inp is None and (row.get("input_tokens") == 0 or row.get("output_tokens") == 0):
        sys.exit("FAIL: missing usage was recorded as 0")
print("LEDGER_ROWS_OK")
PY

echo "summary by client"
"$BIN" ledger summary --by client --json | tee "${PROFILE}/e2e-summary-client.json"
echo "summary by kind"
"$BIN" ledger summary --by kind --json | tee "${PROFILE}/e2e-summary-kind.json"

export PROFILE
python3 - <<'PY'
import json, os, sys
profile = os.environ["PROFILE"]
by_client = json.load(open(f"{profile}/e2e-summary-client.json"))
by_kind = json.load(open(f"{profile}/e2e-summary-kind.json"))

def group(report, key_name, key):
    for item in report["groups"]:
        if item.get(key_name) == key:
            return item
    sys.exit(f"FAIL: no {key_name}={key} in {report}")

if by_client.get("by") != "client":
    sys.exit(f"FAIL: summary --by client reported {by_client.get('by')}")
narrative = group(by_client, "client", "narrative")
du = group(by_client, "client", "du")
# narrative: 194+10+80 prompt, 12+4+15 completion; the null row adds a run only.
if narrative["runs"] != 4 or narrative["input_tokens"] != 284 or narrative["output_tokens"] != 31:
    sys.exit(f"FAIL: narrative aggregate {narrative}")
if du["runs"] != 1 or du["input_tokens"] != 50 or du["output_tokens"] != 7:
    sys.exit(f"FAIL: du aggregate {du}")

if by_kind.get("by") != "kind":
    sys.exit(f"FAIL: summary --by kind reported {by_kind.get('by')}")
output = group(by_kind, "kind", "output")
system = group(by_kind, "kind", "system")
# output: stream A, stream B, null, json → 194+10+50 / 12+4+7
if output["runs"] != 4 or output["input_tokens"] != 254 or output["output_tokens"] != 23:
    sys.exit(f"FAIL: output aggregate {output}")
if system["runs"] != 1 or system["input_tokens"] != 80 or system["output_tokens"] != 15:
    sys.exit(f"FAIL: system aggregate {system}")
print("SUMMARY_OK")
PY

echo "PASS"
