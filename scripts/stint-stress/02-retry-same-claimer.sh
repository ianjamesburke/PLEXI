#!/usr/bin/env bash
# Same claimer retries a claim. Also the status transitions claim does not refuse.
set -u
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
source "$ROOT/lib.sh"
resolve_stint

section "binary"
"$STINT" --version

show_claim_fields() {
  local repo="$1"
  local file
  file="$(echo "$repo"/.stint/tasks/0001-*.md)"
  printf 'file=%s\n' "$(basename "$file")"
  frontmatter_field "$file" "status"
  frontmatter_field "$file" "started_at"
  frontmatter_field "$file" "completed_at"
}

section "second claim of the same task"
repo="$(new_repo)"
(
  cd "$repo"
  "$STINT" add "retry me" --no-edit --priority p1 --size s >/dev/null
  printf '\n-- first claim --\n'
  run "$STINT" claim 0001 --json
  show_claim_fields "$repo"
  first_started="$(frontmatter_field "$repo"/.stint/tasks/0001-*.md started_at)"
  sleep 1
  printf '\n-- second claim, same id, no --restart --\n'
  run "$STINT" claim 0001 --json
  show_claim_fields "$repo"
  second_started="$(frontmatter_field "$repo"/.stint/tasks/0001-*.md started_at)"
  if [[ "$first_started" == "$second_started" ]]; then
    printf 'started_at_unchanged=yes\n'
  else
    printf 'started_at_unchanged=no\n'
  fi
  printf '\n-- claim --restart --\n'
  run "$STINT" claim 0001 --restart --json
  show_claim_fields "$repo"
)
rm -rf "$repo"

section "claim of a blocked task by explicit id"
repo="$(new_repo)"
(
  cd "$repo"
  "$STINT" add "blocker" --no-edit --priority p1 --size s >/dev/null
  "$STINT" add "blocked" --no-edit --priority p0 --size s --blocked-by 0001 >/dev/null
  printf '\n-- next before claim --\n'
  run "$STINT" next
  printf '\n-- claim the blocked task --\n'
  run "$STINT" claim 0002 --json
  printf '\n-- check after that claim --\n'
  run "$STINT" check
)
rm -rf "$repo"

section "claim reopens a done task that has no started_at"
repo="$(new_repo)"
(
  cd "$repo"
  "$STINT" add "finish first" --no-edit --priority p1 --size s >/dev/null
  printf '\n-- done --actual without a prior claim --\n'
  run "$STINT" done 0001 --actual 1h
  show_claim_fields "$repo"
  printf '\n-- claim the done task --\n'
  run "$STINT" claim 0001 --json
  show_claim_fields "$repo"
)
rm -rf "$repo"

section "claim reopens an archived task"
repo="$(new_repo)"
(
  cd "$repo"
  "$STINT" add "archived work" --no-edit --priority p1 --size s >/dev/null
  run "$STINT" archive 0001
  show_claim_fields "$repo"
  printf '\n-- claim the archived task --\n'
  run "$STINT" claim 0001 --json
  show_claim_fields "$repo"
)
rm -rf "$repo"

section "claim of a backlog task skips ready"
repo="$(new_repo)"
(
  cd "$repo"
  "$STINT" add "icebox" --no-edit --priority p1 --size s --backlog >/dev/null
  printf '\n-- next (backlog is not scheduled) --\n'
  run "$STINT" next
  printf '\n-- explicit claim --\n'
  run "$STINT" claim 0001 --json
  show_claim_fields "$repo"
)
rm -rf "$repo"

section "unclaim then claim mints a new started_at"
repo="$(new_repo)"
(
  cd "$repo"
  "$STINT" add "abandon" --no-edit --priority p1 --size s >/dev/null
  run "$STINT" claim 0001
  first="$(frontmatter_field "$repo"/.stint/tasks/0001-*.md started_at)"
  sleep 1
  run "$STINT" unclaim 0001
  show_claim_fields "$repo"
  run "$STINT" claim 0001
  second="$(frontmatter_field "$repo"/.stint/tasks/0001-*.md started_at)"
  printf 'first=%s\nsecond=%s\n' "$first" "$second"
)
rm -rf "$repo"
