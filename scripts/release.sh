#!/usr/bin/env bash
# Usage: scripts/release.sh
# Pushes the current branch and its version tag to origin, triggering the
# GitHub Actions release workflow. Must be run from the repo root.
set -e

branch=$(git rev-parse --abbrev-ref HEAD)
version=$(grep '^version' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')
tag="v$version"

if git ls-remote --tags origin "$tag" | grep -q "$tag"; then
    echo "Error: $tag already exists on remote. Run 'just bump' first."
    exit 1
fi
if ! git tag -l "$tag" | grep -q "$tag"; then
    echo "Error: local tag $tag not found. Run 'just bump' first."
    exit 1
fi

git push origin "$branch" "$tag"
echo "Pushed $tag from $branch — release workflow will run on GitHub Actions"
