#!/usr/bin/env bash
# Usage: scripts/bump-alpha.sh
# Bumps the patch version in Cargo.toml, updates CHANGELOG.md with commits
# since the last alpha bump, and creates a commit. Must be run from the repo root.
set -euo pipefail

current=$(grep '^version' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')
base=$(echo "$current" | sed 's/-.*//')
IFS='.' read -r major minor patch <<< "$base"
new="$major.$minor.$((patch + 1))"

sed -i '' "s/^version = \"$current\"/version = \"$new\"/" Cargo.toml
cargo generate-lockfile

# Collect commits since last alpha bump, stripping merge commits and chore: DEV_LOG entries
last_bump=$(git log --grep="^chore: bump alpha" -1 --format="%H")
if [[ -n "$last_bump" ]]; then
    range="$last_bump..HEAD"
else
    range="HEAD~20..HEAD"
fi

commits=()
while IFS= read -r line; do [[ -n "$line" ]] && commits+=("$line"); done < <(git log "$range" --no-merges --format="%s" | grep -v '^chore: DEV_LOG' | grep -v '^chore: bump' || true)

today=$(TZ=America/New_York date +%Y-%m-%d)
time_et=$(TZ=America/New_York date +%H:%M)

section="## [$new] — $today $time_et ET"$'\n\n'"### Changes"
for c in "${commits[@]}"; do
    section="$section"$'\n'"- $c"
done

# Insert before the first ## version header (ENVIRON avoids awk -v newline parsing bug)
PLEXI_SECTION="$section" awk '
    /^## / && !inserted { printf "%s\n\n", ENVIRON["PLEXI_SECTION"]; inserted=1 }
    { print }
' CHANGELOG.md > CHANGELOG.md.tmp
mv CHANGELOG.md.tmp CHANGELOG.md

git add Cargo.toml Cargo.lock CHANGELOG.md

footer="chore: bump alpha to $new"
if [[ ${#commits[@]} -eq 0 ]]; then
    msg="$footer"
elif [[ ${#commits[@]} -eq 1 ]]; then
    msg="${commits[0]}"$'\n\n'"$footer"
else
    body=""
    for c in "${commits[@]}"; do body="$body"$'\n'"- $c"; done
    msg="${body:1}"$'\n\n'"$footer"
fi

git commit -m "$msg"
echo "Bumped to $new"
