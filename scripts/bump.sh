#!/usr/bin/env bash
# Usage: scripts/bump.sh
# Interactive version bump for releases. Verifies build compiles, prompts for
# bump type, updates Cargo.toml, commits, and tags. Must be run from the repo root.
set -e

echo "Verifying release build compiles..."
cargo build --release

current=$(grep '^version' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')
echo "Current version: $current"
echo "1) prerelease (increment beta number)"
echo "2) patch"
echo "3) minor"
echo "4) major"
read -r -p "Bump type [1-4]: " choice

base=$(echo "$current" | sed 's/-.*//')
IFS='.' read -r major minor patch <<< "$base"

case $choice in
    1)
        pre=$(echo "$current" | grep -oE '\-[a-z]+\.[0-9]+$' | head -1)
        if [[ -z "$pre" ]]; then
            echo "Error: current version has no prerelease suffix to increment"
            exit 1
        fi
        pre_label=$(echo "$pre" | sed 's/-//;s/\.[0-9]*//')
        pre_num=$(echo "$pre" | grep -oE '[0-9]+$')
        new="$base-$pre_label.$((pre_num + 1))"
        ;;
    2) new="$major.$minor.$((patch + 1))" ;;
    3) new="$major.$((minor + 1)).0" ;;
    4) new="$((major + 1)).0.0" ;;
    *) echo "Invalid choice"; exit 1 ;;
esac

sed -i '' "s/^version = \"$current\"/version = \"$new\"/" Cargo.toml
cargo generate-lockfile
echo "Bumped to $new"
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to $new"
git tag "v$new"
echo "Tagged v$new"
