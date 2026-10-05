#!/usr/bin/env bash
# Run every stint stress script. Writes a transcript under results/.
set -u
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
source "$ROOT/lib.sh"
resolve_stint
export STINT STINT_REV

mkdir -p "$ROOT/results"
transcript="$ROOT/results/run-all.txt"
: >"$transcript"

{
  printf 'started_utc=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  printf 'stint_bin=%s\n' "$STINT"
  "$STINT" --version
  printf 'stint_rev=%s\n' "$STINT_REV"
  printf 'uname=%s\n' "$(uname -srm)"
} | tee -a "$transcript"

run_one() {
  local script="$1"
  {
    printf '\n######## %s ########\n' "$(basename "$script")"
  } | tee -a "$transcript"
  set +e
  bash "$script" 2>&1 | tee -a "$transcript"
  local rc=${PIPESTATUS[0]}
  printf 'SCRIPT_EXIT %s %s\n' "$(basename "$script")" "$rc" | tee -a "$transcript"
}

set +e
for script in \
  "$ROOT/01-concurrent-same-task.sh" \
  "$ROOT/02-retry-same-claimer.sh" \
  "$ROOT/03-crash-and-stale-lock.sh" \
  "$ROOT/04-partial-and-corrupt.sh" \
  "$ROOT/05-large-ledger.sh" \
  "$ROOT/06-git-merge.sh"
do
  run_one "$script"
done
set -e

printf 'finished_utc=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" | tee -a "$transcript"
