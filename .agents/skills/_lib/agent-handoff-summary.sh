#!/usr/bin/env bash
set -euo pipefail

PLEXI_BIN="plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL}"

read_slot() {
  local path="${1:-}"
  if [ -n "$path" ] && [ -f "$path" ]; then
    tr '\n' ' ' < "$path" | sed 's/[[:space:]]\+$//'
  fi
}

PANES_JSON="$($PLEXI_BIN pane list)"

printf 'Agent handoff summary\n'
printf '=====================\n'

printf '%s\n' "$PANES_JSON" | jq -r '
  .[]
  | select(.slots != null)
  | select((.slots.pipeline_phase // .slots.status // .slots.issue // .slots.pr // "") != "")
  | [
      (.id // .pane_id // ""),
      (.title // .name // ""),
      (.slots.pipeline_phase // ""),
      (.slots.issue // ""),
      (.slots.pr // ""),
      (.slots.status // ""),
      (.slots.test_instructions // ""),
      (.slots.last_error // "")
    ]
  | @tsv
' | while IFS=$'\t' read -r pane_id title phase_path issue_path pr_path status_path test_path error_path; do
  phase="$(read_slot "$phase_path")"
  issue="$(read_slot "$issue_path")"
  pr="$(read_slot "$pr_path")"
  slot_status="$(read_slot "$status_path")"
  test_instructions="$(read_slot "$test_path")"
  last_error="$(read_slot "$error_path")"

  printf '\nPane %s' "$pane_id"
  if [ -n "$title" ]; then
    printf ' — %s' "$title"
  fi
  printf '\n'

  printf '  doing: %s' "${phase:-unknown}"
  if [ -n "$issue" ]; then
    printf ' issue #%s' "$issue"
  fi
  if [ -n "$pr" ]; then
    printf ' PR #%s' "$pr"
  fi
  printf '\n'

  printf '  waiting: %s\n' "${slot_status:-working}"

  if [ -n "$last_error" ]; then
    printf '  needs from Ian: inspect error — %s\n' "$last_error"
  elif printf '%s' "$slot_status" | grep -Eq 'needs-you|waiting|blocked'; then
    printf '  needs from Ian: %s\n' "${test_instructions:-reply or review the pane}"
  else
    printf '  needs from Ian: nothing right now\n'
  fi
done
