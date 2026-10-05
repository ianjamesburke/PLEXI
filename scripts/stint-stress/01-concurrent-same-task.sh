#!/usr/bin/env bash
# N processes claim one task at the same moment.
# Also records auto-claim (distinct tasks) and concurrent stint add.
set -u
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
source "$ROOT/lib.sh"
resolve_stint
export STINT

N="${N:-32}"
ROUNDS="${ROUNDS:-5}"
BODY_BYTES="${BODY_BYTES:-2097152}"

section "binary"
"$STINT" --version
printf 'rev=%s N=%s rounds=%s body_bytes=%s\n' "$STINT_REV" "$N" "$ROUNDS" "$BODY_BYTES"

prepare_contended() {
  local repo="$1"
  (
    cd "$repo"
    "$STINT" add "contended" --no-edit --priority p0 --size s >/dev/null
    python3 - "$BODY_BYTES" <<PY
import os, pathlib, subprocess, sys
n = int(sys.argv[1])
body = pathlib.Path(os.environ["TMPDIR"] if False else "/tmp") / "stint-stress-body.md"
body.write_text("## Why\n\n" + ("x" * n) + "\n")
subprocess.check_call([os.environ["STINT"], "set", "0001", "--body-file", str(body)])
PY
  )
}

barrier_run() {
  local dir="$1"
  local n="$2"
  local go="$3"
  shift 3
  local i
  for i in $(seq 1 "$n"); do
    (
      while [[ ! -f "$go" ]]; do
        :
      done
      "$STINT" "$@" >"$dir/out.$i" 2>"$dir/err.$i"
      printf '%s\n' "$?" >"$dir/rc.$i"
    ) &
    printf '%s\n' "$!" >"$dir/pid.$i"
  done
  sleep 0.25
  touch "$go"
  for i in $(seq 1 "$n"); do
    wait "$(cat "$dir/pid.$i")" || true
  done
}

summarize_rcs() {
  local dir="$1"
  local n="$2"
  local ok=0 bad=0 i rc
  for i in $(seq 1 "$n"); do
    rc="$(cat "$dir/rc.$i")"
    if [[ "$rc" == "0" ]]; then
      ok=$((ok + 1))
    else
      bad=$((bad + 1))
    fi
  done
  printf 'successes=%s failures=%s\n' "$ok" "$bad"
  printf 'stderr kinds:\n'
  if grep -h . "$dir"/err.* >/dev/null 2>&1; then
    sed '/^$/d' "$dir"/err.* | sort | uniq -c
  else
    printf '  (none)\n'
  fi
  printf 'stdout kinds:\n'
  if grep -h . "$dir"/out.* >/dev/null 2>&1; then
    # JSON claim output includes a path; collapse it so the count is readable.
    sed -E '/^$/d; s#/tmp/stint-stress\.[A-Za-z0-9]+#REPO#g; s#\\n##g' "$dir"/out.* | sort | uniq -c
  else
    printf '  (none)\n'
  fi
}

section "explicit claim of one task"
any_multi=0
round=1
while [[ "$round" -le "$ROUNDS" ]]; do
  repo="$(new_repo)"
  prepare_contended "$repo"
  work="$(mktemp -d)"
  go="$work/go"
  (
    cd "$repo"
    barrier_run "$work" "$N" "$go" claim 0001 --json
  )
  printf '\n-- round %s repo %s --\n' "$round" "$repo"
  summarize_rcs "$work" "$N"
  task_file="$(echo "$repo"/.stint/tasks/0001-*.md)"
  printf 'status_line='
  frontmatter_field "$task_file" "status"
  printf 'started_at_line='
  frontmatter_field "$task_file" "started_at"
  ok=0
  for i in $(seq 1 "$N"); do
    if [[ "$(cat "$work/rc.$i")" == "0" ]]; then
      ok=$((ok + 1))
    fi
  done
  if [[ "$ok" -gt 1 ]]; then
    any_multi=1
    printf 'one worker stdout:\n'
    sed -n '1,20p' "$work/out.1"
    printf 'one worker stderr:\n'
    sed -n '1,20p' "$work/err.1"
    printf 'a failing stderr, if any:\n'
    fail_i=""
    for i in $(seq 1 "$N"); do
      if [[ "$(cat "$work/rc.$i")" != "0" ]]; then
        fail_i="$i"
        break
      fi
    done
    if [[ -n "$fail_i" ]]; then
      sed -n '1,20p' "$work/err.$fail_i"
    fi
  fi
  rm -rf "$work" "$repo"
  round=$((round + 1))
done
printf 'RESULT explicit_claim any_round_with_multiple_successes=%s\n' "$any_multi"

section "auto-claim of distinct tasks (control)"
AUTO_N="${AUTO_N:-8}"
repo="$(new_repo)"
(
  cd "$repo"
  i=1
  while [[ "$i" -le "$AUTO_N" ]]; do
    "$STINT" add "lane $i" --no-edit --priority p1 --size s >/dev/null
    i=$((i + 1))
  done
)
work="$(mktemp -d)"
go="$work/go"
(
  cd "$repo"
  barrier_run "$work" "$AUTO_N" "$go" claim --json
)
printf 'repo=%s\n' "$repo"
summarize_rcs "$work" "$AUTO_N"
printf 'claimed ids:\n'
python3 - "$work" "$AUTO_N" <<'PY'
import json, pathlib, sys
work = pathlib.Path(sys.argv[1])
n = int(sys.argv[2])
ids = []
for i in range(1, n + 1):
    text = (work / f"out.{i}").read_text()
    rc = (work / f"rc.{i}").read_text().strip()
    claimed = None
    try:
        claimed = json.loads(text).get("claimed")
    except json.JSONDecodeError:
        claimed = text.strip() or None
    ids.append(claimed)
    print(f"  worker {i} exit={rc} claimed={claimed}")
present = [i for i in ids if i]
print(f"distinct={len(set(present))} nonempty={len(present)}")
PY
rm -rf "$work" "$repo"

section "concurrent stint add"
ADD_N="${ADD_N:-16}"
repo="$(new_repo)"
work="$(mktemp -d)"
go="$work/go"
(
  cd "$repo"
  i=1
  while [[ "$i" -le "$ADD_N" ]]; do
    (
      while [[ ! -f "$go" ]]; do
        :
      done
      "$STINT" add "added $i" --no-edit --priority p2 --size s >"$work/out.$i" 2>"$work/err.$i"
      printf '%s\n' "$?" >"$work/rc.$i"
    ) &
    printf '%s\n' "$!" >"$work/pid.$i"
    i=$((i + 1))
  done
  sleep 0.25
  touch "$go"
  i=1
  while [[ "$i" -le "$ADD_N" ]]; do
    wait "$(cat "$work/pid.$i")" || true
    i=$((i + 1))
  done
)
printf 'repo=%s\n' "$repo"
summarize_rcs "$work" "$ADD_N"
printf 'task files:\n'
ls "$repo/.stint/tasks"
printf 'id fields:\n'
grep -H '^id:' "$repo"/.stint/tasks/*.md
rm -rf "$work" "$repo"
