#!/usr/bin/env bash
# Post-install smoke test for Plexi.
#
# Launches the binary briefly with PLEXI_AUDIO/PLEXI_VIDEO mocks and scans
# the new log entries for `panic` / `todo!` / `unimplemented`.
# Catches: Rust host crashes (e.g. `todo!()` in a prod factory).
#
# Usage: scripts/smoke-test.sh [channel]
#   channel: main, alpha, beta, pr-N  (default: auto-detect from git branch)
# Exit 0 on clean run; non-zero on failure or missing binary.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

_git_channel() {
  local branch
  branch="$(git -C "$REPO_ROOT" branch --show-current 2>/dev/null || echo "")"
  case "$branch" in
    main)   echo "main" ;;
    alpha)  echo "alpha" ;;
    beta)   echo "beta" ;;
    *)      cat .channel 2>/dev/null || echo "main" ;;
  esac
}

channel="${1:-$(_git_channel)}"

if [[ "$channel" == "main" ]]; then
  suffix=""
else
  suffix="-$channel"
fi

BINARY="/usr/local/bin/plexi${suffix}"
LOG_FILE="${HOME}/.plexi${suffix}/plexi.log"
EFFECTS_FILE="${HOME}/.plexi${suffix}/effects.jsonl"
FAIL=0

pass() { printf '\033[32m✓\033[0m %s\n' "$*"; }
fail() { printf '\033[31m✗\033[0m %s\n' "$*"; FAIL=1; }

if [[ ! -x "$BINARY" ]]; then
  fail "binary missing: $BINARY"
  exit 1
fi

[[ -f "$LOG_FILE" ]] && log_start=$(wc -l < "$LOG_FILE") || log_start=0

PLEXI_RUNNING= PLEXI_AUDIO="mock://" PLEXI_VIDEO="mock://" "$BINARY" >/dev/null 2>&1 &
pid=$!
sleep 2
kill "$pid" 2>/dev/null
wait "$pid" 2>/dev/null

if [[ -f "$LOG_FILE" ]]; then
  tail -n "+$((log_start + 1))" "$LOG_FILE" > /tmp/plexi-smoke.log
  if grep -E 'panicked|todo!|unimplemented|thread .* panicked' /tmp/plexi-smoke.log >/dev/null; then
    fail "host panic found in log since launch:"
    grep -E 'panicked|todo!|unimplemented|thread .* panicked' /tmp/plexi-smoke.log | head -5
  else
    pass "host launch+shutdown clean (no panics in log)"
  fi

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

if [[ $FAIL -ne 0 ]]; then
  echo
  echo "smoke-test: FAILED"
  exit 1
fi
echo
echo "smoke-test: ok (channel=$channel)"
