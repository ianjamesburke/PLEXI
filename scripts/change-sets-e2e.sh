#!/usr/bin/env bash
# Installed-binary check for text-editor change sets.
#
# An Assistant edit of the file open in the editor shows a diff and does not
# touch disk until accept. Revert restores the agent hunk. A conflicting edit
# is stale. Twenty pane-key calls while a proposal is pending return in under
# one second and do not accept.
#
# scripts/e2e/human.sh is the human click (HUMAN_APPROVE). It is not on alpha
# yet. When it is absent this script accepts through the CLI and prints
# VERIFIED-VIA-BYPASS. That proves the feature, not the gate.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${PLEXI_BIN:-${1:-$ROOT/target/release/plexi}}"
if [[ ! -x "$BIN" ]]; then
  echo "FAIL missing binary $BIN" >&2
  exit 1
fi

WORK="$(mktemp -d)"
HOST_UP=0
XVFB_PID=""
cleanup() {
  if [[ "$HOST_UP" -eq 1 ]]; then
    "$BIN" host stop >/dev/null 2>&1 || true
  fi
  if [[ -n "$XVFB_PID" ]]; then
    kill "$XVFB_PID" >/dev/null 2>&1 || true
  fi
  rm -rf "$WORK"
}
trap cleanup EXIT

export HOME="$WORK/home"
mkdir -p "$HOME"
unset PLEXI_PANE_ID PLEXI_CONTEXT_ID PLEXI_CONTEXT_ROOT PLEXI_RUNNING PLEXI_CHANNEL PLEXI_SOCKET || true
export PLEXI_KEYCHAIN_PATH="$WORK/keychain"
mkdir -p "$PLEXI_KEYCHAIN_PATH"
# Always a private X server. An ambient DISPLAY can answer xdpyinfo and still
# never become ready for the wgpu host (software Vulkan on Xvfb does).
display_n=99
while xdpyinfo -display ":$display_n" >/dev/null 2>&1; do
  display_n=$((display_n + 1))
done
export DISPLAY=":$display_n"
Xvfb "$DISPLAY" -screen 0 1280x800x24 >/dev/null 2>&1 &
XVFB_PID=$!
for _ in 1 2 3 4 5 6 7 8 9 10; do
  if xdpyinfo -display "$DISPLAY" >/dev/null 2>&1; then
    break
  fi
  sleep 0.1
done
export WGPU_BACKEND="${WGPU_BACKEND:-vulkan}"
if [[ -z "${VK_DRIVER_FILES:-}" && -f /usr/share/vulkan/icd.d/lvp_icd.json ]]; then
  export VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json
fi
if [[ -z "${XDG_RUNTIME_DIR:-}" ]]; then
  export XDG_RUNTIME_DIR="$WORK/runtime"
fi
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"

WS="$WORK/workspace"
mkdir -p "$WS" "$WORK/outside"
FILE="$WORK/outside/temp.md"
printf 'alpha\n' > "$FILE"
SHOT="${SHOT:-$WORK/pending.png}"

PASS=0
FAIL=0
BYPASS=0
ok() { echo "PASS $1"; PASS=$((PASS + 1)); }
bad() { echo "FAIL $1"; FAIL=$((FAIL + 1)); }

bytes() { sha256sum "$FILE" | awk '{print $1}'; }

HUMAN=""
if [[ -f "$ROOT/scripts/e2e/human.sh" ]]; then
  # shellcheck disable=SC1091
  source "$ROOT/scripts/e2e/human.sh"
  HUMAN=1
fi

approve_pending() {
  local id="$1"
  if [[ -n "$HUMAN" ]]; then
    HUMAN_APPROVE "$id" once
    return
  fi
  BYPASS=1
  echo "VERIFIED-VIA-BYPASS permission $id" >&2
  "$BIN" assistant permission resolve "$id" --choice once >/dev/null
}

run_tool() {
  local name="$1" input="$2" out code id
  set +e
  out="$("$BIN" assistant tool "$name" --input "$input" 2>"$WORK/tool.err")"
  code=$?
  set -e
  if [[ "$code" -eq 2 ]]; then
    id="$(python3 -c 'import json,sys; print(json.load(sys.stdin).get("pending_request_id") or "")' <<<"$out")"
    if [[ -z "$id" ]]; then
      echo "tool $name permission response had no pending id" >&2
      echo "$out" >&2
      return 1
    fi
    approve_pending "$id"
    out="$("$BIN" assistant tool "$name" --input "$input")"
  elif [[ "$code" -ne 0 ]]; then
    echo "tool $name failed exit=$code" >&2
    echo "$out" >&2
    cat "$WORK/tool.err" >&2 || true
    return 1
  fi
  printf '%s\n' "$out"
}

echo "starting host $BIN"
"$BIN" host start --ephemeral --background --timeout-secs 90
HOST_UP=1
PROFILE="$("$BIN" changes profile)"
export PLEXI_SOCKET="$PROFILE/notify.sock"

"$BIN" context set-root "$WS" >/dev/null
PANE="$("$BIN" app open text-editor "$FILE")"
echo "editor pane $PANE file $FILE"

EDITORS="$(run_tool host.editors.list '{}')"
if python3 -c 'import json,sys; body=json.load(sys.stdin); paths=[e.get("path") for e in body["output"]["editors"]]; sys.exit(0 if sys.argv[1] in paths else 1)' "$FILE" <<<"$EDITORS"; then
  ok "assistant sees the open editor path"
else
  bad "assistant sees the open editor path"
  echo "$EDITORS"
fi

EDIT_INPUT="$(python3 -c 'import json,sys; print(json.dumps({"path":sys.argv[1],"old_string":"alpha\n","new_string":"beta\n"}))' "$FILE")"
BEFORE="$(bytes)"
EDITED="$(run_tool host.files.edit "$EDIT_INPUT")"
CS="$(python3 -c 'import json,sys; body=json.load(sys.stdin); out=body["output"]; assert out.get("applied") is False; print(out["change_set_id"])' <<<"$EDITED")"
if [[ "$(bytes)" == "$BEFORE" ]]; then
  ok "propose leaves the file unchanged"
else
  bad "propose leaves the file unchanged"
fi

PENDING=0
STATE=""
for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16; do
  STATE="$("$BIN" pane state "$PANE")"
  if python3 -c 'import json,sys; body=json.load(sys.stdin); cs=(body.get("app_state") or {}).get("change_set") or {}; diff=cs.get("diff") or ""; sys.exit(0 if cs.get("status")=="pending" and "-alpha" in diff and "+beta" in diff and (body.get("app_state") or {}).get("source_text")=="alpha\n" else 1)' <<<"$STATE"; then
    PENDING=1
    break
  fi
  sleep 0.25
done
if [[ "$PENDING" -eq 1 ]]; then
  ok "editor pane shows the pending diff"
else
  bad "editor pane shows the pending diff"
  echo "$STATE"
fi

ACCEPT_MODE="cli"
if [[ -n "$HUMAN" ]]; then
  # The editor Accept button is a pointer click, not a permission-sheet
  # label. HUMAN_APPROVE covers a pending gate id when one is waiting.
  # A set that is already granted still needs the editor click; without a
  # pending id the CLI accept below is the bypass.
  ACCEPT_MODE="human-or-cli"
fi
set +e
ACCEPT_OUT="$("$BIN" changes accept "$CS" 2>"$WORK/accept.err")"
ACCEPT_CODE=$?
set -e
if [[ "$ACCEPT_CODE" -eq 2 ]]; then
  ACCEPT_ID="$(printf '%s\n' "$ACCEPT_OUT" | sed -n 's/^pending_request_id=//p' | head -n 1)"
  approve_pending "$ACCEPT_ID"
  set +e
  ACCEPT_OUT="$("$BIN" changes accept "$CS" 2>"$WORK/accept.err")"
  ACCEPT_CODE=$?
  set -e
fi
if [[ "$ACCEPT_MODE" != "human" ]]; then
  BYPASS=1
  echo "VERIFIED-VIA-BYPASS accept $CS" >&2
fi
if [[ "$ACCEPT_CODE" -eq 0 ]] && python3 -c 'import pathlib,sys; sys.exit(0 if pathlib.Path(sys.argv[1]).read_bytes()==b"beta\n" else 1)' "$FILE"; then
  ok "accept writes the agent edit"
else
  bad "accept writes the agent edit"
  echo "exit=$ACCEPT_CODE"
  echo "$ACCEPT_OUT"
  cat "$WORK/accept.err" >&2 || true
  python3 -c 'import pathlib,sys; print(repr(pathlib.Path(sys.argv[1]).read_bytes()))' "$FILE" || true
fi

UPDATED=0
for _ in 1 2 3 4 5 6 7 8 9 10 11 12; do
  STATE="$("$BIN" pane state "$PANE")"
  if python3 -c 'import json,sys; body=json.load(sys.stdin); text=(body.get("app_state") or {}).get("source_text") or ""; sys.exit(0 if "beta" in text else 1)' <<<"$STATE"; then
    UPDATED=1
    break
  fi
  sleep 0.25
done
if [[ "$UPDATED" -eq 1 ]]; then
  ok "accept updates the open buffer"
else
  bad "accept updates the open buffer"
  echo "$STATE"
fi

"$BIN" changes revert "$CS" >/dev/null
RESTORED=0
for _ in 1 2 3 4 5 6 7 8; do
  if python3 -c 'import pathlib,sys; sys.exit(0 if pathlib.Path(sys.argv[1]).read_bytes()==b"alpha\n" else 1)' "$FILE"; then
    RESTORED=1
    break
  fi
  sleep 0.2
done
if [[ "$RESTORED" -eq 1 ]]; then
  ok "revert restores the agent hunk"
else
  bad "revert restores the agent hunk"
  python3 -c 'import pathlib,sys; print(repr(pathlib.Path(sys.argv[1]).read_bytes()))' "$FILE" || true
fi

printf 'alpha\n' > "$FILE"
EDITED="$(run_tool host.files.edit "$EDIT_INPUT")"
CS2="$(python3 -c 'import json,sys; body=json.load(sys.stdin); print(body["output"]["change_set_id"])' <<<"$EDITED")"
printf 'alpha\nextra\n' > "$FILE"
MUTATED="$(bytes)"
sleep 0.3
set +e
"$BIN" changes accept "$CS2" >/dev/null 2>"$WORK/stale.err"
STALE_CODE=$?
set -e
STALE_PREVIEW="$("$BIN" changes preview "$CS2")"
STALE_STATUS="${STALE_PREVIEW%%$'\n'*}"
if [[ "$STALE_CODE" -eq 3 && "$(bytes)" == "$MUTATED" && "$STALE_STATUS" == "status=stale" ]]; then
  ok "conflicting edit is stale and accept is refused"
else
  bad "conflicting edit is stale code=$STALE_CODE"
  echo "$STALE_PREVIEW"
  cat "$WORK/stale.err" >&2 || true
fi

printf 'alpha\n' > "$FILE"
EDITED="$(run_tool host.files.edit "$EDIT_INPUT")"
CS3="$(python3 -c 'import json,sys; body=json.load(sys.stdin); print(body["output"]["change_set_id"])' <<<"$EDITED")"
KEY_DISK="$(bytes)"
for _ in 1 2 3 4 5 6 7 8 9 10 11 12; do
  STATE="$("$BIN" pane state "$PANE")"
  if python3 -c 'import json,sys; body=json.load(sys.stdin); cs=(body.get("app_state") or {}).get("change_set") or {}; sys.exit(0 if cs.get("id")==sys.argv[1] and cs.get("status") in ("pending","stale") else 1)' "$CS3" <<<"$STATE"; then
    break
  fi
  sleep 0.25
done
START_NS="$(date +%s%N)"
KEY_FAIL=0
for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
  if ! "$BIN" pane key "$PANE" a >/dev/null; then
    KEY_FAIL=1
  fi
done
END_NS="$(date +%s%N)"
ELAPSED_MS=$(( (END_NS - START_NS) / 1000000 ))
STATE="$("$BIN" pane state "$PANE")"
KEY_STATUS="$(python3 -c 'import json,sys; body=json.load(sys.stdin); cs=(body.get("app_state") or {}).get("change_set") or {}; print(cs.get("status") or "")' <<<"$STATE")"
if [[ "$KEY_FAIL" -eq 0 && "$ELAPSED_MS" -lt 1000 && "$KEY_STATUS" != "committed" && "$(bytes)" == "$KEY_DISK" ]]; then
  ok "twenty pane key calls returned in ${ELAPSED_MS}ms and did not accept"
else
  bad "twenty pane key calls elapsed=${ELAPSED_MS}ms status=${KEY_STATUS} key_fail=${KEY_FAIL}"
fi

if "$BIN" host screenshot --pane "$PANE" --output "$SHOT" >/dev/null 2>&1; then
  ok "screenshot of the editor"
else
  echo "note: screenshot skipped"
fi

if [[ "$BYPASS" -eq 1 ]]; then
  echo "VERIFIED-VIA-BYPASS"
else
  echo "HUMAN_APPROVE"
fi
echo "PASS=$PASS FAIL=$FAIL shot=$SHOT"
if [[ "$FAIL" -ne 0 ]]; then
  exit 1
fi
