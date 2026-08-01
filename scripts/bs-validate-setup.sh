#!/usr/bin/env bash
# bs-validate-setup.sh — deterministic setup for PR validation (bs- pipeline).
#
# Usage: bs-validate-setup.sh <pr-number>
#
# Resolves the PR's branch and (if present) feature worktree, runs
# `just pr-install <pr>` (cwd-independent — it resolves and builds the PR's
# actual head itself), and publishes a slot report with the install result.
# Prints BRANCH= and WORKTREE= lines for the calling skill to consume.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [[ $# -ne 1 ]]; then
  echo "bs-validate-setup: missing PR number. Usage: just bs-validate-setup <pr>" >&2
  exit 1
fi
pr="$1"

alpha_root="$(git worktree list --porcelain | awk '
  /^worktree / { path=substr($0, 10) }
  /^branch refs\/heads\/alpha$/ { print path; exit }
')"
if [[ -z "$alpha_root" ]]; then
  echo "bs-validate-setup: could not resolve the alpha worktree from 'git worktree list'." >&2
  exit 1
fi

pr_json="$(gh pr view "$pr" --json headRefName,headRefOid,state 2>/dev/null || true)"
if [[ -z "$pr_json" ]]; then
  echo "bs-validate-setup: could not resolve PR #$pr via gh (gh pr view failed)." >&2
  exit 1
fi
branch="$(python3 -c 'import sys,json; print(json.load(sys.stdin)["headRefName"])' <<<"$pr_json")"
head_oid="$(python3 -c 'import sys,json; print(json.load(sys.stdin)["headRefOid"])' <<<"$pr_json")"
state="$(python3 -c 'import sys,json; print(json.load(sys.stdin)["state"])' <<<"$pr_json")"
if [[ "$state" != "OPEN" ]]; then
  echo "bs-validate-setup: PR #$pr is $state, not OPEN — nothing to validate." >&2
  exit 1
fi

worktree="$alpha_root/worktrees/$branch"
if [[ ! -d "$worktree" ]]; then
  # Not fatal: pr-install builds the PR head in its canonical detached tree.
  worktree=""
fi

bash "$SCRIPT_DIR/bs-slot.sh" validate working "pr=$pr" "detail=installing head ${head_oid:0:8}"
rc=0
(cd "$alpha_root" && just pr-install "$pr") || rc=$?
if [[ "$rc" -ne 0 ]]; then
  bash "$SCRIPT_DIR/bs-slot.sh" validate failed "pr=$pr" "detail=pr-install exited $rc"
  echo "bs-validate-setup: 'just pr-install $pr' failed with exit $rc — see output above; provenance in ~/.plexi-pr-$pr/install.log if the install got that far." >&2
  exit "$rc"
fi
bash "$SCRIPT_DIR/bs-slot.sh" validate done "pr=$pr" "detail=installed head ${head_oid:0:8} as plexi-pr-$pr"

echo "BRANCH=$branch"
echo "WORKTREE=$worktree"
echo "HEAD=$head_oid"
