#!/usr/bin/env bash
# 10_000 task files. Times the read-path commands and checks id ordering.
set -u
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
source "$ROOT/lib.sh"
resolve_stint

COUNT="${COUNT:-10000}"

section "binary"
"$STINT" --version
printf 'count=%s\n' "$COUNT"

repo="$(new_repo)"
printf 'repo=%s\n' "$repo"
(
  cd "$repo"
  python3 - "$COUNT" <<'PY'
import sys
from pathlib import Path
count = int(sys.argv[1])
tasks = Path(".stint/tasks")
tasks.mkdir(parents=True, exist_ok=True)
priorities = ["p0", "p1", "p2", "p3", "p4"]
for i in range(1, count + 1):
    tid = f"{i:04d}"
    text = (
        f'---\nid: "{tid}"\ntitle: "task {tid}"\nstatus: todo\n'
        f"priority: {priorities[i % 5]}\nsize: s\n"
        f'created_at: "2026-01-01T00:00:00Z"\n---\n\n## Why\n\nseed {tid}\n'
    )
    (tasks / f"{tid}-task.md").write_text(text)
print(f"wrote {count} files")
PY
  printf 'bytes=%s\n' "$(du -sh .stint/tasks | awk '{print $1}')"
  printf 'files=%s\n' "$(find .stint/tasks -name '*.md' | wc -l)"

  time_cmd() {
    local name="$1"
    shift
    local out err
    out="$(mktemp)"
    err="$(mktemp)"
    /usr/bin/time -f "elapsed_sec %e max_rss_kb %M" -o "$err" "$@" >"$out"
    local rc=$?
    printf '\n-- %s --\n' "$name"
    printf 'exit=%s\n' "$rc"
    cat "$err"
    printf 'stdout_lines=%s stdout_bytes=%s\n' "$(wc -l <"$out")" "$(wc -c <"$out")"
    if [[ "$(wc -c <"$out")" -lt 2000 ]]; then
      printf 'stdout:\n'
      cat "$out"
    else
      printf 'stdout head:\n'
      head -n 8 "$out"
      printf 'stdout tail:\n'
      tail -n 4 "$out"
    fi
    rm -f "$out" "$err"
  }

  time_cmd check "$STINT" check
  time_cmd status "$STINT" status
  time_cmd next "$STINT" next
  time_cmd list_p0 "$STINT" list --priority p0
  time_cmd claim_5000 "$STINT" claim 5000 --json
  time_cmd show_5000 "$STINT" show 5000
  time_cmd add_one "$STINT" add "one more" --no-edit --priority p1 --size s

  printf '\n-- id order among equal created_at, same priority --\n'
  # 9995 and 10000 are both priority p0 (i % 5 == 0). String sort is not numeric.
  "$STINT" list --priority p0 > /tmp/stint-stress-p0.txt
  python3 - <<'PY'
from pathlib import Path
lines = Path("/tmp/stint-stress-p0.txt").read_text().splitlines()
ids = []
for line in lines:
    parts = line.split()
    if parts and parts[0].isdigit():
        ids.append(parts[0])
want = {"9995", "10000"}
positions = {i: ids.index(i) for i in ids if i in want}
print("same priority p0, equal created_at; string order is not numeric")
print("positions", positions)
if "10000" in positions and "9995" in positions:
    print("10000_before_9995", positions["10000"] < positions["9995"])
print("first five ids", ids[:5])
print("last five ids", ids[-5:])
PY
)
rm -rf "$repo"
