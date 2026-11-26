#!/usr/bin/env bash
# Print the build tag for a source install of the alpha channel: vX.Y.Z-alpha.N.
# Alpha publishes no releases, so the number is counted on this machine.
# X.Y.Z follows release-next-tag.sh (the next patch once vX.Y.Z is released).
# N is one above the higher of the newest vX.Y.Z-alpha.* git tag and the
# installed plexi-alpha build, so each install counts up and N restarts at 1
# when X.Y.Z changes. Run from the repo root. Pure: creates no tag.
set -euo pipefail

version=$(python3 -c 'import tomllib; print(tomllib.load(open("Cargo.toml", "rb"))["package"]["version"])') \
    || { echo "error: could not read [package] version from Cargo.toml" >&2; exit 1; }
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "error: Cargo.toml version '$version' is not X.Y.Z" >&2; exit 1; }
if git rev-parse -q --verify "refs/tags/v$version" >/dev/null; then
    IFS=. read -r major minor patch <<< "$version"
    version="$major.$minor.$((patch + 1))"
fi

n=0
tagged=$(git tag -l "v$version-alpha.*" --sort=-v:refname | head -1)
if [[ "${tagged##*.}" =~ ^[0-9]+$ ]]; then n=${tagged##*.}; fi
if command -v plexi-alpha >/dev/null; then
    # A broken install must stay reinstallable: warn and count from the tags.
    if installed=$(plexi-alpha --version 2>&1); then
        if [[ "$installed" =~ (^|[[:space:]])${version//./\\.}-alpha\.([0-9]+)([[:space:]]|$) ]] && (( BASH_REMATCH[2] > n )); then
            n=${BASH_REMATCH[2]}
        fi
    else
        echo "warning: plexi-alpha --version failed ($installed); counting from git tags" >&2
    fi
fi
echo "v$version-alpha.$((n + 1))"
