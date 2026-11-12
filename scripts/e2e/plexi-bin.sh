#!/usr/bin/env bash
# Channel binary for installed e2e scripts.
#
# The acceptance harness exports PLEXI_BIN (absolute path) and PLEXI_E2E_SHIM
# (this file). Source it once, in place of `BIN="plexi-pr-${PR}"`:
#
#   source "${PLEXI_E2E_SHIM:?}"
#
# A channel-named binary (`plexi-alpha`, `plexi-pr-N`) ignores PLEXI_CHANNEL.
# The profile follows that basename: plexi-alpha → ~/.plexi-alpha.
# A bare `plexi` still follows PLEXI_CHANNEL (default alpha → ~/.plexi-alpha).

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
if [[ "$BIN_NAME" == plexi-* ]]; then
  unset PLEXI_CHANNEL || true
  PROFILE="${HOME}/.${BIN_NAME}"
elif [[ "$BIN_NAME" == "plexi" ]]; then
  if [[ -z "${PLEXI_CHANNEL:-}" ]]; then
    export PLEXI_CHANNEL="alpha"
  fi
  PROFILE="${HOME}/.plexi-${PLEXI_CHANNEL}"
else
  echo "FAIL: PLEXI_BIN basename must be plexi or plexi-<channel>, got $BIN_NAME" >&2
  exit 1
fi
CHANNEL_DIR=".${BIN_NAME}"
export PLEXI_BIN="$BIN"
export BIN BIN_PATH BIN_NAME PROFILE CHANNEL_DIR
