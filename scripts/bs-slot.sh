#!/usr/bin/env bash
# bs-slot.sh — publish a typed one-line JSON slot report for the bs- pipeline.
#
# Usage: bs-slot.sh <phase> <state> [stint=<id>] [pr=<n>] [verdict=<v>] [detail=<text>]
#   phase: start | reset | validate | merge
#   state: working | done | failed
#
# Writes {phase, state, stint, pr, verdict, detail} to the pane's `report` slot
# and mirrors `<phase>:<state>` to the `status` slot (the babysitter head's
# existing contract). Outside a pane (PLEXI_PANE_ID unset) the report is printed
# to stdout instead — loudly, never silently dropped.
set -euo pipefail

if [[ $# -lt 2 ]]; then
  echo "bs-slot: usage: bs-slot.sh <phase> <state> [stint=..] [pr=..] [verdict=..] [detail=..]" >&2
  exit 1
fi

phase="$1"; state="$2"; shift 2
stint=""; pr=""; verdict=""; detail=""
for kv in "$@"; do
  case "$kv" in
    stint=*)   stint="${kv#stint=}" ;;
    pr=*)      pr="${kv#pr=}" ;;
    verdict=*) verdict="${kv#verdict=}" ;;
    detail=*)  detail="${kv#detail=}" ;;
    *) echo "bs-slot: unknown argument '$kv' (expected stint=/pr=/verdict=/detail=)" >&2; exit 1 ;;
  esac
done

report="$(BS_PHASE="$phase" BS_STATE="$state" BS_STINT="$stint" BS_PR="$pr" BS_VERDICT="$verdict" BS_DETAIL="$detail" python3 - <<'PY'
import json, os
print(json.dumps({
    "phase": os.environ["BS_PHASE"],
    "state": os.environ["BS_STATE"],
    "stint": os.environ["BS_STINT"] or None,
    "pr": int(os.environ["BS_PR"]) if os.environ["BS_PR"] else None,
    "verdict": os.environ["BS_VERDICT"] or None,
    "detail": os.environ["BS_DETAIL"] or None,
}))
PY
)"

if [[ -z "${PLEXI_PANE_ID:-}" ]]; then
  echo "bs-slot: no pane (PLEXI_PANE_ID unset) — report not published: $report"
  exit 0
fi

if ! plexi pane slot write report "$report" --replace \
  || ! plexi pane slot write status "${phase}:${state}" --replace; then
  echo "bs-slot: slot write failed on pane $PLEXI_PANE_ID — report was: $report" >&2
  exit 1
fi
echo "bs-slot: published $report"
