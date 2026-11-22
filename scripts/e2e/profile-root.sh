#!/usr/bin/env bash
# Disposable profile root for scripts that isolate Plexi (stint 1695).
#
# Plexi chooses its profile from `--profile-root`, `--profile` and the
# executable name, never from HOME or PLEXI_CHANNEL. A private HOME that is
# not the account's home is refused unless `--profile-root` names where
# profiles live, so a script that isolates itself passes its private
# directory as the root on every invocation. Source after BIN is set:
#
#   source "$ROOT/scripts/e2e/profile-root.sh"
#   plexi_profile_root "$HOME"
#   plx pane list                                  # foreground commands
#   "$BIN" "${PLX_ROOT[@]}" >"$WORK/host.log" 2>&1 & # a host in the background
#   run_in_pane "... $PLX_CMD pane list ..."        # a command string for a pane
#
# Sets PROFILE_ROOT, PLX_ROOT (the flag and its value), PLX_CMD (the quoted
# command prefix for shell strings), PROFILE (the binary's own resolved
# profile directory, read from `host status --json`, never computed in
# shell) and CHANNEL_DIR (that directory's name, the workspace channel
# directory). Every lifecycle verb then reaches only PROFILE's host.

plexi_profile_root() {
  local root="${1:?plexi_profile_root needs a directory}"
  mkdir -p "$root"
  root="$(cd "$root" && pwd -P)"
  PROFILE_ROOT="$root"
  PLX_ROOT=(--profile-root "$PROFILE_ROOT")
  printf -v PLX_CMD '%q --profile-root %q' "$BIN" "$PROFILE_ROOT"
  # Profile routing comes from the flags above, not the environment.
  unset PLEXI_CHANNEL || true
  local status socket
  if ! status="$("$BIN" "${PLX_ROOT[@]}" host status --json)"; then
    echo "FAIL: $BIN could not resolve its profile under $PROFILE_ROOT" >&2
    exit 1
  fi
  socket="$(printf '%s' "$status" | python3 -c 'import json, sys; print(json.load(sys.stdin)["socket"])')"
  PROFILE="$(dirname "$socket")"
  case "$PROFILE" in
    "$PROFILE_ROOT"/.plexi | "$PROFILE_ROOT"/.plexi-*) ;;
    *)
      echo "FAIL: $BIN resolved profile $PROFILE outside $PROFILE_ROOT" >&2
      exit 1
      ;;
  esac
  CHANNEL_DIR="$(basename "$PROFILE")"
  export PROFILE_ROOT PROFILE CHANNEL_DIR PLX_CMD
}

plx() {
  "$BIN" "${PLX_ROOT[@]}" "$@"
}
