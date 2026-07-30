#!/usr/bin/env bash
# Cut and publish a source-build release tag for a channel. Never moves code
# between branches — that's `just promote`. Run this only after
# `just promote beta|main` has landed on the target branch and you're ready
# to make the result live for that channel's auto-updaters.
# Usage: release-tag.sh [beta|main]
set -euo pipefail

REPO_ROOT=$(dirname "$(git rev-parse --git-common-dir)")
BETA_TREE="$REPO_ROOT/worktrees/beta"
MAIN_TREE="$REPO_ROOT/worktrees/main"

die() { echo "error: $*" >&2; exit 1; }

channel="${1:-}"
case "$channel" in
    beta|main) ;;
    *) die "usage: release-tag.sh [beta|main]" ;;
esac

if [[ "$channel" == "beta" ]]; then
    tree="$BETA_TREE"
    branch="beta"
else
    tree="$MAIN_TREE"
    branch="main"
fi

git -C "$tree" diff --quiet && git -C "$tree" diff --cached --quiet \
    || die "$branch worktree has uncommitted changes — commit or restore first"

[[ $(git -C "$tree" rev-parse --abbrev-ref HEAD) == "$branch" ]] \
    || die "$tree worktree is not on '$branch'"

# The tag must pin the exact commit that was promoted — refuse to tag a
# worktree that's stale or has drifted from origin, since either means this
# isn't the commit `just promote $branch` actually landed.
git -C "$tree" fetch origin "$branch" --quiet

local_head=$(git -C "$tree" rev-parse HEAD)
remote_head=$(git -C "$tree" rev-parse "origin/$branch")
[[ "$local_head" == "$remote_head" ]] \
    || die "$branch worktree ($local_head) does not match origin/$branch ($remote_head) — run 'just promote $branch' first, or pull"

version=$(grep '^version' "$tree/Cargo.toml" | head -1 | sed 's/version = "\(.*\)"/\1/')

if [[ "$channel" == "beta" ]]; then
    base=$(echo "$version" | sed 's/-.*//')
    git -C "$tree" fetch origin --tags --quiet 2>/dev/null || true
    highest=$(git -C "$tree" tag -l "v${base}-beta.*" --sort=-v:refname | head -1)
    if [[ -n "$highest" ]]; then
        n=$(echo "$highest" | sed 's/.*-beta\.//')
        next=$((n + 1))
    else
        next=1
    fi
    tag="v${base}-beta.${next}"
    echo "Cutting $tag on beta ($local_head)..."
    git -C "$tree" tag "$tag" "$local_head"
    git -C "$tree" push origin "$tag"
    echo ""
    echo "Published $tag for source-build updates."
else
    tag="v$version"
    if git -C "$tree" tag -l "$tag" | grep -q "$tag"; then
        tagged_commit=$(git -C "$tree" rev-list -n 1 "$tag")
        if [[ "$tagged_commit" != "$local_head" ]]; then
            echo "info: tag $tag points at an older commit — re-tagging at main HEAD..."
            git -C "$tree" tag -f "$tag" "$local_head"
        fi
    else
        echo "Creating tag $tag at main HEAD..."
        git -C "$tree" tag "$tag" "$local_head"
    fi
    echo "Publishing tag $tag for source-build updates..."
    git -C "$tree" push origin "$tag" --force
    echo ""
    echo "REMINDER: republish the agent-skill mirror from this release tree —"
    echo "          copy $tree/skills/plexi-cli/SKILL.md to ianjamesburke/plexi-skills,"
    echo "          push main, tag $tag. Steps: skills/AGENTS.md."
fi

echo "$tag is live."
