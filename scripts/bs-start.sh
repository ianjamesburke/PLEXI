#!/usr/bin/env bash
# bs-start.sh — deterministic start of a stint lane (bs- pipeline).
#
# Usage: bs-start.sh [--base feature/alpha-cleanup] <stint-id> [<stint-id> ...]
#
# Asserts the selected base is clean and synced with origin (hard error naming what and
# where otherwise), mints the wtp worktree feature/stint-<id>[-<id>...]-<slug>
# from that base HEAD, verifies the base, claims every given stint, and publishes
# a slot report. Multiple ids form one batch lane: one worktree, one PR, and
# a branch name `just merge-pr` resolves all the stints from.
# Safe from any cwd inside the repo. The last stdout line is the worktree path.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

base_branch="alpha"
if [[ "${1:-}" == "--base" ]]; then
  base_branch="${2:-}"
  shift 2
  if [[ "$base_branch" != "feature/alpha-cleanup" ]]; then
    echo "bs-start: --base only accepts feature/alpha-cleanup" >&2
    exit 1
  fi
fi
if [[ $# -lt 1 ]]; then
  echo "bs-start: missing stint id. Usage: just bs-start <stint-id> [<stint-id> ...]" >&2
  exit 1
fi
stint_ids=("$@")

# Resolve the canonical alpha checkout — never the caller's cwd.
alpha_root="$(git worktree list --porcelain | awk '
  /^worktree / { path=substr($0, 10) }
  /^branch refs\/heads\/alpha$/ { print path; exit }
')"
if [[ -z "$alpha_root" ]]; then
  echo "bs-start: could not resolve the alpha worktree from 'git worktree list'." >&2
  exit 1
fi

base_root="$(git worktree list --porcelain | awk -v target="refs/heads/$base_branch" '
  /^worktree / { path=substr($0, 10) }
  /^branch / && $2 == target { print path; exit }
')"
if [[ -z "$base_root" ]]; then
  echo "bs-start: no worktree for base $base_branch" >&2
  exit 1
fi
if [[ "$base_branch" == "feature/alpha-cleanup" ]]; then
  # Refuse blocked product dispatch before worktree creation or stint claim.
  uv run --no-project python "$SCRIPT_DIR/babysitter-policy.py" preflight "${stint_ids[@]}" >/dev/null
fi

# The selected base must be clean.
git -C "$base_root" update-index -q --refresh || true
dirty="$(git -C "$base_root" status --porcelain)"
if [[ -n "$dirty" ]]; then
  echo "bs-start: $base_branch is dirty at $base_root — commit or restore these before starting a lane:" >&2
  echo "$dirty" >&2
  exit 1
fi

# The selected base must be synced with origin (neither ahead nor behind).
git -C "$base_root" fetch -q origin "$base_branch"
behind="$(git -C "$base_root" rev-list --count "HEAD..origin/$base_branch")"
ahead="$(git -C "$base_root" rev-list --count "origin/$base_branch..HEAD")"
if [[ "$behind" != "0" ]]; then
  echo "bs-start: $base_branch at $base_root is $behind commit(s) behind origin — sync it first." >&2
  exit 1
fi
if [[ "$ahead" != "0" ]]; then
  echo "bs-start: $base_branch at $base_root has $ahead unpushed commit(s) — push or drop them before starting a lane." >&2
  exit 1
fi

# Resolve every stint's task file; the first id's filename provides the slug.
slug=""
for stint_id in "${stint_ids[@]}"; do
  shopt -s nullglob
  task_files=("$alpha_root/.stint/tasks/${stint_id}"-*.md)
  shopt -u nullglob
  if [[ ${#task_files[@]} -eq 0 ]]; then
    echo "bs-start: no task file matches .stint/tasks/${stint_id}-*.md in $alpha_root — check the id with 'stint list'." >&2
    exit 1
  fi
  if [[ ${#task_files[@]} -gt 1 ]]; then
    echo "bs-start: stint id '$stint_id' is ambiguous — matches: ${task_files[*]}" >&2
    exit 1
  fi
  status="$(sed -n 's/^status: *//p' "${task_files[0]}" | head -1)"
  if [[ "$status" == "done" || "$status" == "archived" ]]; then
    echo "bs-start: stint $stint_id is '$status' (${task_files[0]}) — nothing to start." >&2
    exit 1
  fi
  if [[ -z "$slug" ]]; then
    slug="$(basename "${task_files[0]}" .md)"
    slug="${slug#"${stint_id}"-}"
    # Cap the slug at 6 hyphen-words to keep branch names sane.
    slug="$(printf '%s' "$slug" | cut -d- -f1-6)"
  fi
done

ids_joined="$(printf '%s-' "${stint_ids[@]}")"
branch="feature/stint-${ids_joined}${slug}"
worktree="$alpha_root/worktrees/$branch"

# Refuse to silently reuse prior state — that is bs-reset's job.
if [[ -d "$worktree" ]]; then
  echo "bs-start: worktree already exists: $worktree — resume there, or start over with 'just bs-reset ${stint_ids[0]}'." >&2
  exit 1
fi
if git -C "$alpha_root" show-ref --verify --quiet "refs/heads/$branch"; then
  echo "bs-start: local branch $branch already exists — resume it, or start over with 'just bs-reset ${stint_ids[0]}'." >&2
  exit 1
fi
if git -C "$alpha_root" ls-remote --exit-code --heads origin "$branch" >/dev/null 2>&1; then
  echo "bs-start: branch $branch already exists on origin — inspect it before re-minting a lane." >&2
  exit 1
fi

# Mint the worktree from the selected base (wtp, never raw git worktree).
(cd "$alpha_root" && wtp add -b "$branch" "refs/heads/$base_branch")

# Verify the new worktree sits exactly at the selected base HEAD.
alpha_head="$(git -C "$base_root" rev-parse HEAD)"
wt_head="$(git -C "$worktree" rev-parse HEAD)"
if [[ "$wt_head" != "$alpha_head" ]]; then
  echo "bs-start: worktree base mismatch: $worktree is at ${wt_head:0:8}, $base_branch HEAD is ${alpha_head:0:8}. Removing the bad worktree." >&2
  (cd "$alpha_root" && wtp remove -f --with-branch "$branch") || true
  exit 1
fi

# Claim every stint (stint owns state; run from the canonical checkout).
for stint_id in "${stint_ids[@]}"; do
  (cd "$alpha_root" && stint claim "$stint_id")
done

bash "$SCRIPT_DIR/bs-slot.sh" start done "stint=${stint_ids[*]}" "detail=branch=$branch head=${alpha_head:0:8}"
echo "bs-start: stint(s) ${stint_ids[*]} claimed, branch $branch from $base_branch at ${alpha_head:0:8}"
echo "$worktree"
