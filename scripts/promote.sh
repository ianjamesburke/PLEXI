#!/usr/bin/env bash
# Plexi channel promotion pipeline.
# Usage: promote.sh [beta|main]
#   No argument: auto-detects current branch and prompts for confirmation.
#   With argument: skips prompt (useful for scripting).
set -euo pipefail

REPO_ROOT=$(dirname "$(git rev-parse --git-common-dir)")
ALPHA_TREE="$REPO_ROOT/worktrees/alpha"
BETA_TREE="$REPO_ROOT/worktrees/beta"

# ── helpers ───────────────────────────────────────────────────────────────────

die() { echo "error: $*" >&2; exit 1; }

check_clean() {
    local tree="$1" label="$2"
    git -C "$tree" diff --quiet && git -C "$tree" diff --cached --quiet \
        || die "$label has uncommitted changes — commit first"
}

check_pushed() {
    local tree="$1" branch="$2" label="$3"
    local n
    n=$(git -C "$tree" log "origin/$branch..$branch" --oneline 2>/dev/null | wc -l | tr -d ' ')
    [[ "$n" -eq 0 ]] || die "$label has $n unpushed commit(s) — run: git push from $label"
}

bump_patch() {
    local tree="$1"
    local current base major minor patch new
    current=$(grep '^version' "$tree/Cargo.toml" | head -1 | sed 's/version = "\(.*\)"/\1/')
    base=$(echo "$current" | sed 's/-.*//')
    IFS='.' read -r major minor patch <<< "$base"
    new="$major.$minor.$((patch + 1))"
    sed -i '' "s/^version = \"$current\"/version = \"$new\"/" "$tree/Cargo.toml"
    (cd "$tree" && cargo generate-lockfile)
    echo "$new"
}

prepend_changelog() {
    local tree="$1" ver="$2"
    local today commits_file tmp
    today=$(date '+%Y-%m-%d')
    commits_file=$(mktemp)
    git -C "$tree" log origin/main..alpha --oneline --no-merges \
        | sed 's/^[a-f0-9]* /- /' > "$commits_file"
    [[ -s "$commits_file" ]] || echo "- (no new commits since last release)" > "$commits_file"
    tmp=$(mktemp)
    awk -v ver="$ver" -v dt="$today" -v cf="$commits_file" '
        /^## \[/ && !inserted {
            print "## [" ver "] — " dt
            print ""
            print "### Changes"
            while ((getline line < cf) > 0) print line
            print ""
            inserted=1
        }
        { print }
    ' "$tree/CHANGELOG.md" > "$tmp"
    rm -f "$commits_file"
    mv "$tmp" "$tree/CHANGELOG.md"
}

# ── resolve target ────────────────────────────────────────────────────────────

current_branch=$(git rev-parse --abbrev-ref HEAD)
to="${1:-}"

if [[ -z "$to" ]]; then
    case "$current_branch" in
        alpha) to="beta" ;;
        beta)  to="main" ;;
        *) die "not on alpha or beta (on '$current_branch') — pass target explicitly: promote.sh beta|main" ;;
    esac
    read -r -p "Promote $current_branch → $to? [y/N] " confirm
    [[ "$confirm" == "y" || "$confirm" == "Y" ]] || { echo "Aborted."; exit 0; }
fi

case "$to" in
    beta) ;;
    main) ;;
    *) die "unknown target '$to' — must be: beta | main" ;;
esac

# ── promote alpha → beta ──────────────────────────────────────────────────────

if [[ "$to" == "beta" ]]; then
    check_clean "$ALPHA_TREE" "alpha"
    check_pushed "$ALPHA_TREE" "alpha" "alpha"
    check_clean "$BETA_TREE" "beta"

    echo "Bumping version and updating changelog on alpha..."
    new=$(bump_patch "$ALPHA_TREE")
    prepend_changelog "$ALPHA_TREE" "$new"
    git -C "$ALPHA_TREE" add Cargo.toml Cargo.lock CHANGELOG.md
    git -C "$ALPHA_TREE" commit -m "chore: promote to beta — v$new"
    git -C "$ALPHA_TREE" push

    echo "Force-pushing alpha to beta and syncing worktree..."
    git push origin alpha:beta --force-with-lease
    git -C "$BETA_TREE" pull

    echo ""
    echo "v$new is on beta — worktrees/beta/ is synced and ready."
    exit 0
fi

# ── promote beta → main ───────────────────────────────────────────────────────

check_clean "$BETA_TREE" "beta"
check_pushed "$BETA_TREE" "beta" "beta"
check_clean "$REPO_ROOT" "main worktree"

version=$(grep '^version' "$BETA_TREE/Cargo.toml" | head -1 | sed 's/version = "\(.*\)"/\1/')

echo "Promoting beta → main (v$version)..."
git push origin beta:main

echo "Syncing main worktree..."
git -C "$REPO_ROOT" pull origin main

git -C "$REPO_ROOT" tag "v$version"
git -C "$REPO_ROOT" push origin "v$version"

echo ""
echo "Released v$version — tag pushed, GitHub Actions release workflow will run."
