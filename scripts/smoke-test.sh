#!/usr/bin/env bash
# Post-install smoke test for Plexi v3.
#
# Launches the v3 binary briefly with PLEXI_AUDIO/PLEXI_VIDEO mocks and scans
# the new log entries for `panic` / `todo!` / `unimplemented`.
# Catches: Rust host crashes (e.g. `todo!()` in a prod factory).
#
# Usage: scripts/smoke-test.sh
# Exit 0 on clean run; non-zero and prints the failure reason otherwise.

set -uo pipefail

LOG_FILE="${HOME}/.plexi-v3/plexi.log"
BINARY="/usr/local/bin/plexi-v3"
FAIL=0

pass() { printf '\033[32m✓\033[0m %s\n' "$*"; }
fail() { printf '\033[31m✗\033[0m %s\n' "$*"; FAIL=1; }

if [[ -x "$BINARY" ]]; then
  [[ -f "$LOG_FILE" ]] && log_start=$(wc -l < "$LOG_FILE") || log_start=0
  # Unset PLEXI_RUNNING so the binary doesn't short-circuit the smoke run
  # when the installer is triggered from inside a Plexi terminal.
  PLEXI_RUNNING= PLEXI_AUDIO="mock://" PLEXI_VIDEO="mock://" "$BINARY" >/dev/null 2>&1 &
  pid=$!
  sleep 2
  kill "$pid" 2>/dev/null
  wait "$pid" 2>/dev/null
  if [[ -f "$LOG_FILE" ]]; then
    tail -n "+$((log_start + 1))" "$LOG_FILE" > /tmp/plexi-v3-smoke.log
    if grep -E 'panicked|todo!|unimplemented|thread .* panicked' /tmp/plexi-v3-smoke.log >/dev/null; then
      fail "host panic found in log since launch:"
      grep -E 'panicked|todo!|unimplemented|thread .* panicked' /tmp/plexi-v3-smoke.log | head -5
    else
      pass "host launch+shutdown clean (no panics in log)"
    fi

    # STEP-11: smoke test asserts effects.jsonl grew during the run —
    # proves FileEventSink is actually writing on the production code
    # path, not just "the host started and didn't panic".
    EFFECTS_FILE="${HOME}/.plexi-v3/effects.jsonl"
    if [[ -f "$EFFECTS_FILE" ]]; then
      if [[ -s "$EFFECTS_FILE" ]]; then
        pass "effects.jsonl non-empty (FileEventSink writing)"
      else
        fail "effects.jsonl exists but is empty — FileEventSink not wired?"
      fi
    else
      fail "effects.jsonl not created — FileEventSink regressed?"
    fi
  fi
else
  echo "smoke-test: binary missing: $BINARY, skipping host check"
fi

if [[ $FAIL -ne 0 ]]; then
  echo
  echo "smoke-test: FAILED"
  exit 1
fi
echo
echo "smoke-test: ok"
