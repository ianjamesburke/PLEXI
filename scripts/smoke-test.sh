#!/usr/bin/env bash
# Post-install smoke test for Plexi v3.
#
# Exercises two layers:
#
#   1. App handshake — for every app in ~/.plexi-v3/apps/, feed a minimal PGAP
#      Init on stdin and assert `"type":"ready"` appears on stdout within 2s.
#      Catches: missing shebang, broken imports, SDK crashes, manifest mismatch.
#
#   2. Host sanity — launch the v3 binary briefly with PLEXI_AUDIO/PLEXI_VIDEO
#      mocks and scan the new log entries for `panic` / `todo!` / `unimplemented`.
#      Catches: Rust host crashes (e.g. `todo!()` in a prod factory).
#
# Usage: scripts/smoke-test.sh
# Exit 0 on clean run; non-zero and prints the failure reason otherwise.

set -uo pipefail

APPS_DIR="${HOME}/.plexi-v3/apps"
LOG_FILE="${HOME}/.plexi-v3/plexi.log"
BINARY="/usr/local/bin/plexi-v3"
FAIL=0

pass() { printf '\033[32m✓\033[0m %s\n' "$*"; }
fail() { printf '\033[31m✗\033[0m %s\n' "$*"; FAIL=1; }

# Portable timeout. macOS has no GNU `timeout`. Runs $cmd with stdin from
# $stdin_file, captures stdout+stderr, kills after $secs seconds.
#   usage: run_app stdin_file secs cmd args...
run_app() {
  local stdin_file=$1 secs=$2
  shift 2
  local out
  out=$(mktemp)
  "$@" <"$stdin_file" >"$out" 2>&1 &
  local pid=$!
  ( sleep "$secs"; kill "$pid" 2>/dev/null ) &
  local killer=$!
  wait "$pid" 2>/dev/null
  kill "$killer" 2>/dev/null
  wait "$killer" 2>/dev/null
  cat "$out"
  rm -f "$out"
}

if ! command -v python3 >/dev/null 2>&1; then
  echo "smoke-test: python3 not on PATH, skipping app handshake checks"
else
  if [[ ! -d "$APPS_DIR" ]]; then
    fail "apps dir missing: $APPS_DIR"
  else
    for app_dir in "$APPS_DIR"/*/; do
      [[ -d "$app_dir" ]] || continue
      manifest="$app_dir/manifest.toml"
      [[ -f "$manifest" ]] || { fail "$(basename "$app_dir"): no manifest.toml"; continue; }
      entry=$(grep -E '^entry' "$manifest" | sed -E 's/entry *= *"([^"]+)"/\1/' | tr -d ' \t')
      [[ -n "$entry" ]] || { fail "$(basename "$app_dir"): no entry in manifest"; continue; }
      entry_path="$app_dir$entry"
      [[ -x "$entry_path" ]] || { fail "$(basename "$app_dir"): entry not executable: $entry_path"; continue; }
      app_id=$(grep -E '^id' "$manifest" | sed -E 's/id *= *"([^"]+)"/\1/' | tr -d ' \t')
      init_file=$(mktemp)
      printf '{"type":"init","protocol":"pgap/3","app_id":"%s","workspace_root":"/tmp","capabilities":[],"feature_flags":["media_v1","pane_groups_v1"]}\n' "$app_id" > "$init_file"
      out=$(run_app "$init_file" 3 "$entry_path" | head -n 3 || true)
      rm -f "$init_file"
      if grep -q '"type": *"ready"' <<<"$out"; then
        pass "$app_id handshake"
      else
        fail "$app_id handshake — no 'ready' in 3s. First lines: $(printf '%s' "$out" | head -c 200)"
      fi
    done
  fi
fi

if [[ -x "$BINARY" ]]; then
  [[ -f "$LOG_FILE" ]] && log_start=$(wc -l < "$LOG_FILE") || log_start=0
  PLEXI_AUDIO="mock://" PLEXI_VIDEO="mock://" "$BINARY" >/dev/null 2>&1 &
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
