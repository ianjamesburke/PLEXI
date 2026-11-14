#!/usr/bin/env bash
# Print the next alpha tag (vX.Y.Z-alpha.N) for the tested commit when it has
# commits since the latest alpha tag; print nothing otherwise. The tested
# commit must be on alpha: a commit the nightly tested that is not an ancestor
# of alpha is refused (exit 1). Commits pushed to alpha after the tested one
# wait for the next night; nothing untested is ever tagged (stint 1480).
# Pure: creates no tag. Caller (nightly.yml) tags the tested sha and
# dispatches release.yml.
# Usage: nightly-alpha-tag.sh <tested-sha> <alpha-ref>
#        (run in a full-history checkout with tags)
set -euo pipefail

die() { echo "error: $*" >&2; exit 1; }

[[ $# -eq 2 ]] || die "usage: nightly-alpha-tag.sh <tested-sha> <alpha-ref>"
tested=$1
alpha=$2
git rev-parse --verify -q "${tested}^{commit}" >/dev/null || die "tested commit $tested not found"
git rev-parse --verify -q "${alpha}^{commit}" >/dev/null || die "alpha ref $alpha not found"
git merge-base --is-ancestor "$tested" "$alpha" \
    || die "tested commit $tested is not an ancestor of alpha ($alpha); refusing to tag it"

version=$(git show "${tested}:Cargo.toml" | grep -m1 '^version' | sed 's/version = "\(.*\)"/\1/')
[[ -n "$version" ]] || die "no version in Cargo.toml at $tested"
base=${version%%-*}

latest=$(git tag -l "v${base}-alpha.*" --sort=-v:refname | head -1)
if [[ -z "$latest" ]]; then
    echo "v${base}-alpha.1"
    exit 0
fi

if [[ -z "$(git rev-list "${latest}..${tested}" --max-count=1)" ]]; then
    exit 0
fi

n=${latest##*-alpha.}
[[ "$n" =~ ^[0-9]+$ ]] || die "unparseable alpha tag: $latest"
echo "v${base}-alpha.$((n + 1))"
