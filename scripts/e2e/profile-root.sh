#!/usr/bin/env bash
# Disposable profile root for scripts that isolate Plexi (stint 1695).
#
# Plexi chooses its profile from `--profile-root`, `--profile` and the
# executable name, never from HOME or PLEXI_CHANNEL. A private HOME that is
# not the account's home is refused unless `--profile-root` names where
# profiles live. Source after BIN names the channel binary:
#
#   source "$ROOT/scripts/e2e/profile-root.sh"
#   plexi_profile_root "$HOME" [profile-name]
#   "$BIN" pane list                       # rooted
#   run_in_pane "$BIN pane list"           # rooted inside a pane too
#
# plexi_profile_root writes a launcher that runs the real binary with
# `--profile-root <root>` (and `--profile <name>` when one is given: the
# profile a bare `plexi` once took from PLEXI_CHANNEL) and points BIN at it, so every invocation, in this
# shell, in a pane command string or from Python, reaches only the root's
# profile. It then asks the binary which profile it resolved and refuses to
# continue unless that profile is inside the root.
#
# Sets PROFILE_ROOT, PLEXI_REAL_BIN (the channel binary itself), BIN (the
# rooted launcher), PROFILE (the binary's own resolved profile directory,
# from `host status --json`, never computed in shell) and CHANNEL_DIR (its
# directory name).

plexi_profile_root() {
  local root="${1:?plexi_profile_root needs a directory}"
  local profile="${2:-}"
  local real="${BIN:?plexi_profile_root needs BIN}"
  if [[ "$real" == */.profile-root-bin/* ]]; then
    echo "FAIL: plexi_profile_root: BIN ($real) is already a rooted launcher; set BIN to the channel binary" >&2
    exit 1
  fi
  if [[ "$real" != /* ]]; then
    real="$(command -v "$real" 2>/dev/null || true)"
  fi
  if [[ -z "$real" || ! -x "$real" ]]; then
    echo "FAIL: plexi_profile_root: $BIN is not an executable" >&2
    exit 1
  fi
  mkdir -p "$root"
  root="$(cd "$root" && pwd -P)"
  local name launcher_dir launcher
  name="$(basename "$real")"
  launcher_dir="$root/.profile-root-bin"
  launcher="$launcher_dir/$name"
  mkdir -p "$launcher_dir"
  local selectors
  printf -v selectors -- '--profile-root %q' "$root"
  if [[ -n "$profile" ]]; then
    printf -v selectors -- '%s --profile %q' "$selectors" "$profile"
  fi
  printf '#!/bin/sh\nexec %q %s "$@"\n' "$real" "$selectors" >"$launcher"
  chmod 755 "$launcher"
  PROFILE_ROOT="$root"
  PLEXI_REAL_BIN="$real"
  BIN="$launcher"
  # Profile routing comes from the flag, not the environment.
  unset PLEXI_CHANNEL PLEXI_SOCKET || true
  local status socket
  if ! status="$("$BIN" host status --json)"; then
    echo "FAIL: $real could not resolve its profile under $PROFILE_ROOT" >&2
    exit 1
  fi
  socket="$(printf '%s' "$status" | python3 -c 'import json, sys; print(json.load(sys.stdin)["socket"])')"
  PROFILE="$(dirname "$socket")"
  case "$PROFILE" in
    "$PROFILE_ROOT"/.plexi | "$PROFILE_ROOT"/.plexi-*) ;;
    *)
      echo "FAIL: $real resolved profile $PROFILE outside $PROFILE_ROOT" >&2
      exit 1
      ;;
  esac
  CHANNEL_DIR="$(basename "$PROFILE")"
  echo "profile-root: $real resolved $PROFILE"
  export PROFILE_ROOT PROFILE CHANNEL_DIR
}
