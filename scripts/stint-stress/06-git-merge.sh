#!/usr/bin/env bash
# Git merges of the per-task files. There is no single ledger file.
set -u
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
source "$ROOT/lib.sh"
resolve_stint

section "binary"
"$STINT" --version

repo="$(new_repo)"
printf 'repo=%s\n' "$repo"
cd "$repo"
git init -b main >/dev/null
git config user.email "stint-stress@example.com"
git config user.name "Stint Stress"

"$STINT" add "shared task" --no-edit --priority p1 --size m >/dev/null
git add .stint
git commit -m "base task" >/dev/null

section "two branches edit the same task file"
git checkout -b lane-a >/dev/null
"$STINT" claim 0001 --started-at 2026-10-05T00:00:00Z >/dev/null
git add .stint
git commit -m "claim on lane-a" >/dev/null

git checkout main >/dev/null
git checkout -b lane-b >/dev/null
body="$(mktemp)"
cat >"$body" <<'EOF'
## Why

lane-b rewrote the body and marked the task done in prose only.
The status change is applied with stint set by rewriting the file below.
EOF
# stint set replaces the body and rewrites frontmatter, so both sides touch
# the same file. Force a status divergence by editing after set.
"$STINT" set 0001 --body-file "$body" >/dev/null
python3 - <<'PY'
from pathlib import Path
path = next(Path(".stint/tasks").glob("0001-*.md"))
text = path.read_text()
text = text.replace("status: todo", "status: done", 1)
path.write_text(text)
print(path.read_text().split("---", 2)[1])
PY
git add .stint
git commit -m "rewrite on lane-b" >/dev/null

git checkout lane-a >/dev/null
set +e
merge_out="$(git merge --no-edit lane-b 2>&1)"
merge_rc=$?
set +e
printf '$ git merge --no-edit lane-b\nexit=%s\n%s\n' "$merge_rc" "$merge_out"
printf '\n-- conflicted file --\n'
file="$(echo .stint/tasks/0001-*.md)"
cat "$file"
printf '\n-- stint check --\n'
run "$STINT" check
printf '\n-- stint list --\n'
run "$STINT" list
printf '\n-- stint show 0001 --\n'
run "$STINT" show 0001

section "divergent adds allocate the same id"
git merge --abort 2>/dev/null || true
git checkout -- . >/dev/null || true
git checkout main >/dev/null

git checkout -b add-left >/dev/null
"$STINT" add "left only" --no-edit --priority p2 --size s >/dev/null
git add .stint
git commit -m "left add" >/dev/null
printf 'left files:\n'
ls .stint/tasks

git checkout main >/dev/null
git checkout -b add-right >/dev/null
"$STINT" add "right only" --no-edit --priority p3 --size s >/dev/null
git add .stint
git commit -m "right add" >/dev/null
printf 'right files:\n'
ls .stint/tasks

git checkout add-left >/dev/null
set +e
merge_out="$(git merge --no-edit add-right 2>&1)"
merge_rc=$?
set +e
printf '\n$ git merge --no-edit add-right\nexit=%s\n%s\n' "$merge_rc" "$merge_out"
printf 'files after merge:\n'
ls .stint/tasks
printf '\n-- stint check --\n'
run "$STINT" check
printf '\n-- stint list --\n'
run "$STINT" list
printf '\n-- stint next --json --\n'
run "$STINT" next --json
printf '\n-- stint show 0002 --\n'
run "$STINT" show 0002

section "sprint file is a single shared index"
git checkout main >/dev/null
"$STINT" sprint new s1 "Oct 5" --goal "index" >/dev/null
"$STINT" sprint add s1 0001 >/dev/null
git add .stint
git commit -m "sprint index" >/dev/null

append_new_task() {
  local title="$1"
  "$STINT" add "$title" --no-edit --priority p1 --size s >/dev/null
  local file id
  file="$(ls -1 .stint/tasks | sort | tail -1)"
  id="${file%%-*}"
  "$STINT" sprint add s1 "$id" >/dev/null
  printf 'added %s as %s\n' "$title" "$id"
}

git checkout -b sprint-left >/dev/null
append_new_task "left sprint task"
git add .stint
git commit -m "sprint left" >/dev/null

git checkout main >/dev/null
git checkout -b sprint-right >/dev/null
append_new_task "right sprint task"
git add .stint
git commit -m "sprint right" >/dev/null

git checkout sprint-left >/dev/null
set +e
merge_out="$(git merge --no-edit sprint-right 2>&1)"
merge_rc=$?
set +e
printf '$ git merge --no-edit sprint-right\nexit=%s\n%s\n' "$merge_rc" "$merge_out"
printf '\n-- sprint file --\n'
if [[ -f .stint/sprints/s1.md ]]; then
  cat .stint/sprints/s1.md
fi
printf '\n-- stint sprint show s1 --\n'
run "$STINT" sprint show s1
printf '\n-- stint check --\n'
run "$STINT" check

cd /
rm -rf "$repo"
