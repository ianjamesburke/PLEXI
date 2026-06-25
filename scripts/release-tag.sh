#!/usr/bin/env bash
# Push a version tag to trigger the GitHub Actions release workflow.
#
# Stable tags (vX.Y.Z) are pushed from the main worktree (run `just promote main`
# first). Prerelease tags (vX.Y.Z-alpha.N / vX.Y.Z-beta.N) are pushed from alpha,
# where they were cut by `scripts/release-version.sh --alpha|--beta`.
set -euo pipefail

REPO_ROOT=$(dirname "$(git rev-parse --git-common-dir)")
ALPHA_TREE="$REPO_ROOT"
MAIN_TREE="$REPO_ROOT/worktrees/main"

die() { echo "error: $*" >&2; exit 1; }

push_tag() {
    local tree="$1" tag="$2"
    remote_tag=$(git ls-remote --tags origin "refs/tags/$tag" | awk '{print $1}')
    if [[ -n "$remote_tag" ]]; then
        echo "Tag $tag already on remote — GitHub Actions release already triggered."
    else
        echo "Pushing tag $tag to origin..."
        git -C "$tree" push origin "$tag"
        echo "GitHub Actions release workflow triggered for $tag."
    fi
}

# ── prerelease flow: newest alpha/beta tag on alpha ───────────────────────────

base=$(grep '^version' "$ALPHA_TREE/Cargo.toml" | head -1 | sed 's/version = "\(.*\)"/\1/' | sed 's/-.*//')
pre_tag=$(git -C "$ALPHA_TREE" tag -l "v${base}-alpha.*" "v${base}-beta.*" --sort=-v:refname | head -1)

if [[ -n "$pre_tag" ]]; then
    tagged_commit=$(git -C "$ALPHA_TREE" rev-list -n 1 "$pre_tag")
    head_commit=$(git -C "$ALPHA_TREE" rev-parse HEAD)
    remote_tag=$(git ls-remote --tags origin "refs/tags/$pre_tag" | awk '{print $1}')
    if [[ -z "$remote_tag" ]]; then
        [[ "$tagged_commit" == "$head_commit" ]] \
            || die "$pre_tag points at $tagged_commit, not alpha HEAD $head_commit — re-cut the prerelease"
        push_tag "$ALPHA_TREE" "$pre_tag"
        exit 0
    fi
fi

# ── stable flow: vX.Y.Z from the main worktree ────────────────────────────────

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

push_tag "$MAIN_TREE" "v$version"
