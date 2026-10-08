#!/usr/bin/env bash
# Installed-binary check for text-editor change sets.
#
# An Assistant edit of the file open in the editor shows a diff and does not
# touch disk until accept. Revert restores the agent hunk. A conflicting edit
# is stale. Twenty pane-key calls while a proposal is pending return in under
# one second and do not accept.
#
# scripts/e2e/human.sh is required. Permission asks are HUMAN_APPROVE clicks.
# The editor Accept button is a pointer click through the same driver. The
# script does not resolve a permission or accept a change set from the CLI.
#
# Channel binary: PLEXI_BIN or the first argument, any `plexi-*` name
# (plexi-alpha, plexi-pr-N). The profile follows that binary. A PR number
# alone is not an address.
#
# Linux host startup reads the seal key from Secret Service before the notify
# socket exists. An inherited session bus that accepts and never completes the
# handshake makes `host start` sit until its 90s deadline. A private session
# bus is the same arrangement the needs-you and folder-secret checks use.
set -euo pipefail

if [[ "$(uname -s)" == "Linux" && -z "${CHANGE_SETS_E2E_INNER:-}" ]] && command -v dbus-run-session >/dev/null 2>&1; then
  # Drop the caller's bus before the daemon starts, including a socket at
  # $XDG_RUNTIME_DIR/bus left by an earlier item.
  session_runtime="$(mktemp -d "${TMPDIR:-/tmp}/plexi-change-sets-bus.XXXXXX")"
  chmod 700 "$session_runtime"
  unset DBUS_SESSION_BUS_ADDRESS || true
  export XDG_RUNTIME_DIR="$session_runtime"
  exec dbus-run-session -- env CHANGE_SETS_E2E_INNER=1 XDG_RUNTIME_DIR="$session_runtime" "$0" "$@"
fi
if [[ -z "${CHANGE_SETS_E2E_INNER:-}" ]]; then
  unset DBUS_SESSION_BUS_ADDRESS || true
fi

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

# current_exe() follows symlinks. A symlink named plexi-alpha would report the
# bare binary and open ~/.plexi. Copy to a real file that keeps the channel name.
BIN_NAME="$(basename "$BIN")"
BIN_NAME="${BIN_NAME%.exe}"
BIN_NAME="${BIN_NAME%.EXE}"
if [[ -L "$BIN" && "$BIN_NAME" == plexi-* ]]; then
  mkdir -p "$WORK/bin"
  cp -f "$(readlink -f "$BIN")" "$WORK/bin/$BIN_NAME"
  chmod +x "$WORK/bin/$BIN_NAME"
  BIN="$WORK/bin/$BIN_NAME"
fi

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
# Never the caller's runtime dir. Its bus socket is shared with earlier items,
# and a socket that accepts without a handshake blocks seal startup.
# Inside dbus-run-session the private runtime already holds that bus. Replacing
# it leaves Secret Service activation on a directory with no daemon.
if [[ -z "${CHANGE_SETS_E2E_INNER:-}" ]]; then
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
ok() { echo "PASS $1"; PASS=$((PASS + 1)); }
bad() { echo "FAIL $1"; FAIL=$((FAIL + 1)); }

bytes() { sha256sum "$FILE" | awk '{print $1}'; }

if [[ ! -f "$ROOT/scripts/e2e/human.sh" ]]; then
  echo "FAIL scripts/e2e/human.sh is required" >&2
  exit 1
fi
# shellcheck disable=SC1091
source "$ROOT/scripts/e2e/human.sh"

approve_pending() {
  local id="$1"
  HUMAN_APPROVE "$id" once
}

# Pointer click on a labeled button. Same XTEST path as HUMAN_APPROVE.
click_label() {
  local label="$1"
  local pid wid center attempt x y
  pid="$(human__host_pid)"
  wid="$(human__window_id "$pid")"
  if [[ -z "$wid" ]]; then
    echo "FAIL no host window for $label" >&2
    return 1
  fi
  for attempt in 1 2 3 4 5 6 7 8; do
    center="$(human__button_center "$label" || true)"
    if [[ -z "$center" ]]; then
      sleep 0.3
      continue
    fi
    x="${center%% *}"
    y="${center##* }"
    echo "human: click '$label' at ${x},${y} window=$wid attempt=$attempt" >&2
    human__click_window "$wid" "$x" "$y" "window" || true
    return 0
  done
  echo "FAIL button $label not found" >&2
  return 1
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

if [[ "$(uname -s)" == "Linux" ]] && command -v gnome-keyring-daemon >/dev/null 2>&1; then
  # Claim org.freedesktop.secrets on this bus before the host reads the seal
  # key. Activation from inside PlexiApp::new can sit until host start's
  # deadline when the daemon never takes the name.
  if ! printf '\n' | gnome-keyring-daemon --unlock --components=secrets --daemonize >"$WORK/keyring.out" 2>"$WORK/keyring.err"; then
    echo "note: gnome-keyring unlock failed; the host will refuse a plaintext seal key" >&2
    cat "$WORK/keyring.err" >&2 || true
  fi
fi

echo "starting host $BIN"
"$BIN" host start --ephemeral --background --timeout-secs 90
HOST_UP=1
PROFILE="$("$BIN" changes profile)"
if [[ "$BIN_NAME" == plexi-* ]]; then
  suffix="${BIN_NAME#plexi-}"
  case "$PROFILE" in
    */.plexi-"$suffix") ;;
    *)
      echo "FAIL profile $PROFILE does not follow channel binary $BIN_NAME" >&2
      exit 1
      ;;
  esac
fi
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

if click_label "Accept"; then
  ACCEPT_CLICKED=1
else
  ACCEPT_CLICKED=0
fi
WROTE=0
for _ in 1 2 3 4 5 6 7 8 9 10 11 12; do
  if python3 -c 'import pathlib,sys; sys.exit(0 if pathlib.Path(sys.argv[1]).read_bytes()==b"beta\n" else 1)' "$FILE"; then
    WROTE=1
    break
  fi
  sleep 0.25
done
if [[ "$ACCEPT_CLICKED" -eq 1 && "$WROTE" -eq 1 ]]; then
  ok "accept writes the agent edit"
else
  bad "accept writes the agent edit"
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

echo "PASS=$PASS FAIL=$FAIL shot=$SHOT"
if [[ "$FAIL" -ne 0 ]]; then
  exit 1
fi
