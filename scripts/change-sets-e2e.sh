#!/usr/bin/env bash
# Installed-binary check for text-editor change sets.
# Propose must not touch the file. Accept writes it and records the agent.
# Revert restores it. A later disk edit is stale until refresh.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${PLEXI_BIN:-$ROOT/target/release/plexi}"
if [[ ! -x "$BIN" ]]; then
  echo "FAIL missing binary $BIN" >&2
  exit 1
fi

WORK="$(mktemp -d)"
cleanup() { rm -rf "$WORK"; }
trap cleanup EXIT

export HOME="$WORK/home"
mkdir -p "$HOME"
unset PLEXI_SOCKET PLEXI_CHANNEL || true

FILE="$WORK/draft.txt"
printf 'alpha\n' > "$FILE"
AGENT="editor-bot"
PASS=0
FAIL=0

ok() { echo "PASS $1"; PASS=$((PASS + 1)); }
bad() { echo "FAIL $1"; FAIL=$((FAIL + 1)); }

bytes() { sha256sum "$FILE" | awk '{print $1}'; }

BEFORE="$(bytes)"
set +e
PROPOSE_OUT="$("$BIN" changes propose --agent "$AGENT" --file "$FILE" --old alpha --new beta 2>"$WORK/propose-ungranted.err")"
PROPOSE_CODE=$?
set -e
if [[ "$PROPOSE_CODE" -eq 2 && "$PROPOSE_OUT" == *permission_required* && "$(bytes)" == "$BEFORE" ]]; then
  ok "ungranted propose is permission_required and does not write"
else
  bad "ungranted propose code=$PROPOSE_CODE"
fi

"$BIN" changes allow --agent "$AGENT" --file "$FILE" --old alpha --new beta >/dev/null

PROP="$("$BIN" changes propose --agent "$AGENT" --file "$FILE" --old alpha --new beta)"
CS="$(printf '%s\n' "$PROP" | sed -n 's/^change_set=//p' | head -n 1)"
PREVIEW="$("$BIN" changes preview "$CS")"
if [[ -n "$CS" && "$PROP" == *applied=false* && "$(bytes)" == "$BEFORE" && "$PREVIEW" == *"-alpha"* && "$PREVIEW" == *"+beta"* ]]; then
  ok "propose stays off disk and preview is a diff"
else
  bad "propose/preview"
fi

"$BIN" changes accept "$CS" >/dev/null
AUDIT="$HOME/.plexi/permission-audit.jsonl"
LEDGER="$HOME/.plexi/change-ledger.jsonl"
AFTER="$(printf 'beta\n' | sha256sum | awk '{print $1}')"
if [[ "$(bytes)" == "$AFTER" && -f "$AUDIT" && -f "$LEDGER" ]] \
  && grep -q 'agent:editor-bot' "$AUDIT" \
  && grep -q '"decision":"commit"' "$AUDIT" \
  && grep -q '"agent_id":"agent:editor-bot"' "$LEDGER"; then
  ok "accept writes the file and records the agent"
else
  bad "accept audit/ledger"
fi

"$BIN" changes revert "$CS" >/dev/null
if [[ "$(bytes)" == "$BEFORE" ]] && grep -q '"action":"revert"' "$LEDGER"; then
  ok "revert restores the committed file"
else
  bad "revert"
fi

PROP2="$("$BIN" changes propose --agent "$AGENT" --file "$FILE" --old alpha --new beta)"
CS2="$(printf '%s\n' "$PROP2" | sed -n 's/^change_set=//p' | head -n 1)"
printf 'alpha\nextra\n' > "$FILE"
MUTATED="$(bytes)"
set +e
"$BIN" changes accept "$CS2" >/dev/null 2>"$WORK/stale.err"
STALE_CODE=$?
set -e
STALE_PREVIEW="$("$BIN" changes preview "$CS2")"
if [[ "$STALE_CODE" -eq 3 && "$(bytes)" == "$MUTATED" && "$STALE_PREVIEW" == status=stale* ]]; then
  ok "disk change is stale and accept is refused"
else
  bad "stale accept code=$STALE_CODE"
fi

"$BIN" changes refresh "$CS2" >/dev/null
"$BIN" changes accept "$CS2" >/dev/null
WANT="$(printf 'beta\nextra\n' | sha256sum | awk '{print $1}')"
if [[ "$(bytes)" == "$WANT" ]]; then
  ok "refresh then accept writes the rebased edit"
else
  bad "refresh accept"
fi

echo "$PASS passed, $FAIL failed"
if [[ "$FAIL" -ne 0 ]]; then
  exit 1
fi
