#!/usr/bin/env bash
# Run from the calling worktree before pr-install selects its build tree.
set -euo pipefail

pr="$1"
head_ref="$2"
api_oid="$3"
branch_ref="refs/heads/$head_ref"

if ! remote_ref="$(git ls-remote --exit-code --refs origin "$branch_ref")"; then
    printf 'Error: PR #%s GitHub API head %s could not be verified against origin/%s. Wait and retry.\n' \
        "$pr" "$api_oid" "$head_ref" >&2
    exit 1
fi
read -r remote_oid _ <<< "$remote_ref"
if [[ "$api_oid" != "$remote_oid" ]]; then
    printf 'Error: PR #%s GitHub API head %s disagrees with origin/%s at %s. Wait for GitHub to catch up and retry.\n' \
        "$pr" "$api_oid" "$head_ref" "$remote_oid" >&2
    exit 1
fi

if local_ref="$(git symbolic-ref --quiet HEAD)" && [[ "$local_ref" == "$branch_ref" ]]; then
    local_oid="$(git rev-parse HEAD)"
    if [[ "$local_oid" != "$remote_oid" ]]; then
        printf 'Error: PR #%s local HEAD %s disagrees with origin/%s and the GitHub API at %s. Bring the worktree and pushed branch into sync, then wait and retry.\n' \
            "$pr" "$local_oid" "$head_ref" "$remote_oid" >&2
        exit 1
    fi
fi
