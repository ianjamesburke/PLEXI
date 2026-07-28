#!/usr/bin/env bash
# Plexi channel promotion pipeline.
# Usage: promote.sh [beta|main] [install] [release]
#   No argument: auto-detects current branch and prompts for confirmation.
#   install: build and install the target channel after promoting.
#   release: cut and push a prerelease tag (beta) or stable tag (main) for source-build updates.
#
# Run `just bump` on alpha before promoting if you haven't already.
set -euo pipefail

REPO_ROOT=$(dirname "$(git rev-parse --git-common-dir)")
ALPHA_TREE="$REPO_ROOT"
BETA_TREE="$REPO_ROOT/worktrees/beta"
MAIN_TREE="$REPO_ROOT/worktrees/main"

# ── helpers ───────────────────────────────────────────────────────────────────

die() { echo "error: $*" >&2; exit 1; }

check_version_bumped() {
    local tree="$1"
    local current
    current=$(grep '^version' "$tree/Cargo.toml" | head -1 | sed 's/version = "\(.*\)"/\1/')
    local base
    base=$(echo "$current" | sed 's/-.*//')
    local last_stable
    last_stable=$(git -C "$tree" tag --sort=-v:refname \
        | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' \
        | { while IFS= read -r tag; do
              git -C "$tree" merge-base --is-ancestor "$tag" origin/main 2>/dev/null \
                || git -C "$tree" merge-base --is-ancestor "$tag" origin/beta 2>/dev/null \
                && echo "$tag" && break
            done; } \
        | head -1 | sed 's/^v//')
    if [[ -n "$last_stable" && "$base" == "$last_stable" ]]; then
        echo ""
        echo "WARNING: Cargo.toml version ($current) matches the last stable tag (v$last_stable)."
        echo "         You may be promoting without a version bump."
        echo "         Run 'just bump' first, or proceed if this is intentional."
        echo ""
        read -r -p "Continue anyway? [y/N] " confirm
        [[ "$confirm" == "y" || "$confirm" == "Y" ]] || die "Aborted — run 'just bump' before promoting."
    fi
}

check_clean() {
    local tree="$1" label="$2"
    git -C "$tree" diff --quiet && git -C "$tree" diff --cached --quiet \
        || die "$label has uncommitted changes — commit first"
}

check_cargo_config() {
    local tree="$1"
    local cfg="$tree/.cargo/config.toml"
    [[ -f "$cfg" ]] || return 0
    if grep -Eq '/Users/|/home/' "$cfg"; then
        echo "error: $cfg contains a hardcoded absolute path:" >&2
        grep -En '/Users/|/home/' "$cfg" >&2
        die "fix .cargo/config.toml to use relative paths before promoting"
    fi
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
do_install=""
do_release=""
for arg in "${@:2}"; do
    case "$arg" in
        install) do_install="install" ;;
        release) do_release="release" ;;
        *) die "unknown flag '$arg' — valid flags: install, release" ;;
    esac
done

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
    check_version_bumped "$ALPHA_TREE"
    check_clean "$ALPHA_TREE" "alpha"
    check_cargo_config "$ALPHA_TREE"
    check_pushed "$ALPHA_TREE" "alpha" "alpha"
    check_clean "$BETA_TREE" "beta"

    echo "Checking CLI docs are up to date..."
    if ! (cd "$ALPHA_TREE" && just check-cli-docs) 2>/dev/null; then
        echo "CLI docs are stale — regenerating automatically..."
        (cd "$ALPHA_TREE" && just gen-cli-docs) \
            || die "Failed to regenerate CLI docs"
    fi

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

    if [[ "$do_release" == "release" || "$do_release" == "true" ]]; then
        base=$(echo "$version" | sed 's/-.*//')
        git -C "$ALPHA_TREE" fetch origin --tags --quiet 2>/dev/null || true
        highest=$(git -C "$ALPHA_TREE" tag -l "v${base}-beta.*" --sort=-v:refname | head -1)
        if [[ -n "$highest" ]]; then
            n=$(echo "$highest" | sed 's/.*-beta\.//')
            next=$((n + 1))
        else
            next=1
        fi
        tag="v${base}-beta.${next}"
        echo "Cutting $tag on alpha..."
        git -C "$ALPHA_TREE" tag "$tag"
        git -C "$ALPHA_TREE" push origin "$tag"
        echo "Published $tag for source-build updates."
    fi

    if [[ "$do_install" == "install" || "$do_install" == "true" ]]; then
        echo ""
        echo "Installing beta (v$version)..."
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
check_clean "$MAIN_TREE" "main worktree"
[[ $(git -C "$MAIN_TREE" rev-parse --abbrev-ref HEAD) == "main" ]] || die "main worktree is not on 'main' branch"

# Stable releases must ship a skill that documents this binary version.
bash "$REPO_ROOT/scripts/check-skill-version.sh" "$BETA_TREE" \
    || die "published agent skill is out of lockstep with the release version"

version=$(grep '^version' "$BETA_TREE/Cargo.toml" | head -1 | sed 's/version = "\(.*\)"/\1/')

echo "Promoting beta → main (v$version)..."
git push origin beta:main

echo "Syncing main worktree..."
git -C "$MAIN_TREE" pull origin main

if [[ "$do_release" == "release" || "$do_release" == "true" ]]; then
    main_commit=$(git -C "$MAIN_TREE" rev-parse HEAD)
    if git -C "$MAIN_TREE" tag -l "v$version" | grep -q "v$version"; then
        tagged_commit=$(git -C "$MAIN_TREE" rev-list -n 1 "v$version")
        if [[ "$tagged_commit" != "$main_commit" ]]; then
            echo "info: tag v$version points at an older commit — re-tagging at main HEAD..."
            git -C "$MAIN_TREE" tag -f "v$version" "$main_commit"
        fi
    else
        echo "Creating tag v$version at main HEAD..."
        git -C "$MAIN_TREE" tag "v$version"
    fi
    echo "Publishing tag v$version for source-build updates..."
    git -C "$MAIN_TREE" push origin "v$version" --force
else
    echo "Code promoted to main. Run 'just promote main release' to publish its source-build tag."
fi

if [[ "$do_install" == "install" || "$do_install" == "true" ]]; then
    echo ""
    echo "Installing main (v$version)..."
    (cd "$MAIN_TREE" && just install)
fi

echo ""
echo "v$version is on main."
