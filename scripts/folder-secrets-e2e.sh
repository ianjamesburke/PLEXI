#!/usr/bin/env bash
# Installed-binary check for folder-scoped secrets.
# Contract: src/workspace/AGENTS.md (folder secrets).
#
#   bash scripts/folder-secrets-e2e.sh [path-to-plexi-binary]
#
# Uses a private HOME so the check never touches a real profile. The secret
# value is generated at runtime and is not written into this script.
set -uo pipefail

# The permission audit is sealed with the host MAC in Secret Service. A private
# session bus lets that write succeed without using the login keyring. Folder
# secret values stay on the encrypted-file backend selected below.
if [[ "$(uname -s)" == "Linux" && -z "${FOLDER_E2E_INNER:-}" ]] && command -v dbus-run-session >/dev/null 2>&1; then
  exec dbus-run-session -- env FOLDER_E2E_INNER=1 "$0" "$@"
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PLEXI="${PLEXI_BIN:-${1:-$REPO_ROOT/target/release/plexi}}"
WORK="$(mktemp -d -t plexi-folder-secrets-XXXXXX)"
HOME_DIR="$WORK/home"
DIR_A="$WORK/A"
DIR_B="$WORK/B"
DIR_SIB="$WORK/A-extra"
# Same rule as channel_suffix_from_basename in src/config/mod.rs. A PR
# install is `plexi-pr-<N>` and writes `~/.plexi-pr-<N>`, not `~/.plexi`.
profile_dirname() {
  local base suffix
  base="$(basename "$1")"
  base="${base%.exe}"
  base="${base%.EXE}"
  if [[ "$base" == plexi-* ]]; then
    suffix="${base#plexi-}"
    if [[ -n "$suffix" ]]; then
      printf '.plexi-%s' "$suffix"
      return
    fi
  fi
  printf '.plexi'
}
PROFILE="$HOME_DIR/$(profile_dirname "$PLEXI")"
KEY_DIR="$HOME_DIR/.local/share/plexi"
SECRET=""
SECRET_B=""
HOST_STARTED=0
XVFB_PID=""
PASS_N=0
FAIL_N=0
KEYCHAIN=""

unset PLEXI_SOCKET PLEXI_CHANNEL PLEXI_CONTEXT_ROOT PLEXI_CONTEXT_ID \
  PLEXI_CONTEXT_NAME PLEXI_RUNNING PLEXI_PANE_ID PLEXI_CALL_CREDENTIAL \
  PLEXI_HOST_MCP_PORT PLEXI_HOST_MCP_TOKEN PLEXI_KEYCHAIN_PATH \
  PLEXI_KEYCHAIN_PASSWORD PLEXI_FOLDER_SECRETS_BACKEND
# The pane check opens a real window. Keep the caller's X cookie; a private
# HOME would otherwise hide ~/.Xauthority and the host could not connect.
if [[ -z "${XAUTHORITY:-}" ]]; then
  if [[ -n "${HOME:-}" && -f "$HOME/.Xauthority" ]]; then
    export XAUTHORITY="$HOME/.Xauthority"
  else
    login_home=""
    if command -v getent >/dev/null 2>&1; then
      login_home="$(getent passwd "$(id -un)" | cut -d: -f6 || true)"
    elif [[ "$(uname -s)" == "Darwin" ]] && command -v dscl >/dev/null 2>&1; then
      login_home="$(dscl . -read "/Users/$(id -un)" NFSHomeDirectory 2>/dev/null | awk '/NFSHomeDirectory:/ {print $2}' || true)"
    fi
    if [[ -n "$login_home" && -f "$login_home/.Xauthority" ]]; then
      export XAUTHORITY="$login_home/.Xauthority"
    fi
  fi
fi
if [[ -z "${XDG_RUNTIME_DIR:-}" || ! -d "${XDG_RUNTIME_DIR:-}" ]]; then
  export XDG_RUNTIME_DIR="/tmp/runtime-$(id -u)"
  mkdir -p "$XDG_RUNTIME_DIR"
  chmod 700 "$XDG_RUNTIME_DIR"
fi

pass() { PASS_N=$((PASS_N + 1)); printf 'PASS: %s\n' "$1"; }
fail() { FAIL_N=$((FAIL_N + 1)); printf 'FAIL: %s\n' "$1" >&2; }

# Point plexi at a path it creates and opens by path. Reads and writes use
# that file only. This script's `security` calls are read-only: no `-s` and
# no keychain path, which are the forms that rewrite the search list or the
# default keychain.
isolate_macos_keychain() {
  [[ "$(uname -s)" == "Darwin" ]] || return 0
  KEYCHAIN="$WORK/test.keychain-db"
  export PLEXI_KEYCHAIN_PATH="$KEYCHAIN"
  export PLEXI_KEYCHAIN_PASSWORD
  PLEXI_KEYCHAIN_PASSWORD="$(openssl rand -hex 24)"
}

# Linux has no login keychain. Force the labeled file so the check never
# probes Secret Service (a probe can launch a keyring dialog).
isolate_linux_backend() {
  [[ "$(uname -s)" == "Darwin" ]] && return 0
  export PLEXI_FOLDER_SECRETS_BACKEND=encrypted-file-fallback
}

# Print the default keychain and the search list. Exit status is part of
# the snapshot so a later failure cannot compare as equal to an earlier one.
user_keychain_snapshot() {
  if ! command -v security >/dev/null 2>&1; then
    printf 'security-absent\n'
    return 0
  fi
  local def="" list="" def_rc=0 list_rc=0
  def="$(security default-keychain 2>>"$WORK/security-snapshot.err")" || def_rc=$?
  list="$(security list-keychains 2>>"$WORK/security-snapshot.err")" || list_rc=$?
  printf 'default_rc:%s\ndefault:%s\nlist_rc:%s\nlist:%s\n' \
    "$def_rc" "$def" "$list_rc" "$list"
}

cleanup() {
  stty echo 2>/dev/null || true
  if [[ "$HOST_STARTED" == 1 ]]; then
    "$PLEXI" host stop >/dev/null 2>&1 || true
  fi
  if [[ -n "$XVFB_PID" ]]; then
    kill "$XVFB_PID" >/dev/null 2>&1 || true
    XVFB_PID=""
  fi
  if [[ -n "$KEYCHAIN" ]]; then
    rm -f "$KEYCHAIN"
    KEYCHAIN=""
  fi
  unset PLEXI_KEYCHAIN_PASSWORD
  if [[ -n "$SECRET" || -n "$SECRET_B" ]]; then
    # Drop the values before removing the work tree so a crash dump of the
    # script's environment is the only remaining copy, and it is not on disk.
    SECRET=""
    SECRET_B=""
  fi
  rm -rf "$WORK"
}
trap cleanup EXIT

mkdir -p "$HOME_DIR" "$DIR_A" "$DIR_B" "$DIR_SIB" "$KEY_DIR"
# The binary creates and opens this file by path. The trap above deletes
# only that file. The Linux override selects the encrypted file and does
# not contact Secret Service.
isolate_macos_keychain
isolate_linux_backend
export HOME="$HOME_DIR"
export XDG_DATA_HOME="$HOME_DIR/.local/share"
export XDG_CONFIG_HOME="$HOME_DIR/.config"
export XDG_CACHE_HOME="$HOME_DIR/.cache"
export HISTFILE=/dev/null
set +o history

contains_secret() {
  local text="$1"
  { [[ -n "$SECRET" && "$text" == *"$SECRET"* ]] ; } \
    || { [[ -n "$SECRET_B" && "$text" == *"$SECRET_B"* ]] ; }
}

if [[ ! -x "$PLEXI" ]]; then
  echo "FAIL: no executable at $PLEXI — run 'just build' first" >&2
  exit 1
fi

SECRET="$(openssl rand -hex 24)"
SECRET="fs9-${SECRET}"
SECRET_B="$(openssl rand -hex 24)"
SECRET_B="fsb-${SECRET_B}"

KEYCHAIN_BEFORE="$(user_keychain_snapshot)"

# ── set ──────────────────────────────────────────────────────────────────────
set_out="$(printf '%s\n' "$SECRET" | "$PLEXI" secret set FOLDER_E2E_SECRET --folder "$DIR_A" 2>"$WORK/set.err")"
set_code=$?
set_err="$(cat "$WORK/set.err" 2>/dev/null || true)"
if [[ "$set_code" -eq 0 ]] && ! contains_secret "$set_out" && ! contains_secret "$set_err"; then
  pass "secret set stored FOLDER_E2E_SECRET for folder A without printing the value"
else
  fail "secret set (exit $set_code)"
fi

# ── list: names and folders only ─────────────────────────────────────────────
list_out="$("$PLEXI" secret list 2>"$WORK/list.err")"
list_code=$?
list_err="$(cat "$WORK/list.err" 2>/dev/null || true)"
if [[ "$list_code" -eq 0 ]] \
  && [[ "$list_out" == *"FOLDER_E2E_SECRET"* ]] \
  && [[ "$list_out" == *"$DIR_A"* ]] \
  && ! contains_secret "$list_out" \
  && ! contains_secret "$list_err"; then
  pass "secret list shows the name and folder and not the value"
else
  fail "secret list shows the name and folder and not the value"
fi
if [[ "$list_err" == *"encrypted-file fallback"* || "$list_err" == *"secret-service"* || "$list_err" == *"macos-keychain"* || "$list_err" == *"windows-credential-manager"* ]]; then
  pass "secret list names the backend"
else
  fail "secret list names the backend"
fi

set_b_out="$(printf '%s\n' "$SECRET_B" | "$PLEXI" secret set FOLDER_B_SECRET --folder "$DIR_B" 2>"$WORK/set-b.err")"
set_b_code=$?
set_b_err="$(cat "$WORK/set-b.err" 2>/dev/null || true)"
if [[ "$set_b_code" -eq 0 ]] && ! contains_secret "$set_b_out" && ! contains_secret "$set_b_err"; then
  pass "secret set stored FOLDER_B_SECRET for folder B without printing the value"
else
  fail "secret set stored FOLDER_B_SECRET for folder B without printing the value"
fi

help_out="$("$PLEXI" secret --help 2>&1 || true)"
if [[ "$help_out" == *"Same-user native processes are not isolated"* ]]; then
  pass "secret help states same-user processes are not isolated"
else
  fail "secret help states same-user processes are not isolated"
fi

# ── spawn env, same map a new pane receives ──────────────────────────────────
exec_a="$("$PLEXI" secret exec --cwd "$DIR_A" -- sh -c 'if [ -n "$FOLDER_E2E_SECRET" ]; then echo folder-secret-present; else echo folder-secret-absent; fi' 2>"$WORK/exec-a.err")"
exec_b="$("$PLEXI" secret exec --cwd "$DIR_B" -- sh -c 'if [ -n "$FOLDER_E2E_SECRET" ]; then echo folder-secret-present; else echo folder-secret-absent; fi' 2>"$WORK/exec-b.err")"
exec_sib="$("$PLEXI" secret exec --cwd "$DIR_SIB" -- sh -c 'if [ -n "$FOLDER_E2E_SECRET" ]; then echo folder-secret-present; else echo folder-secret-absent; fi')"
if [[ "$exec_a" == "folder-secret-present" ]] && ! contains_secret "$exec_a"; then
  pass "spawn env inside folder A sees the env var"
else
  fail "spawn env inside folder A sees the env var"
fi
if [[ "$exec_b" == "folder-secret-absent" && "$exec_sib" == "folder-secret-absent" ]]; then
  pass "spawn env outside folder A does not see the env var"
else
  fail "spawn env outside folder A does not see the env var"
fi

# ── live panes ───────────────────────────────────────────────────────────────
pane_marker() {
  local dir="$1"
  local pane_id capture i
  pane_id="$("$PLEXI" pane new --cwd "$dir" --no-focus 'if [ -n "$FOLDER_E2E_SECRET" ]; then echo folder-secret-present; else echo folder-secret-absent; fi; sleep 30' 2>"$WORK/pane-new.err" | tr -d '[:space:]')"
  if [[ ! "$pane_id" =~ ^[0-9]+$ ]]; then
    printf 'no-pane\n'
    return
  fi
  capture=""
  for i in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
    capture="$("$PLEXI" pane capture "$pane_id" --plain 2>/dev/null || true)"
    if [[ "$capture" == *"folder-secret-present"* || "$capture" == *"folder-secret-absent"* ]]; then
      break
    fi
    sleep 0.5
  done
  "$PLEXI" pane close "$pane_id" >/dev/null 2>&1 || true
  if contains_secret "$capture"; then
    printf 'leaked\n'
    return
  fi
  if [[ "$capture" == *"folder-secret-present"* ]]; then
    printf 'present\n'
  elif [[ "$capture" == *"folder-secret-absent"* ]]; then
    printf 'absent\n'
  else
    printf 'unseen\n'
  fi
}

# Pane checks run when a display is available. On Linux with no display,
# start Xvfb so the pane refusals still run. A pending read is approved
# with HUMAN_APPROVE, not with `secret grant`.
pane_gui=0
if [[ "$(uname -s)" == "Darwin" ]]; then
  if pgrep -q WindowServer; then
    pane_gui=1
  else
    printf 'SKIP: pane checks (no WindowServer)\n'
  fi
elif [[ -n "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ]]; then
  pane_gui=1
elif command -v Xvfb >/dev/null 2>&1; then
  xvfb_display=":47"
  Xvfb "$xvfb_display" -screen 0 1280x800x24 >"$WORK/xvfb.log" 2>&1 &
  XVFB_PID=$!
  export DISPLAY="$xvfb_display"
  export LIBGL_ALWAYS_SOFTWARE=1
  export WINIT_UNIX_BACKEND=x11
  if [[ -f /usr/share/vulkan/icd.d/lvp_icd.x86_64.json ]]; then
    export VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.x86_64.json
  fi
  sleep 0.5
  if kill -0 "$XVFB_PID" 2>/dev/null; then
    pane_gui=1
  else
    printf 'SKIP: pane checks (Xvfb exited)\n'
    XVFB_PID=""
  fi
else
  printf 'SKIP: pane checks (no display)\n'
fi
if [[ "$pane_gui" == 1 ]]; then
  start_out="$("$PLEXI" host start --ephemeral --timeout-secs 90 2>&1)" || true
  printf '%s\n' "$start_out" >"$WORK/host-start.txt"
  status_out="$("$PLEXI" host status --json 2>&1 || true)"
  if [[ "$status_out" == *'"ready":true'* || "$status_out" == *'"ready": true'* ]]; then
    HOST_STARTED=1
    export PLEXI_SOCKET
    PLEXI_SOCKET="$(sed -n 's/.*"socket"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' <<<"$status_out")"
    mark_a="$(pane_marker "$DIR_A")"
    mark_b="$(pane_marker "$DIR_B")"
    if [[ "$mark_a" == "present" ]]; then
      pass "pane in folder A sees the env var"
    else
      fail "pane in folder A sees the env var ($mark_a)"
    fi
    if [[ "$mark_b" == "absent" ]]; then
      pass "pane in folder B does not see the env var"
    else
      fail "pane in folder B does not see the env var ($mark_b)"
    fi

    # Commands run inside a pane whose cwd is folder A. Markers only.
    # Stdout that might contain a value stays in $WORK and is never echoed.
    agent_script="$WORK/pane-agent-check.sh"
    cat >"$agent_script" <<EOF
set -u
meta="\$1"
PLEXI="$PLEXI"
DIR_B="$DIR_B"
"\$PLEXI" secret exec --cwd "\$DIR_B" -- env >"\$meta.exec" 2>"\$meta.exec.err" || echo "exec_code:\$?" >>"\$meta"
if [[ ! -f "\$meta" ]] || ! grep -q '^exec_code:' "\$meta"; then
  echo "exec_code:0" >>"\$meta"
fi
env -u PLEXI_PANE_ID "\$PLEXI" secret exec --cwd "\$DIR_B" -- env >"\$meta.execu" 2>"\$meta.execu.err" || echo "execu_code:\$?" >>"\$meta"
if ! grep -q '^execu_code:' "\$meta"; then
  echo "execu_code:0" >>"\$meta"
fi
case "\$(basename "\$PLEXI")" in
  plexi-*)
    env -u PLEXI_PANE_ID -u PLEXI_SOCKET "\$PLEXI" secret exec --cwd "\$DIR_B" -- env >"\$meta.execboth" 2>"\$meta.execboth.err" || echo "execboth_code:\$?" >>"\$meta"
    if ! grep -q '^execboth_code:' "\$meta"; then
      echo "execboth_code:0" >>"\$meta"
    fi
    ;;
esac
"\$PLEXI" secret grant FOLDER_B_SECRET --agent me --folder "\$DIR_B" >"\$meta.grant" 2>"\$meta.grant.err" || echo "grant_code:\$?" >>"\$meta"
if ! grep -q '^grant_code:' "\$meta"; then
  echo "grant_code:0" >>"\$meta"
fi
if env | grep -q '^PLEXI_TERMINAL_ENV_VALUE_'; then
  echo "dup:present" >>"\$meta"
else
  echo "dup:absent" >>"\$meta"
fi
nested=\$("\$PLEXI" pane new --cwd "\$DIR_B" --no-focus 'if [ -n "\$FOLDER_B_SECRET" ]; then echo folder-b-present; else echo folder-b-absent; fi; sleep 30' | tr -d '[:space:]')
echo "nested:\$nested" >>"\$meta"
if [[ "\$nested" =~ ^[0-9]+\$ ]]; then
  folderb="unseen"
  for _i in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
    cap=\$("\$PLEXI" pane capture "\$nested" --plain 2>/dev/null || true)
    printf '%s\n' "\$cap" >"\$meta.capture"
    if [[ "\$cap" == *folder-b-present* ]]; then
      folderb="present"
      break
    fi
    if [[ "\$cap" == *folder-b-absent* ]]; then
      folderb="absent"
      break
    fi
    sleep 0.5
  done
  "\$PLEXI" pane close "\$nested" >/dev/null 2>&1 || true
  echo "folderb:\$folderb" >>"\$meta"
else
  echo "folderb:no-pane" >>"\$meta"
fi
echo done >>"\$meta"
EOF
    chmod 700 "$agent_script"
    agent_meta="$WORK/agent"
    agent_pane="$("$PLEXI" pane new --cwd "$DIR_A" --no-focus "bash '$agent_script' '$agent_meta'" 2>"$WORK/agent-pane.err" | tr -d '[:space:]')"
    if [[ "$agent_pane" =~ ^[0-9]+$ ]]; then
      for _i in {1..80}; do
        if [[ -f "$agent_meta" ]] && grep -q '^done$' "$agent_meta"; then
          break
        fi
        sleep 0.5
      done
      "$PLEXI" pane close "$agent_pane" >/dev/null 2>&1 || true
    fi
    agent_body="$(cat "$agent_meta" 2>/dev/null || true)"
    exec_body="$(cat "$agent_meta.exec" 2>/dev/null || true)"
    execu_body="$(cat "$agent_meta.execu" 2>/dev/null || true)"
    execboth_body="$(cat "$agent_meta.execboth" 2>/dev/null || true)"
    grant_body="$(cat "$agent_meta.grant" 2>/dev/null || true)"
    grant_err_body="$(cat "$agent_meta.grant.err" 2>/dev/null || true)"
    capture_body="$(cat "$agent_meta.capture" 2>/dev/null || true)"
    leaked=0
    if contains_secret "$exec_body" || contains_secret "$execu_body" || contains_secret "$execboth_body" || contains_secret "$grant_body" || contains_secret "$grant_err_body" || contains_secret "$capture_body" || contains_secret "$agent_body"; then
      leaked=1
    fi
    rm -f "$agent_meta.exec" "$agent_meta.execu" "$agent_meta.execboth" "$agent_meta.grant" "$agent_meta.grant.err" "$agent_meta.capture" "$agent_meta.exec.err" "$agent_meta.execu.err" "$agent_meta.execboth.err"
    if [[ "$leaked" -eq 0 ]] \
      && [[ "$exec_body" == *"permission_denied"* ]] \
      && grep -q '^exec_code:1$' <<<"$agent_body"; then
      pass "pane in folder A: secret exec --cwd B is permission_denied"
    else
      fail "pane in folder A: secret exec --cwd B is permission_denied"
    fi
    if [[ "$leaked" -eq 0 ]] \
      && [[ "$execu_body" == *"permission_denied"* ]] \
      && grep -q '^execu_code:1$' <<<"$agent_body"; then
      pass "pane in folder A: secret exec survives env -u PLEXI_PANE_ID"
    else
      fail "pane in folder A: secret exec survives env -u PLEXI_PANE_ID"
    fi
    if [[ "$leaked" -eq 0 ]] \
      && [[ "$grant_body" == *"permission_denied"* ]] \
      && grep -q '^grant_code:1$' <<<"$agent_body"; then
      pass "pane in folder A: secret grant --folder B is permission_denied"
    else
      fail "pane in folder A: secret grant --folder B is permission_denied"
    fi
    if [[ "$agent_body" == *"dup:absent"* ]]; then
      pass "pane env has no PLEXI_TERMINAL_ENV_VALUE_ duplicate"
    else
      fail "pane env has no PLEXI_TERMINAL_ENV_VALUE_ duplicate"
    fi
    if [[ "$leaked" -eq 0 && "$agent_body" == *"folderb:absent"* ]]; then
      pass "pane in folder A cannot spawn a pane that receives folder B"
    else
      fail "pane in folder A cannot spawn a pane that receives folder B"
    fi
    if [[ -f "$agent_meta" ]] && grep -q '^execboth_code:' "$agent_meta"; then
      if [[ "$leaked" -eq 0 ]] \
        && [[ "$execboth_body" == *"permission_denied"* ]] \
        && grep -q '^execboth_code:1$' <<<"$agent_body"; then
        pass "pane in folder A: secret exec survives clearing PLEXI_PANE_ID and PLEXI_SOCKET"
      else
        fail "pane in folder A: secret exec survives clearing PLEXI_PANE_ID and PLEXI_SOCKET"
      fi
    fi
    unset PLEXI_SOCKET
  else
    fail "pane in folder A sees the env var (host did not become ready)"
    fail "pane in folder B does not see the env var (host did not become ready)"
  fi
fi

# ── agent read without a grant ───────────────────────────────────────────────
read_out="$("$PLEXI" secret read FOLDER_E2E_SECRET --agent reader --folder "$DIR_A" 2>"$WORK/read.err")" || read_code=$?
read_code="${read_code:-0}"
if [[ "$read_code" -eq 2 ]] \
  && [[ "$read_out" == *"permission_required"* ]] \
  && [[ "$read_out" == *"pending_request_id="* ]] \
  && ! contains_secret "$read_out"; then
  pass "agent without a grant gets permission_required"
else
  fail "agent without a grant gets permission_required (exit ${read_code})"
fi

audit="$PROFILE/permission-audit.jsonl"
audit_pat="$(mktemp)"
chmod 600 "$audit_pat"
printf '%s\n%s\n' "$SECRET" "$SECRET_B" >"$audit_pat"
# A sealed audit line stores the fact as a JSON string, so the quotes are escaped.
if [[ -f "$audit" ]] \
  && grep -q 'FOLDER_E2E_SECRET' "$audit" \
  && { grep -F -q '"kind":"ask"' "$audit" || grep -F -q '\"kind\":\"ask\"' "$audit"; } \
  && ! grep -q -F -f "$audit_pat" "$audit"; then
  pass "audit row names the secret and does not contain the value"
else
  fail "audit row names the secret and does not contain the value"
fi
rm -f "$audit_pat"

# ── CLI grant does not record an allow ──────────────────────────────────────
grant_out="$("$PLEXI" secret grant FOLDER_E2E_SECRET --agent reader --folder "$DIR_A" 2>"$WORK/grant.err")" || grant_code=$?
grant_code="${grant_code:-0}"
grant_err="$(cat "$WORK/grant.err" 2>/dev/null || true)"
if [[ "$grant_code" -ne 0 ]] \
  && [[ "$grant_out" == *"permission_denied"* ]] \
  && ! contains_secret "$grant_out" \
  && ! contains_secret "$grant_err"; then
  pass "secret grant does not record an allow"
else
  fail "secret grant does not record an allow (exit ${grant_code})"
fi

got="$("$PLEXI" secret read FOLDER_E2E_SECRET --agent reader --folder "$DIR_A" 2>"$WORK/read2.err")" || got_code=$?
got_code="${got_code:-0}"
if [[ "$got_code" -ne 0 ]] && ! contains_secret "$got" && [[ "$got" != "$SECRET" ]]; then
  pass "a refused grant does not let the next read return the value"
else
  fail "a refused grant does not let the next read return the value (exit ${got_code})"
fi
got=""

# ── keyboard grant ───────────────────────────────────────────────────────────
# The pending id came from the ungranted read. A click on Allow once is the
# grant. `secret grant` above is the negative and is not this click.
pending_id=""
if [[ "$read_out" == *"pending_request_id="* ]]; then
  pending_id="${read_out#*pending_request_id=}"
  pending_id="${pending_id%%$'\n'*}"
fi
if [[ ! -f "$REPO_ROOT/scripts/e2e/human.sh" ]]; then
  fail "human click grants the pending read (scripts/e2e/human.sh missing)"
elif ! command -v xdotool >/dev/null 2>&1; then
  fail "human click grants the pending read (xdotool missing)"
elif [[ "$HOST_STARTED" != 1 ]]; then
  fail "human click grants the pending read (host is not running)"
elif [[ -z "$pending_id" ]]; then
  fail "human click grants the pending read (no pending)"
else
  assist=""
  for _i in $(seq 1 20); do
    "$PLEXI" pane list >"$WORK/panes.json" 2>/dev/null || true
    assist="$(python3 - "$WORK/panes.json" <<'PY'
import json, sys
try:
    rows = json.load(open(sys.argv[1]))
except Exception:
    print("")
    raise SystemExit
for row in rows:
    title = str(row.get("title") or "")
    manifest = str(row.get("manifest_id") or "")
    if row.get("type") == "app" and (
        title.lower() == "assistant" or manifest.lower() == "assistant"
    ):
        print(row.get("id", ""))
        raise SystemExit
print("")
PY
)"
    [[ -n "$assist" ]] && break
    sleep 0.3
  done
  if [[ -n "$assist" ]]; then
    "$PLEXI" pane focus "$assist" >/dev/null 2>&1 || true
  fi
  export BIN="$PLEXI"
  # shellcheck disable=SC1091
  source "$REPO_ROOT/scripts/e2e/human.sh"
  if HUMAN_APPROVE "$pending_id" once; then
    clicked="$("$PLEXI" secret read FOLDER_E2E_SECRET --agent reader --folder "$DIR_A" 2>"$WORK/read-click.err")" || click_code=$?
    click_code="${click_code:-0}"
    if [[ "$click_code" -eq 0 && "$clicked" == "$SECRET" ]]; then
      pass "human click grants the pending read"
    else
      fail "human click grants the pending read (exit ${click_code})"
    fi
    clicked=""
  else
    fail "human click grants the pending read"
  fi
fi

# ── plaintext search ─────────────────────────────────────────────────────────
pat="$(mktemp)"
chmod 600 "$pat"
printf '%s\n%s\n' "$SECRET" "$SECRET_B" >"$pat"
shopt -s nullglob
roots=("$HOME_DIR"/.plexi "$HOME_DIR"/.plexi-* "$KEY_DIR")
shopt -u nullglob
leaks="$(grep -r -l -F -a -f "$pat" "${roots[@]}" 2>/dev/null || true)"
rm -f "$pat"
if [[ -z "$leaks" ]]; then
  pass "grep of ~/.plexi-* , logs, and the key file finds no value"
else
  # Paths only. Matching lines would reprint the value.
  fail "grep of ~/.plexi-* , logs, and the key file finds no value"
  printf '  leaked in:\n' >&2
  printf '%s\n' "$leaks" | sed 's/^/    /' >&2
fi

if [[ "$HOST_STARTED" == 1 ]]; then
  "$PLEXI" host stop >/dev/null 2>&1 || true
  HOST_STARTED=0
fi

KEYCHAIN_AFTER="$(user_keychain_snapshot)"
if [[ "$KEYCHAIN_BEFORE" == "$KEYCHAIN_AFTER" ]]; then
  if [[ -n "$KEYCHAIN" && "$KEYCHAIN_AFTER" == *"$KEYCHAIN"* ]]; then
    fail "default keychain and search list are unchanged"
    printf '  temp keychain path is in the user keychain snapshot\n' >&2
  else
    pass "default keychain and search list are unchanged"
  fi
else
  fail "default keychain and search list are unchanged"
  printf '  before:\n%s\n  after:\n%s\n' "$KEYCHAIN_BEFORE" "$KEYCHAIN_AFTER" >&2
fi

echo
printf 'folder-secrets-e2e: %d PASS, %d FAIL\n' "$PASS_N" "$FAIL_N"
if [[ "$FAIL_N" -eq 0 ]]; then
  exit 0
fi
exit 1
