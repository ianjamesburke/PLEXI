#!/usr/bin/env bash
# Crash mid-claim, stale claim.lock, and what recovery actually does.
set -u
set -m
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
source "$ROOT/lib.sh"
resolve_stint

section "binary"
"$STINT" --version

section "commands related to recovery"
"$STINT" --help
printf '\n-- claim help --\n'
"$STINT" claim --help

section "pre-created claim.lock, no holder"
repo="$(new_repo)"
(
  cd "$repo"
  "$STINT" add "waiting" --no-edit --priority p1 --size s >/dev/null
  mkdir -p .stint/claim.lock
  printf 'lock_is_dir=%s\n' "$([[ -d .stint/claim.lock ]] && echo yes || echo no)"
  printf '\n-- auto-claim while the directory lock exists --\n'
  /usr/bin/time -f 'elapsed_sec %e' "$STINT" claim --json
  printf 'exit=%s\n' "$?"
  printf 'lock_still_there=%s\n' "$([[ -d .stint/claim.lock ]] && echo yes || echo no)"
  printf '\n-- explicit claim of 0001 while the directory lock exists --\n'
  /usr/bin/time -f 'elapsed_sec %e' "$STINT" claim 0001 --json
  printf 'exit=%s\n' "$?"
)
rm -rf "$repo"

section "kill -9 while auto-claim holds claim.lock"
repo="$(new_repo)"
pushd "$repo" >/dev/null
  python3 - <<'PY'
from pathlib import Path
tasks = Path(".stint/tasks")
tasks.mkdir(parents=True, exist_ok=True)
for i in range(1, 401):
    tid = f"{i:04d}"
    text = f'''---
id: "{tid}"
title: "fat {tid}"
status: todo
priority: p1
size: s
created_at: "2026-01-01T00:00:00Z"
---

## Why

{"y" * 4096}
'''
    (tasks / f"{tid}-fat.md").write_text(text)
print("seeded", 400)
PY
  strace -f -o /tmp/stint-stress-claim-read.strace \
    -e trace=read -e inject=read:delay_enter=2000 \
    "$STINT" claim --json >/tmp/stint-stress-claim-kill.out 2>/tmp/stint-stress-claim-kill.err &
  spid=$!
  saw=no
  deadline=$((SECONDS + 30))
  while [[ "$SECONDS" -lt "$deadline" ]]; do
    if [[ -d .stint/claim.lock ]]; then
      kill -9 -- -"$spid" 2>/dev/null || kill -9 "$spid" 2>/dev/null || true
      saw=yes
      break
    fi
    if ! kill -0 "$spid" 2>/dev/null; then
      break
    fi
  done
  wait "$spid" 2>/dev/null || true
  printf 'saw_lock_before_kill=%s\n' "$saw"
  printf 'lock_remains=%s\n' "$([[ -d .stint/claim.lock ]] && echo yes || echo no)"
  printf 'in_progress_files=%s\n' "$(grep -l 'status: in-progress' .stint/tasks/*.md | wc -l)"
  printf '\n-- auto-claim after the killed holder --\n'
  /usr/bin/time -f 'elapsed_sec %e' "$STINT" claim --json
  printf 'exit=%s\n' "$?"
  printf '\n-- manual rmdir, then auto-claim --\n'
  rmdir .stint/claim.lock
  run "$STINT" claim --json
  printf 'in_progress_files_after=%s\n' "$(grep -l 'status: in-progress' .stint/tasks/*.md | wc -l)"
popd >/dev/null
rm -rf "$repo"

section "explicit claim killed during the truncating write"
repo="$(new_repo)"
pushd "$repo" >/dev/null
  "$STINT" add "torn write" --no-edit --priority p1 --size s >/dev/null
  file="$(echo .stint/tasks/0001-*.md)"
  cp "$file" /tmp/stint-stress-before-claim.md
  printf 'bytes_before=%s\n' "$(wc -c <"$file")"
  strace -f -o /tmp/stint-stress-claim-write.strace \
    -e trace=write -e inject=write:delay_enter=3000000 \
    "$STINT" claim 0001 --json >/tmp/stint-stress-write-kill.out 2>/tmp/stint-stress-write-kill.err &
  spid=$!
  saw_empty=no
  for _ in $(seq 1 80); do
    size="$(wc -c <"$file")"
    if [[ "$size" == "0" ]]; then
      kill -9 -- -"$spid" 2>/dev/null || kill -9 "$spid" 2>/dev/null || true
      saw_empty=yes
      break
    fi
    if ! kill -0 "$spid" 2>/dev/null; then
      break
    fi
    sleep 0.05
  done
  wait "$spid" 2>/dev/null || true
  printf 'saw_empty_file_before_kill=%s\n' "$saw_empty"
  printf 'bytes_after=%s\n' "$(wc -c <"$file")"
  printf 'file_bytes:\n'
  # Show a short prefix even if the file is binary-ish or empty.
  if [[ -s "$file" ]]; then
    head -c 200 "$file"
    printf '\n'
  else
    printf '(empty)\n'
  fi
  printf '\n-- check --\n'
  run "$STINT" check
  printf '\n-- list --\n'
  run "$STINT" list
  printf '\n-- show --\n'
  run "$STINT" show 0001
  printf '\n-- claim again --\n'
  run "$STINT" claim 0001 --json
popd >/dev/null
rm -rf "$repo"

section "stale in-progress is not reaped, and unclaim is unauthenticated"
repo="$(new_repo)"
(
  cd "$repo"
  "$STINT" add "old claim" --no-edit --priority p1 --size s --area host/pane >/dev/null
  "$STINT" add "same area" --no-edit --priority p0 --size s --area host/pane >/dev/null
  "$STINT" claim 0001 --started-at 2020-01-01T00:00:00Z
  printf '\n-- next, with a years-old claim still in progress --\n'
  run "$STINT" next
  printf '\n-- second explicit claim --\n'
  run "$STINT" claim 0001 --json
  printf '\n-- a different process unclaims and claims --\n'
  run "$STINT" unclaim 0001
  run "$STINT" claim 0001 --json
  printf '\n-- auto-claim while 0001 is in progress in the same area --\n'
  run "$STINT" claim --json
  printf '\n-- next --include-area-conflicts --\n'
  run "$STINT" next --include-area-conflicts
)
rm -rf "$repo"
