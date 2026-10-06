#!/usr/bin/env bash
# Installed-binary check for folder-scoped secrets.
# Contract: src/workspace/AGENTS.md (folder secrets).
#
#   bash scripts/folder-secrets-e2e.sh [path-to-plexi-binary]
#
# Uses a private HOME so the check never touches a real profile. The secret
# value is generated at runtime and is not written into this script.
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PLEXI="${1:-$REPO_ROOT/target/release/plexi}"
WORK="$(mktemp -d -t plexi-folder-secrets-XXXXXX)"
HOME_DIR="$WORK/home"
DIR_A="$WORK/A"
DIR_B="$WORK/B"
DIR_SIB="$WORK/A-extra"
PROFILE="$HOME_DIR/.plexi"
KEY_DIR="$HOME_DIR/.local/share/plexi"
SECRET=""
HOST_STARTED=0
PASS_N=0
FAIL_N=0
KEYCHAIN=""
SEARCH_FILE=""
DEFAULT_KC=""

unset PLEXI_SOCKET PLEXI_CHANNEL PLEXI_CONTEXT_ROOT PLEXI_CONTEXT_ID \
  PLEXI_CONTEXT_NAME PLEXI_RUNNING PLEXI_PANE_ID PLEXI_CALL_CREDENTIAL \
  PLEXI_HOST_MCP_PORT PLEXI_HOST_MCP_TOKEN PLEXI_KEYCHAIN_PATH
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

restore_keychain_search() {
  [[ "$(uname -s)" == "Darwin" ]] || return 0
  [[ -f "${SEARCH_FILE:-}" ]] || return 0
  local -a paths=()
  local line
  while IFS= read -r line; do
    [[ -n "$line" ]] && paths+=("$line")
  done <"$SEARCH_FILE"
  if ((${#paths[@]})); then
    security list-keychains -d user -s "${paths[@]}" >/dev/null
  fi
  if [[ -n "${DEFAULT_KC:-}" ]]; then
    security default-keychain -s "$DEFAULT_KC" >/dev/null 2>&1 || true
  fi
}

# Snapshot the real user's keychain list, create a throwaway keychain, then
# put the list and the default keychain back. `security create-keychain` adds
# the new file to the search list; the restore runs before HOME changes so
# the login keychain is what it was.
isolate_macos_keychain() {
  [[ "$(uname -s)" == "Darwin" ]] || return 0
  KEYCHAIN="$WORK/test.keychain-db"
  SEARCH_FILE="$WORK/keychain-search.txt"
  : >"$SEARCH_FILE"
  local line pw
  while IFS= read -r line; do
    line="${line#"${line%%[![:space:]]*}"}"
    line="${line%\"}"
    line="${line#\"}"
    [[ -n "$line" ]] && printf '%s\n' "$line" >>"$SEARCH_FILE"
  done < <(security list-keychains -d user)
  DEFAULT_KC="$(security default-keychain 2>/dev/null | tr -d '"' | awk '{$1=$1; print}' || true)"
  pw="$(openssl rand -hex 24)"
  if ! security create-keychain -p "$pw" "$KEYCHAIN"; then
    echo "FAIL: could not create an isolated keychain" >&2
    exit 1
  fi
  restore_keychain_search
  if ! security unlock-keychain -p "$pw" "$KEYCHAIN"; then
    echo "FAIL: could not unlock the isolated keychain" >&2
    exit 1
  fi
  security set-keychain-settings -t 3600 "$KEYCHAIN" >/dev/null
  unset pw
  export PLEXI_KEYCHAIN_PATH="$KEYCHAIN"
  if security list-keychains -d user | grep -F "$KEYCHAIN" >/dev/null; then
    echo "FAIL: isolated keychain is still on the user search list" >&2
    exit 1
  fi
}

cleanup() {
  stty echo 2>/dev/null || true
  if [[ "$HOST_STARTED" == 1 ]]; then
    "$PLEXI" host stop >/dev/null 2>&1 || true
  fi
  if [[ -n "$KEYCHAIN" ]]; then
    restore_keychain_search
    security delete-keychain "$KEYCHAIN" >/dev/null 2>&1 || rm -f "$KEYCHAIN"
    KEYCHAIN=""
  fi
  if [[ -n "$SECRET" ]]; then
    # Drop the value before removing the work tree so a crash dump of the
    # script's environment is the only remaining copy, and it is not on disk.
    SECRET=""
  fi
  rm -rf "$WORK"
}
trap cleanup EXIT

mkdir -p "$HOME_DIR" "$DIR_A" "$DIR_B" "$DIR_SIB" "$KEY_DIR"
# Create the keychain while HOME is still the caller's, then restore the
# search list and the default keychain. The binary reads PLEXI_KEYCHAIN_PATH
# and does not open the login keychain.
isolate_macos_keychain
export HOME="$HOME_DIR"
export XDG_DATA_HOME="$HOME_DIR/.local/share"
export XDG_CONFIG_HOME="$HOME_DIR/.config"
export XDG_CACHE_HOME="$HOME_DIR/.cache"
export HISTFILE=/dev/null
set +o history

contains_secret() {
  local text="$1"
  [[ -n "$SECRET" && "$text" == *"$SECRET"* ]]
}

if [[ ! -x "$PLEXI" ]]; then
  echo "FAIL: no executable at $PLEXI — run 'just build' first" >&2
  exit 1
fi

SECRET="$(openssl rand -hex 24)"
SECRET="fs9-${SECRET}"

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

# A window is optional. macOS does not need X. Headless runs, and
# PLEXI_E2E_SKIP_PANES=1, skip the pane check instead of failing it.
pane_gui=0
if [[ "${PLEXI_E2E_SKIP_PANES:-}" == 1 ]]; then
  printf 'SKIP: pane checks (PLEXI_E2E_SKIP_PANES=1)\n'
elif [[ "$(uname -s)" == "Darwin" ]]; then
  if pgrep -q WindowServer; then
    pane_gui=1
  else
    printf 'SKIP: pane checks (no WindowServer)\n'
  fi
elif [[ -n "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ]]; then
  pane_gui=1
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
printf '%s' "$SECRET" >"$audit_pat"
if [[ -f "$audit" ]] \
  && grep -q 'FOLDER_E2E_SECRET' "$audit" \
  && grep -q '"kind":"ask"' "$audit" \
  && ! grep -q -F -f "$audit_pat" "$audit"; then
  pass "audit row names the secret and does not contain the value"
else
  fail "audit row names the secret and does not contain the value"
fi
rm -f "$audit_pat"

# ── grant, then read ─────────────────────────────────────────────────────────
grant_out="$("$PLEXI" secret grant FOLDER_E2E_SECRET --agent reader --folder "$DIR_A" 2>"$WORK/grant.err")" || grant_code=$?
grant_code="${grant_code:-0}"
grant_err="$(cat "$WORK/grant.err" 2>/dev/null || true)"
if [[ "$grant_code" -eq 0 ]] && ! contains_secret "$grant_out" && ! contains_secret "$grant_err"; then
  pass "secret grant records an allow without printing the value"
else
  fail "secret grant records an allow without printing the value"
fi

got="$("$PLEXI" secret read FOLDER_E2E_SECRET --agent reader --folder "$DIR_A" 2>"$WORK/read2.err")" || got_code=$?
got_code="${got_code:-0}"
if [[ "$got_code" -eq 0 && "$got" == "$SECRET" ]]; then
  pass "agent with the grant reads the secret"
else
  fail "agent with the grant reads the secret (exit ${got_code})"
fi
got=""

# ── plaintext search ─────────────────────────────────────────────────────────
pat="$(mktemp)"
chmod 600 "$pat"
printf '%s' "$SECRET" >"$pat"
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

echo
printf 'folder-secrets-e2e: %d PASS, %d FAIL\n' "$PASS_N" "$FAIL_N"
if [[ "$FAIL_N" -eq 0 ]]; then
  exit 0
fi
exit 1
