#!/usr/bin/env bash
# Assistant host-tool path for an open editor file outside the workspace.
# The tool proposes a change set. The editor pane shows the pending diff.
# The file on disk stays unchanged until accept.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${PLEXI_BIN:-$ROOT/target/release/plexi}"
if [[ ! -x "$BIN" ]]; then
  echo "FAIL missing binary $BIN" >&2
  exit 1
fi

WORK="$(mktemp -d)"
HOST_UP=0
cleanup() {
  if [[ "$HOST_UP" -eq 1 ]]; then
    "$BIN" host stop >/dev/null 2>&1 || true
  fi
  rm -rf "$WORK"
}
trap cleanup EXIT

export HOME="$WORK/home"
mkdir -p "$HOME"
unset PLEXI_PANE_ID PLEXI_CONTEXT_ID PLEXI_CONTEXT_ROOT PLEXI_RUNNING PLEXI_CHANNEL PLEXI_SOCKET || true
export DISPLAY="${DISPLAY:-:99}"
export WGPU_BACKEND="${WGPU_BACKEND:-vulkan}"
export VK_DRIVER_FILES="${VK_DRIVER_FILES:-/usr/share/vulkan/icd.d/lvp_icd.json}"
if [[ -z "${XDG_RUNTIME_DIR:-}" ]]; then
  export XDG_RUNTIME_DIR="/tmp/runtime-ubuntu"
fi
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"

WS="$WORK/workspace"
mkdir -p "$WS" "$WORK/outside"
FILE="$WORK/outside/temp.md"
printf 'alpha\n' > "$FILE"
SHOT="${SHOT:-/tmp/assistant-editor-pending.png}"

PASS=0
FAIL=0
ok() { echo "PASS $1"; PASS=$((PASS + 1)); }
bad() { echo "FAIL $1"; FAIL=$((FAIL + 1)); }

echo "starting host $BIN"
"$BIN" host start --ephemeral --background --timeout-secs 90
HOST_UP=1
PROFILE="$("$BIN" changes profile)"
export PLEXI_SOCKET="$PROFILE/notify.sock"

"$BIN" context set-root "$WS" >/dev/null
PANE="$("$BIN" app open text-editor "$FILE")"
echo "editor pane $PANE file $FILE"

run_tool() {
  local name="$1" input="$2" out code id
  set +e
  out="$("$BIN" assistant tool "$name" --input "$input" 2>"$WORK/tool.err")"
  code=$?
  set -e
  if [[ "$code" -eq 2 ]]; then
    id="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["pending_request_id"])' <<<"$out")"
    "$BIN" assistant permission resolve "$id" --choice once >/dev/null
    out="$("$BIN" assistant tool "$name" --input "$input")"
  elif [[ "$code" -ne 0 ]]; then
    echo "tool $name failed exit=$code" >&2
    echo "$out" >&2
    cat "$WORK/tool.err" >&2 || true
    return 1
  fi
  printf '%s\n' "$out"
}

EDITORS="$(run_tool host.editors.list '{}')"
if python3 -c 'import json,sys; body=json.load(sys.stdin); paths=[e.get("path") for e in body["output"]["editors"]]; sys.exit(0 if sys.argv[1] in paths else 1)' "$FILE" <<<"$EDITORS"; then
  ok "host.editors.list returns the open absolute path"
else
  bad "host.editors.list returns the open absolute path"
  echo "$EDITORS"
fi

LIST_INPUT="$(python3 -c 'import json; print(json.dumps({"path":"temp.md"}))')"
LISTED="$(run_tool host.files.list "$LIST_INPUT")"
if python3 -c 'import json,sys; body=json.load(sys.stdin); text=json.dumps(body); sys.exit(0 if sys.argv[1] in text and "path_not_found" not in text else 1)' "$FILE" <<<"$LISTED"; then
  ok "host.files.list bare name resolves to the open editor"
else
  bad "host.files.list bare name resolves to the open editor"
  echo "$LISTED"
fi

OPEN_INPUT="$(python3 -c 'import json,sys; print(json.dumps({"type_id":"text-editor","pane_id":int(sys.argv[1])}))' "$PANE")"
OPENED="$(run_tool host.panes.open "$OPEN_INPUT")"
if python3 -c 'import json,sys; body=json.load(sys.stdin); out=body["output"]; sys.exit(0 if out.get("already_open") is True and out.get("path")==sys.argv[1] else 1)' "$FILE" <<<"$OPENED"; then
  ok "host.panes.open on the editor returns already_open"
else
  bad "host.panes.open on the editor returns already_open"
  echo "$OPENED"
fi

EDIT_INPUT="$(python3 -c 'import json,sys; print(json.dumps({"path":sys.argv[1],"old_string":"alpha\n","new_string":"beta\n"}))' "$FILE")"
EDITED="$(run_tool host.files.edit "$EDIT_INPUT")"
CS="$(python3 -c 'import json,sys; body=json.load(sys.stdin); out=body["output"]; assert out.get("applied") is False; print(out["change_set_id"])' <<<"$EDITED")"
if [[ "$(cat "$FILE")" == $'alpha\n' ]]; then
  ok "propose leaves the file unchanged"
else
  bad "propose leaves the file unchanged"
fi

PENDING=0
STATE=""
for _ in 1 2 3 4 5 6 7 8 9 10 11 12; do
  STATE="$("$BIN" pane state "$PANE")"
  if python3 -c 'import json,sys; body=json.load(sys.stdin); cs=(body.get("app_state") or {}).get("change_set") or {}; diff=cs.get("diff") or ""; sys.exit(0 if cs.get("status")=="pending" and "-alpha" in diff and "+beta" in diff and body.get("app_state",{}).get("source_text")=="alpha\n" else 1)' <<<"$STATE"; then
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

if "$BIN" host screenshot --pane "$PANE" --output "$SHOT" >/dev/null; then
  ok "screenshot of the pending editor"
else
  bad "screenshot of the pending editor"
fi

set +e
ACCEPT_OUT="$("$BIN" changes accept "$CS" 2>"$WORK/accept.err")"
ACCEPT_CODE=$?
set -e
if [[ "$ACCEPT_CODE" -eq 0 && "$(cat "$FILE")" == $'beta\n' ]]; then
  ok "accept writes the one-line change"
else
  bad "accept writes the one-line change"
  echo "exit=$ACCEPT_CODE"
  echo "$ACCEPT_OUT"
  cat "$WORK/accept.err" >&2 || true
fi

UPDATED=0
for _ in 1 2 3 4 5 6 7 8; do
  STATE="$("$BIN" pane state "$PANE")"
  if python3 -c 'import json,sys; body=json.load(sys.stdin); sys.exit(0 if (body.get("app_state") or {}).get("source_text")=="beta\n" else 1)' <<<"$STATE"; then
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

echo "PASS=$PASS FAIL=$FAIL shot=$SHOT"
if [[ "$FAIL" -ne 0 ]]; then
  exit 1
fi
