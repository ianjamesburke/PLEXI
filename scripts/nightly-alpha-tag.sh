#!/usr/bin/env bash
# Print the next alpha tag (vX.Y.Z-alpha.N) for the current Cargo.toml base
# when alpha has commits since the latest alpha tag; print nothing otherwise.
# Pure: creates no tag. Caller (nightly.yml) tags and dispatches release.yml.
# Usage: nightly-alpha-tag.sh   (run in a full-history checkout with tags)
set -euo pipefail

die() { echo "error: $*" >&2; exit 1; }

version=$(grep -m1 '^version' Cargo.toml | sed 's/version = "\(.*\)"/\1/')
[[ -n "$version" ]] || die "no version in Cargo.toml"
base=${version%%-*}

latest=$(git tag -l "v${base}-alpha.*" --sort=-v:refname | head -1)
if [[ -z "$latest" ]]; then
    echo "v${base}-alpha.1"
    exit 0
fi

if [[ -z "$(git rev-list "${latest}..HEAD" --max-count=1)" ]]; then
    exit 0
fi

n=${latest##*-alpha.}
[[ "$n" =~ ^[0-9]+$ ]] || die "unparseable alpha tag: $latest"
echo "v${base}-alpha.$((n + 1))"
