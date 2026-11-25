#!/usr/bin/env bash
# Bump version and regenerate CHANGELOG via git-cliff, as one commit. Tags
# nothing: release.yml tags vX.Y.Z when the commit reaches main.
# Usage: scripts/release-version.sh [patch|minor|major]   — called by `just bump`
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
if [[ "${1:-}" == --alpha-lock-inner ]]; then
    shift
    python3 -I "$SCRIPT_DIR/alpha_lock.py" --verify
else
    exec python3 -I "$SCRIPT_DIR/alpha_lock.py" bash "$SCRIPT_DIR/release-version.sh" --alpha-lock-inner "$@"
fi
REPO_ROOT=$(dirname "$(git rev-parse --path-format=absolute --git-common-dir)")
TREE="$REPO_ROOT"

die() { echo "error: $*" >&2; exit 1; }

# ── validate args ─────────────────────────────────────────────────────────────

bump="${1:-patch}"
case "$bump" in
    patch|minor|major) ;;
    *) die "unknown arg '$bump' — must be: patch | minor | major" ;;
esac

# ── require clean state ───────────────────────────────────────────────────────

git -C "$TREE" diff --quiet && git -C "$TREE" diff --cached --quiet \
    || die "alpha has uncommitted changes — commit first"

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
(cd "$TREE" && cargo update --workspace)

# ── bump Python SDK ───────────────────────────────────────────────────────────

sdk_toml="$TREE/sdk/python/pyproject.toml"
sdk_current=$(grep '^version' "$sdk_toml" | head -1 | sed 's/version = "\(.*\)"/\1/')
sed -i '' "s/^version = \"$sdk_current\"/version = \"$new\"/" "$sdk_toml"
# uv.lock records the workspace's own editable package with its version, the
# same way Cargo.lock records the crate's. Re-lock in the same breath as the
# bump or the lock trails one release behind, every release.
(cd "$TREE/sdk/python" && uv lock --quiet)
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
(cd "$TREE" && bash scripts/gen-doc-safe.sh website/src/content/docs/cli.md -- \
    bash scripts/cargo-with-lease.sh --class short cargo run -p gen_cli_docs)

# ── commit ────────────────────────────────────────────────────────────────────

git -C "$TREE" add Cargo.toml Cargo.lock CHANGELOG.md website/src/content/docs/cli.md sdk/python/pyproject.toml sdk/python/uv.lock skills/plexi-cli/SKILL.md
git -C "$TREE" commit -m "chore: release v$new"

echo ""
echo "v$new committed on alpha. Push alpha, then: just promote beta, just promote main."
