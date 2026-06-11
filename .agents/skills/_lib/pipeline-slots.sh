#!/usr/bin/env bash

# Shared pane-slot publishing for ship-pipeline skills.

pipeline_slots_plexi() {
  printf 'plexi%s' "${PLEXI_CHANNEL:+-$PLEXI_CHANNEL}"
}

pipeline_slot_write() {
  local name="$1"
  local value="${2:-}"

  if [ -z "${PLEXI_SOCKET:-}" ] || [ -z "${PLEXI_PANE_ID:-}" ]; then
    return 0
  fi

  "$(pipeline_slots_plexi)" pane slot write "$name" "$value" --replace >/dev/null 2>&1 || true
}

pipeline_slots_set() {
  local phase="${1:-}"
  local issue="${2:-}"
  local pr="${3:-}"
  local slot_status="${4:-}"
  local test_instructions="${5:-}"
  local last_error="${6:-}"

  pipeline_slot_write pipeline_phase "$phase"
  pipeline_slot_write issue "$issue"
  pipeline_slot_write pr "$pr"
  pipeline_slot_write status "$slot_status"
  pipeline_slot_write test_instructions "$test_instructions"
  pipeline_slot_write last_error "$last_error"
}
