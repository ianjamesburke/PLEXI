#!/usr/bin/env bash
# bs-integrate.sh — the head's one-step aggregate batch merge (bs- pipeline).
#
# Usage: bs-integrate.sh [--dry-run] <pr-number> <record.json> <run-dir>/aggregate.json>
#
# Resolves the aggregate worktree from `git worktree list` and runs the whole
# step from there, so the caller's cwd is irrelevant: `integrate`'s post-merge
# reap then never targets the directory it was invoked from. Then retires the
# batch's PR build. No policy logic, no rebase, no merge-flag changes.
set -euo pipefail

if [[ "${1:-}" == "--dry-run" ]]; then
  dry_run=1
  shift
else
  dry_run=0
fi
if [[ $# -ne 3 ]]; then
  echo "bs-integrate: usage: bs-integrate.sh [--dry-run] <pr> <record.json> <aggregate.json>" >&2
  exit 1
fi
pr="$1"; record="$2"; manifest="$3"

# Absolute before any cd, so a relative record/manifest path still resolves.
abspath() { case "$1" in /*) printf '%s' "$1" ;; *) printf '%s/%s' "$PWD" "$1" ;; esac; }
record="$(abspath "$record")"
manifest="$(abspath "$manifest")"

aggregate_root="$(git worktree list --porcelain | awk '
  /^worktree / { path=substr($0, 10) }
  /^branch refs\/heads\/feature\/alpha-cleanup$/ { print path; exit }
')"
if [[ -z "$aggregate_root" ]]; then
  echo "bs-integrate: no worktree for feature/alpha-cleanup — cannot integrate PR #$pr." >&2
  exit 1
fi

if [[ "$dry_run" == 1 ]]; then
  echo "bs-integrate: dry run"
  echo "  aggregate worktree: $aggregate_root"
  echo "  uv run --no-project python scripts/babysitter-policy.py integrate $pr $record $manifest"
  echo "  just channel-clean pr-$pr"
  exit 0
fi

cd "$aggregate_root"
uv run --no-project python scripts/babysitter-policy.py integrate "$pr" "$record" "$manifest"
just channel-clean "pr-$pr"
echo "bs-integrate: PR #$pr integrated into feature/alpha-cleanup; plexi-pr-$pr uninstalled."
