#!/usr/bin/env bash
# Plexi channel promotion pipeline.
# Usage: promote.sh [beta|main]
#   No argument: auto-detects current branch and prompts for confirmation.
#   With argument: skips prompt (useful for scripting).
#
# Run `just bump` on alpha before promoting to beta if you haven't already.
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
    if [[ "$n" -gt 0 ]]; then
        echo "info: $label has $n unpushed commit(s) — pushing..."
        git -C "$tree" push origin "$branch"
    fi
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

promote_alpha_to_beta() {
    check_clean "$ALPHA_TREE" "alpha"
    check_pushed "$ALPHA_TREE" "alpha" "alpha"
    check_clean "$BETA_TREE" "beta"

    local version
    version=$(grep '^version' "$ALPHA_TREE/Cargo.toml" | head -1 | sed 's/version = "\(.*\)"/\1/')

    echo "Force-pushing alpha to beta and syncing worktree..."
    git push origin alpha:beta --force-with-lease
    git -C "$BETA_TREE" pull

    echo "v$version is on beta."
}

if [[ "$to" == "beta" ]]; then
    promote_alpha_to_beta

    version=$(grep '^version' "$ALPHA_TREE/Cargo.toml" | head -1 | sed 's/version = "\(.*\)"/\1/')
    echo ""
    echo "v$version is on beta — worktrees/beta/ is synced and ready."

    read -r -p "Install beta now? [y/N] " install_confirm
    if [[ "$install_confirm" == "y" || "$install_confirm" == "Y" ]]; then
        echo "Installing beta..."
        (cd "$BETA_TREE" && just install)
    fi
    exit 0
fi

# ── promote beta → main (optionally chaining through alpha first) ─────────────

if [[ "$current_branch" == "alpha" ]]; then
    echo "On alpha — promoting alpha → beta first..."
    promote_alpha_to_beta
    echo ""
fi

check_clean "$BETA_TREE" "beta"
check_pushed "$BETA_TREE" "beta" "beta"
check_clean "$REPO_ROOT" "main worktree"

version=$(grep '^version' "$BETA_TREE/Cargo.toml" | head -1 | sed 's/version = "\(.*\)"/\1/')

echo "Promoting beta → main (v$version)..."
git push origin beta:main

echo "Syncing main worktree..."
git -C "$REPO_ROOT" pull origin main

if git -C "$REPO_ROOT" tag -l "v$version" | grep -q "v$version"; then
    echo "Tag v$version already exists — skipping tag creation."
else
    git -C "$REPO_ROOT" tag "v$version"
fi

if git ls-remote --tags origin "v$version" | grep -q "v$version"; then
    echo "Tag v$version already on remote — skipping push."
else
    git -C "$REPO_ROOT" push origin "v$version"
fi

echo ""
echo "Released v$version — GitHub Actions release workflow will run."
