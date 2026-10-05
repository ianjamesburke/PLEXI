#!/usr/bin/env bash
# Partial writes and corrupt task files.
set -u
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
source "$ROOT/lib.sh"
resolve_stint

section "binary"
"$STINT" --version

probe() {
  local label="$1"
  local repo="$2"
  section "$label"
  (
    cd "$repo"
    printf 'files:\n'
    ls -la .stint/tasks
    printf '\n-- check --\n'
    run "$STINT" check
    printf '\n-- list --\n'
    run "$STINT" list
    printf '\n-- next --\n'
    run "$STINT" next
    printf '\n-- show 0001 --\n'
    run "$STINT" show 0001
  )
}

section "reference: healthy file"
repo="$(new_repo)"
(
  cd "$repo"
  "$STINT" add "healthy" --no-edit --priority p1 --size s >/dev/null
)
probe "healthy file" "$repo"
cp "$repo"/.stint/tasks/0001-*.md /tmp/stint-stress-healthy.md
rm -rf "$repo"

make_repo_with() {
  local name="$1"
  local repo
  repo="$(new_repo)"
  cat >"$repo/.stint/tasks/0001-${name}.md"
  printf '%s' "$repo"
}

repo="$(make_repo_with empty </dev/null)"
probe "empty file" "$repo"
rm -rf "$repo"

repo="$(make_repo_with truncated <<'EOF'
---
id: "0001"
title: "torn"
status: to
EOF
)"
probe "frontmatter cut off before the closing delimiter" "$repo"
rm -rf "$repo"

python3 - /tmp/stint-stress-healthy.md <<'PY'
import pathlib, sys
src = pathlib.Path(sys.argv[1]).read_bytes()
pathlib.Path("/tmp/stint-stress-prefix.md").write_bytes(src[:40])
print("prefix_len", 40, "full_len", len(src))
print(src[:40])
PY
repo="$(new_repo)"
cp /tmp/stint-stress-prefix.md "$repo/.stint/tasks/0001-prefix.md"
probe "first 40 bytes of a healthy file" "$repo"
rm -rf "$repo"

repo="$(make_repo_with conflict <<'EOF'
---
id: "0001"
title: "merged"
<<<<<<< HEAD
status: in-progress
started_at: "2026-10-05T00:00:00Z"
=======
status: done
completed_at: "2026-10-05T01:00:00Z"
>>>>>>> lane-b
---

## Why

both sides
EOF
)"
probe "git conflict markers inside the task file" "$repo"
rm -rf "$repo"

repo="$(make_repo_with bad-status <<'EOF'
---
id: "0001"
title: "bad"
status: running
---

## Why

not an enum value
EOF
)"
probe "status: running" "$repo"
rm -rf "$repo"

repo="$(make_repo_with body-gone <<'EOF'
---
id: "0001"
title: "body gone"
status: in-progress
started_at: "2026-10-05T00:00:00Z"
priority: p1
---
EOF
)"
probe "valid frontmatter, body missing" "$repo"
rm -rf "$repo"

section "one corrupt file beside two healthy tasks"
repo="$(new_repo)"
(
  cd "$repo"
  "$STINT" add "alpha" --no-edit --priority p1 --size s >/dev/null
  "$STINT" add "beta" --no-edit --priority p0 --size s >/dev/null
  printf 'not markdown at all' >.stint/tasks/0003-garbage.md
  # Force the id in the garbage name to be a third file the parser must skip.
  printf '\n-- check --\n'
  run "$STINT" check
  printf '\n-- list --\n'
  run "$STINT" list
  printf '\n-- next --\n'
  run "$STINT" next
  printf '\n-- show 0003 --\n'
  run "$STINT" show 0003
)
rm -rf "$repo"
