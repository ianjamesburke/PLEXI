#!/usr/bin/env bash
# Print the release tag a push to <branch> at <sha> publishes, or nothing when
# the branch has no commits since its last release. Pure: creates no tag.
#   alpha -> vX.Y.Z-alpha.N   beta -> vX.Y.Z-beta.N   main -> vX.Y.Z
# X.Y.Z is Cargo.toml's version at <sha>. Once vX.Y.Z is released, prereleases
# count on the next patch: semver ranks X.Y.Z-alpha.N below X.Y.Z and the
# updater only offers newer versions (select in crates/distribution/src/release.rs).
# main exits 1 when vX.Y.Z is released and <sha> adds commits: bump first.
# Usage: release-next-tag.sh <alpha|beta|main> <sha>   (full history and tags)
set -euo pipefail

die() { echo "error: $*" >&2; exit 1; }

[[ $# -eq 2 ]] || die "usage: release-next-tag.sh <alpha|beta|main> <sha>"
branch=$1
sha=$2
git rev-parse -q --verify "${sha}^{commit}" >/dev/null || die "commit $sha not found"
version=$(git show "${sha}:Cargo.toml" | python3 -c 'import sys, tomllib; print(tomllib.load(sys.stdin.buffer)["package"]["version"])') \
    || die "could not read [package] version from Cargo.toml at $sha"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "Cargo.toml version '$version' at $sha is not X.Y.Z"

released() { git rev-parse -q --verify "refs/tags/$1" >/dev/null; }
has_commits_since() { [[ -n "$(git rev-list "$1..$sha" --max-count=1)" ]]; }

case "$branch" in
    main)
        released "v$version" || { echo "v$version"; exit 0; }
        ! has_commits_since "v$version" \
            || die "main has commits since v$version but Cargo.toml is still $version: run 'just bump' on alpha, then promote"
        ;;
    alpha|beta)
        if released "v$version"; then
            IFS=. read -r major minor patch <<< "$version"
            version="$major.$minor.$((patch + 1))"
        fi
        latest=$(git tag -l "v$version-$branch.*" --sort=-v:refname | head -1)
        [[ -n "$latest" ]] || { echo "v$version-$branch.1"; exit 0; }
        has_commits_since "$latest" || exit 0
        n=${latest##*.}
        [[ "$n" =~ ^[0-9]+$ ]] || die "unparseable $branch tag: $latest"
        echo "v$version-$branch.$((n + 1))"
        ;;
    *) die "unknown branch '$branch': must be alpha, beta or main" ;;
esac
