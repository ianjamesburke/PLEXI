#!/usr/bin/env bash
# Bump version, regenerate CHANGELOG via git-cliff, commit, and tag the release commit.
# Usage: scripts/release-version.sh [patch|minor|major]   — called by `just bump`
# Default: patch
# After bumping: just promote beta release → just promote main release
set -euo pipefail

REPO_ROOT=$(dirname "$(git rev-parse --git-common-dir)")
TREE="$REPO_ROOT"

die() { echo "error: $*" >&2; exit 1; }

# ── validate args ─────────────────────────────────────────────────────────────

arg="${1:-patch}"
prekind=""
case "$arg" in
    patch|minor|major) bump="$arg" ;;
    --alpha) prekind="alpha" ;;
    --beta) prekind="beta" ;;
    *) die "unknown arg '$arg' — must be: patch | minor | major | --alpha | --beta" ;;
esac

# ── require clean state ───────────────────────────────────────────────────────

git -C "$TREE" diff --quiet && git -C "$TREE" diff --cached --quiet \
    || die "alpha has uncommitted changes — commit first"

# ── prerelease cut: tag only, no version bump, no changelog ───────────────────

if [[ -n "$prekind" ]]; then
    current=$(grep '^version' "$TREE/Cargo.toml" | head -1 | sed 's/version = "\(.*\)"/\1/')
    base=$(echo "$current" | sed 's/-.*//')
    git -C "$TREE" fetch origin --tags --quiet 2>/dev/null || true
    highest=$(git -C "$TREE" tag -l "v${base}-${prekind}.*" --sort=-v:refname | head -1)
    if [[ -n "$highest" ]]; then
        n=$(echo "$highest" | sed "s/.*-${prekind}\.//")
        next=$((n + 1))
    else
        next=1
    fi
    tag="v${base}-${prekind}.${next}"
    if git -C "$TREE" rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
        die "tag $tag already exists — refusing to reuse a release boundary"
    fi
    echo "Cutting $prekind prerelease: $tag (base $base)"
    git -C "$TREE" tag "$tag"
    echo ""
    echo "$tag tagged locally on alpha."
    echo "Next: just promote beta release"
    exit 0
fi

# ── require git-cliff ─────────────────────────────────────────────────────────

command -v git-cliff >/dev/null 2>&1 || die "git-cliff not found — brew install git-cliff"

# ── calculate new version ─────────────────────────────────────────────────────

current=$(grep '^version' "$TREE/Cargo.toml" | head -1 | sed 's/version = "\(.*\)"/\1/')
base=$(echo "$current" | sed 's/-.*//')

# ── refuse an empty bump ──────────────────────────────────────────────────────
# Cargo.toml's current version is the last release commit's tag. If nothing has
# landed on alpha since that tag, bumping again produces a version + changelog
# entry with no changes (see v0.2.2).

if git -C "$TREE" rev-parse -q --verify "refs/tags/v$current" >/dev/null; then
    unreleased=$(git -C "$TREE" log "v$current..HEAD" --oneline | wc -l | tr -d ' ')
    [[ "$unreleased" -gt 0 ]] || die "no commits since v$current — nothing to release"
fi

IFS='.' read -r major minor patch <<< "$base"

case "$bump" in
    patch) new="$major.$minor.$((patch + 1))" ;;
    minor) new="$major.$((minor + 1)).0" ;;
    major) new="$((major + 1)).0.0" ;;
esac

echo "Bumping $current → $new ($bump)..."
tag="v$new"

if git -C "$TREE" rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
    die "tag $tag already exists — refusing to reuse a release boundary"
fi

# ── bump Cargo.toml ───────────────────────────────────────────────────────────

sed -i '' "s/^version = \"$current\"/version = \"$new\"/" "$TREE/Cargo.toml"
(cd "$TREE" && cargo generate-lockfile --quiet 2>/dev/null || cargo generate-lockfile)

# ── bump Python SDK ───────────────────────────────────────────────────────────

sdk_toml="$TREE/sdk/python/pyproject.toml"
sdk_current=$(grep '^version' "$sdk_toml" | head -1 | sed 's/version = "\(.*\)"/\1/')
sed -i '' "s/^version = \"$sdk_current\"/version = \"$new\"/" "$sdk_toml"
echo "SDK: $sdk_current → $new"

# ── stamp agent skill ─────────────────────────────────────────────────────────
# plexi_version in the skill frontmatter must track Cargo.toml; the
# skill_version_matches_binary test fails otherwise. See skills/AGENTS.md.

skill_md="$TREE/skills/plexi-cli/SKILL.md"
sed -i '' "s/^plexi_version: \".*\"/plexi_version: \"$new\"/" "$skill_md"
grep -q "^plexi_version: \"$new\"" "$skill_md" || die "failed to stamp plexi_version in $skill_md"
echo "Skill: plexi_version → $new"

# ── update CHANGELOG via git-cliff ───────────────────────────────────────────

echo "Generating changelog..."
(cd "$TREE" && git-cliff \
    --config cliff.toml \
    --unreleased \
    --tag "v$new" \
    --prepend CHANGELOG.md)

# ── update generated docs ─────────────────────────────────────────────────────

echo "Regenerating CLI docs..."
(cd "$TREE" && just gen-cli-docs)

# ── commit ────────────────────────────────────────────────────────────────────

git -C "$TREE" add Cargo.toml Cargo.lock CHANGELOG.md website/src/content/docs/cli.md sdk/python/pyproject.toml skills/plexi-cli/SKILL.md
git -C "$TREE" commit -m "chore: release v$new"
git -C "$TREE" tag "$tag"

echo ""
echo "$tag committed and tagged locally on alpha."
echo "Next: just promote beta release"
