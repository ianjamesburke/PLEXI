#!/usr/bin/env bash
# Installed-binary checks for cloud basics.
# Usage: scripts/cloud-basics-e2e.sh <PR>
#
# Uses a temp HOME. The channel profile is ${HOME}/.<binary-name>, derived
# from the installed plexi-pr-<PR> binary. Never touches the OS keychain and
# never opens a secret dialog: it does not call secret commands, and the host
# is started with --background so it does not activate a window.

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
echo "CHECK no-account: $ACCOUNT"

if [[ "$RETENTION" == PASS && "$ACCOUNT" == PASS ]]; then
  exit 0
fi
exit 1
