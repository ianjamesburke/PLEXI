#!/usr/bin/env bash
# Bump version, regenerate CHANGELOG via git-cliff, and commit.
# Usage: scripts/release-version.sh [patch|minor|major]
# Default: patch
# After this, run: just promote [beta|main]
set -euo pipefail

REPO_ROOT=$(dirname "$(git rev-parse --git-common-dir)")
TREE="$REPO_ROOT"

die() { echo "error: $*" >&2; exit 1; }

# ── validate args ─────────────────────────────────────────────────────────────

bump="${1:-patch}"
case "$bump" in
    patch|minor|major) ;;
    *) die "unknown bump type '$bump' — must be: patch | minor | major" ;;
esac

# ── require clean state ───────────────────────────────────────────────────────

git -C "$TREE" diff --quiet && git -C "$TREE" diff --cached --quiet \
    || die "alpha has uncommitted changes — commit first"

# ── require git-cliff ─────────────────────────────────────────────────────────

command -v git-cliff >/dev/null 2>&1 || die "git-cliff not found — brew install git-cliff"

# ── calculate new version ─────────────────────────────────────────────────────

current=$(grep '^version' "$TREE/Cargo.toml" | head -1 | sed 's/version = "\(.*\)"/\1/')
base=$(echo "$current" | sed 's/-.*//')
IFS='.' read -r major minor patch <<< "$base"

case "$bump" in
    patch) new="$major.$minor.$((patch + 1))" ;;
    minor) new="$major.$((minor + 1)).0" ;;
    major) new="$((major + 1)).0.0" ;;
esac

echo "Bumping $current → $new ($bump)..."

# ── bump Cargo.toml ───────────────────────────────────────────────────────────

sed -i '' "s/^version = \"$current\"/version = \"$new\"/" "$TREE/Cargo.toml"
(cd "$TREE" && cargo generate-lockfile --quiet 2>/dev/null || cargo generate-lockfile)

# ── update CHANGELOG via git-cliff ───────────────────────────────────────────

echo "Generating changelog..."
(cd "$TREE" && git-cliff \
    --config cliff.toml \
    --unreleased \
    --tag "v$new" \
    --prepend CHANGELOG.md)

# ── commit ────────────────────────────────────────────────────────────────────

git -C "$TREE" add Cargo.toml Cargo.lock CHANGELOG.md
git -C "$TREE" commit -m "chore: release v$new"

echo ""
echo "v$new committed on alpha."
echo "Next: just promote beta"
