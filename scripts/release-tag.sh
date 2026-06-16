#!/usr/bin/env bash
# Push the version tag for the current Cargo.toml version.
# Tags main HEAD if needed, then pushes the tag to trigger the GitHub Actions release workflow.
# Run `just promote main` first to ensure main is up to date.
set -euo pipefail

REPO_ROOT=$(dirname "$(git rev-parse --git-common-dir)")
MAIN_TREE="$REPO_ROOT/worktrees/main"

die() { echo "error: $*" >&2; exit 1; }

[[ $(git -C "$MAIN_TREE" rev-parse --abbrev-ref HEAD) == "main" ]] \
    || die "main worktree is not on 'main' branch"

git -C "$MAIN_TREE" fetch origin main
local_main=$(git -C "$MAIN_TREE" rev-parse HEAD)
remote_main=$(git -C "$MAIN_TREE" rev-parse origin/main)
[[ "$local_main" == "$remote_main" ]] \
    || die "main worktree is not in sync with origin/main — run 'just promote main' first"

version=$(grep '^version' "$MAIN_TREE/Cargo.toml" | head -1 | sed 's/version = "\(.*\)"/\1/')

if git -C "$MAIN_TREE" tag -l "v$version" | grep -q "v$version"; then
    tagged_commit=$(git -C "$MAIN_TREE" rev-list -n 1 "v$version")
    if [[ "$tagged_commit" != "$local_main" ]]; then
        die "tag v$version points at $tagged_commit, not main HEAD $local_main — run 'just bump' on alpha first"
    fi
    echo "Tag v$version already exists at main HEAD."
else
    echo "Creating tag v$version at main HEAD..."
    git -C "$MAIN_TREE" tag "v$version"
fi

remote_tag=$(git ls-remote --tags origin "refs/tags/v$version" | awk '{print $1}')
if [[ -n "$remote_tag" ]]; then
    echo "Tag v$version already on remote — GitHub Actions release already triggered."
else
    echo "Pushing tag v$version to origin..."
    git -C "$MAIN_TREE" push origin "v$version"
    echo "GitHub Actions release workflow triggered for v$version."
fi
