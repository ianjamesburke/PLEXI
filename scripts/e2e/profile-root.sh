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
# profile. It first asks the binary which profile it resolved, and refuses
# unless that profile is inside the root and the root is not the account home.
#
# Sets PROFILE_ROOT, BIN (the rooted launcher), PROFILE (the binary's own resolved profile directory,
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
  # Logical, not physical: resolving /var to /private/var on macOS can push
  # the endpoint past the Unix socket path limit.
  root="$(cd "$root" && pwd -L)"
  local -a selectors=(--profile-root "$root")
  if [[ -n "$profile" ]]; then
    selectors+=(--profile "$profile")
  fi
  # Profile routing comes from the flags, not the environment.
  unset PLEXI_CHANNEL PLEXI_SOCKET || true
  # Ask the binary before writing anything into the root.
  local status resolved socket account_rooted
  if ! status="$("$real" "${selectors[@]}" host status --json)"; then
    echo "FAIL: $real could not resolve its profile under $root" >&2
    exit 1
  fi
  if ! resolved="$(printf '%s' "$status" | python3 -c 'import json, sys; d = json.load(sys.stdin); print(d["socket"]); print(str(d["account_rooted"]).lower())')"; then
    echo "FAIL: $real did not report its resolved profile and account_rooted" >&2
    exit 1
  fi
  socket="${resolved%$'\n'*}"
  account_rooted="${resolved##*$'\n'}"
  if [[ "$account_rooted" != false ]]; then
    echo "FAIL: $root is the account's home; a script never runs Plexi on the owner's profiles" >&2
    exit 1
  fi
  PROFILE="$(dirname "$socket")"
  case "$PROFILE" in
    "$root"/.plexi | "$root"/.plexi-*) ;;
    *)
      echo "FAIL: $real resolved profile $PROFILE outside $root" >&2
      exit 1
      ;;
  esac
  local name launcher_dir launcher quoted
  name="$(basename "$real")"
  launcher_dir="$root/.profile-root-bin"
  launcher="$launcher_dir/$name"
  mkdir -p "$launcher_dir"
  printf -v quoted ' %q' "${selectors[@]}"
  printf '#!/bin/sh\nexec %q%s "$@"\n' "$real" "$quoted" >"$launcher"
  chmod 755 "$launcher"
  PROFILE_ROOT="$root"
  BIN="$launcher"
  CHANNEL_DIR="$(basename "$PROFILE")"
  echo "profile-root: $real resolved $PROFILE"
  export PROFILE_ROOT PROFILE CHANNEL_DIR
}

# Written into every root plexi_script_root creates: an inherited root is
# accepted only with it, so a stray variable cannot aim a script at real data.
PLEXI_DISPOSABLE_MARKER=".disposable-plexi-root"

# The disposable root a script runs Plexi in (sets SCRIPT_ROOT). The V1
# harness creates its root here and exports it as PLEXI_E2E_PROFILE_ROOT so
# its items share the profile it seeded; an inherited root without the marker
# is refused. Any other run gets a fresh temp dir, and HOME moves there too,
# so nothing the script or its hosts write (profile, context logs, config
# rewrites, cleanup stops) reaches the account's home or its live hosts. Call
# before the first plexi_profile_root.
plexi_script_root() {
  if [[ -n "${PLEXI_E2E_PROFILE_ROOT:-}" ]]; then
    local marker="$PLEXI_E2E_PROFILE_ROOT/$PLEXI_DISPOSABLE_MARKER"
    if [[ -L "$PLEXI_E2E_PROFILE_ROOT" || -L "$marker" || ! -f "$marker" ]]; then
      echo "FAIL: PLEXI_E2E_PROFILE_ROOT=$PLEXI_E2E_PROFILE_ROOT was not created by plexi_script_root" >&2
      exit 1
    fi
    SCRIPT_ROOT="$PLEXI_E2E_PROFILE_ROOT"
    return 0
  fi
  local account_home="$HOME"
  SCRIPT_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/plexi-e2e-root.XXXXXX")"
  : >"$SCRIPT_ROOT/$PLEXI_DISPOSABLE_MARKER"
  mkdir -p "$SCRIPT_ROOT/.plexi"
  # Python apps load the WASI bundle from the profile home.
  if [[ -d "$account_home/.plexi/wasm-bundles" ]]; then
    ln -s "$account_home/.plexi/wasm-bundles" "$SCRIPT_ROOT/.plexi/wasm-bundles"
  fi
  export HOME="$SCRIPT_ROOT"
  echo "profile-root: standalone run in disposable $SCRIPT_ROOT"
}
