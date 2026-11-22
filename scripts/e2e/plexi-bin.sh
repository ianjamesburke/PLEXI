#!/usr/bin/env bash
# Channel binary for installed e2e scripts.
#
# The acceptance harness exports PLEXI_BIN (absolute path) and PLEXI_E2E_SHIM
# (this file). Source it once, in place of `BIN="plexi-pr-${PR}"`:
#
#   source "${PLEXI_E2E_SHIM:?}"
#
# The binary selects its profile from `--profile-root`, `--profile` and its
# own name, never from HOME or PLEXI_CHANNEL (stint 1695). Sourcing the shim
# roots every invocation in the script's private HOME: call `plx`, or pass
# "${PLX_ROOT[@]}", and read PROFILE, which the binary itself resolved.

if [[ -n "${PLEXI_BIN_SHIM_LOADED:-}" ]]; then
  return 0
fi
PLEXI_BIN_SHIM_LOADED=1

if [[ -z "${PLEXI_BIN:-}" ]]; then
  echo "FAIL: PLEXI_BIN is unset (path to plexi-alpha, plexi-pr-N, or another channel binary)" >&2
  exit 1
fi

if [[ "$PLEXI_BIN" == /* ]]; then
  BIN="$PLEXI_BIN"
else
  BIN="$(command -v "$PLEXI_BIN" 2>/dev/null || true)"
fi
if [[ -z "$BIN" || ! -x "$BIN" ]]; then
  echo "FAIL: PLEXI_BIN ($PLEXI_BIN) is not an executable channel binary" >&2
  exit 1
fi

BIN_PATH="$BIN"
BIN_NAME="$(basename "$BIN")"
BIN_NAME="${BIN_NAME%.exe}"
BIN_NAME="${BIN_NAME%.EXE}"
if [[ "$BIN_NAME" != plexi && "$BIN_NAME" != plexi-* ]]; then
  echo "FAIL: PLEXI_BIN basename must be plexi or plexi-<channel>, got $BIN_NAME" >&2
  exit 1
fi
# shellcheck source=scripts/e2e/profile-root.sh
source "$(dirname "${BASH_SOURCE[0]}")/profile-root.sh"
plexi_profile_root "$HOME"
export PLEXI_BIN="$BIN"
export BIN BIN_PATH BIN_NAME
