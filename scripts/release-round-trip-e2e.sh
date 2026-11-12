#!/usr/bin/env bash
# Non-publishing acceptance for one bump-to-stable release round trip.
#
# Runs the REAL scripts/release-version.sh, promote.sh and release-tag.sh
# (copied byte-for-byte) inside a throwaway repository whose `origin` is a
# bare repository in a temp directory, then feeds the tags they cut to the
# updater's channel policy (crates/distribution/tests/release_round_trip.rs).
#
# Isolation is by construction and asserted before any release script runs:
#   - origin is a local temp path (no network transport can resolve it),
#   - HOME, XDG_*, git config and the cargo target dir are temp,
#   - `gh` on PATH is a shim that fails, so a GitHub Release cannot be made,
#   - nothing here reads or writes a ~/.plexi* profile or an installed binary.
# The real publication step (the tag-triggered release workflow and
# publish-release.sh) is outside this test by design.
#
# macOS only: release-version.sh uses BSD `sed -i ''`.
# Needs: git, git-cliff, uv, cargo.
# Usage: bash scripts/release-round-trip-e2e.sh
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
[[ "$(uname)" == "Darwin" ]] || { echo "error: release-round-trip-e2e.sh needs macOS (BSD sed -i '' in release-version.sh)" >&2; exit 1; }
for tool in git git-cliff uv cargo; do
    command -v "$tool" >/dev/null || { echo "error: $tool not found on PATH (required by the release scripts)" >&2; exit 1; }
done

REAL_TARGET="${CARGO_TARGET_DIR:-$REPO/target}"
# Cargo and rustup must keep finding the real toolchain once HOME is swapped.
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
UV_BIN="$(command -v uv)"; GIT_CLIFF_BIN="$(command -v git-cliff)"; CARGO_BIN_DIR="$(dirname "$(command -v cargo)")"

T="$(cd "$(mktemp -d "${TMPDIR:-/tmp}/plexi-release-round-trip.XXXXXX")" && pwd -P)"
trap 'rm -rf "$T"' EXIT
mkdir -p "$T/home" "$T/bin"
cat > "$T/bin/gh" <<'SH'
#!/usr/bin/env bash
echo "FORBIDDEN: the release round trip must never call gh ($*)" >&2
exit 97
SH
chmod +x "$T/bin/gh"
for tool in "$UV_BIN" "$GIT_CLIFF_BIN"; do ln -s "$tool" "$T/bin/$(basename "$tool")"; done

export HOME="$T/home" XDG_CONFIG_HOME="$T/home/.config" XDG_CACHE_HOME="$T/home/.cache"
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null GIT_TERMINAL_PROMPT=0
export GIT_AUTHOR_NAME=fixture GIT_AUTHOR_EMAIL=fixture@example.invalid
export GIT_COMMITTER_NAME=fixture GIT_COMMITTER_EMAIL=fixture@example.invalid
export CARGO_TARGET_DIR="$T/target" PLEXI_CARGO_LOCK="$T/cargo.lock"
export UV_CACHE_DIR="$T/uv-cache"
unset PLEXI_CHANNEL PLEXI_SOCKET PLEXI_PANE_ID
export PATH="$T/bin:$CARGO_BIN_DIR:/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin"

ORIGIN="$T/origin.git"
ALPHA="$T/work/repo"
BETA="$ALPHA/worktrees/beta"
MAIN="$ALPHA/worktrees/main"

step() { echo; echo "== $*"; }
die() { echo "FAIL: $*" >&2; exit 1; }
eq() { [[ "$1" == "$2" ]] || die "$3: expected '$2', got '$1'"; }
contains() { grep -qF -- "$2" <<<"$1" || die "$3: missing '$2'"; }
origin_ref() { git --git-dir="$ORIGIN" rev-parse -q --verify "$1" 2>/dev/null || echo "(absent)"; }
origin_tags() { git --git-dir="$ORIGIN" tag -l | sort | tr '\n' ' '; }
# Run a release script expecting a refusal: it must fail and say why.
refuses() {
    local why="$1"; shift
    local out
    if out=$("$@" 2>&1 </dev/null); then die "expected refusal ($why) but succeeded: $*"; fi
    contains "$out" "$why" "refusal message for: $*"
}

# ── build the fixture repository ─────────────────────────────────────────────

step "fixture: bare origin + alpha/beta/main trees under $T"
git init -q --bare -b alpha "$ORIGIN"
mkdir -p "$T/work" && git init -q -b alpha "$ALPHA"
cd "$ALPHA"
mkdir -p src gen_cli_docs/src sdk/python skills/plexi-cli website/src/content/docs scripts
cat > Cargo.toml <<'TOML'
[package]
name = "plexi-fixture"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "plexi"
path = "src/main.rs"

[workspace]
members = ["gen_cli_docs"]
TOML
cat > src/main.rs <<'RS'
fn main() {}

#[cfg(test)]
mod tests {
    #[test]
    fn skill_surface_fixture() {}
}
RS
cat > gen_cli_docs/Cargo.toml <<'TOML'
[package]
name = "gen_cli_docs"
version = "0.0.0"
edition = "2021"
TOML
echo 'fn main() { println!("# CLI\n"); }' > gen_cli_docs/src/main.rs
cat > sdk/python/pyproject.toml <<'TOML'
[project]
name = "plexi-fixture-sdk"
version = "0.1.0"
requires-python = ">=3.11"
dependencies = []
TOML
printf -- '---\nplexi_version: "0.1.0"\n---\n' > skills/plexi-cli/SKILL.md
echo "# CLI" > website/src/content/docs/cli.md
printf '# Changelog\n\nNewest releases appear first.\n' > CHANGELOG.md
printf 'check-cli-docs:\n    @true\n\ngen-cli-docs:\n    @true\n' > justfile
printf 'target/\nworktrees/\n' > .gitignore
cp "$REPO/cliff.toml" cliff.toml
for s in release-version.sh promote.sh release-tag.sh gen-doc-safe.sh cargo-with-lease.sh; do
    cp "$REPO/scripts/$s" "scripts/$s"
done
git add -A && git commit -q -m "chore: seed fixture"
git tag v0.1.0
git remote add origin "$ORIGIN"
git push -q origin alpha alpha:beta alpha:main v0.1.0
git fetch -q origin
git worktree add -q -b beta worktrees/beta origin/beta
git worktree add -q -b main worktrees/main origin/main
git branch -q --set-upstream-to=origin/alpha alpha
git commit -q --allow-empty -m "feat: add the round trip feature"
git commit -q --allow-empty -m "fix: repair the round trip fix"
git push -q origin alpha
SEED_TAGS="$(origin_tags)"

# ── isolation proof (before any release script runs) ─────────────────────────

step "isolation: origin must be the local temp bare repo"
for tree in "$ALPHA" "$BETA" "$MAIN"; do
    url="$(git -C "$tree" remote get-url origin)"
    eq "$url" "$ORIGIN" "origin of $tree"
    [[ "$url" == "$T"/* && "$url" != *://* && "$url" != *@*:* ]] || die "origin is not a local temp path: $url"
    [[ -z "$(git -C "$tree" remote get-url --push --all origin | grep -v -x -F "$ORIGIN" || true)" ]] || die "push URL escapes the temp origin"
    eq "$(git -C "$tree" remote | tr '\n' ' ')" "origin " "remotes of $tree"
done
eq "$(git -C "$ALPHA" rev-parse --git-common-dir)" ".git" "alpha common dir"
[[ "$(cd "$BETA" && git rev-parse --git-common-dir)" == "$ALPHA/.git" ]] || die "beta tree is not a worktree of the temp repo"
[[ "$ALPHA" != "$REPO"* ]] || die "fixture lives inside the real repo"
if gh --version >/dev/null 2>&1; then die "gh shim not first on PATH"; fi
echo "origin=$ORIGIN (local path); gh shimmed; HOME=$HOME"

# ── release and promotion stay separate ──────────────────────────────────────

step "bump on alpha: version, SDK, skill stamp, changelog, local release tag"
cd "$ALPHA"
bash scripts/release-version.sh patch
eq "$(grep '^version' Cargo.toml | head -1)" 'version = "0.1.1"' "Cargo.toml version"
contains "$(grep '^version' sdk/python/pyproject.toml)" '0.1.1' "SDK version"
contains "$(cat skills/plexi-cli/SKILL.md)" 'plexi_version: "0.1.1"' "skill stamp"
changelog="$(cat CHANGELOG.md)"
contains "$changelog" "## [0.1.1]" "changelog heading"
contains "$changelog" "feat: add the round trip feature" "changelog feat entry"
contains "$changelog" "fix: repair the round trip fix" "changelog fix entry"
[[ "$changelog" != *"seed fixture"* ]] || die "changelog leaked a commit from before v0.1.0"
eq "$(git log -1 --format=%s)" "chore: release v0.1.1" "release commit subject"
RELEASE_COMMIT="$(git rev-parse HEAD)"
eq "$(git rev-parse v0.1.1)" "$RELEASE_COMMIT" "v0.1.1 pins the release commit"
eq "$(origin_ref refs/tags/v0.1.1)" "(absent)" "bump must not publish its tag"
refuses "no commits since v0.1.1" bash scripts/release-version.sh patch
refuses "unknown arg" bash scripts/release-version.sh sideways
git push -q origin alpha
eq "$(origin_tags)" "$SEED_TAGS" "bump + push of alpha added tags to origin"

step "alpha prerelease tag pins the same base without bumping"
bash scripts/release-version.sh --alpha
ALPHA_TAG="v0.1.1-alpha.1"
eq "$(git rev-parse "$ALPHA_TAG")" "$RELEASE_COMMIT" "alpha tag target"
eq "$(grep '^version' Cargo.toml | head -1)" 'version = "0.1.1"' "alpha cut must not bump Cargo.toml"
git push -q origin "$ALPHA_TAG"

step "release before promote cannot publish the new version"
before_heads="$(origin_ref refs/heads/alpha) $(origin_ref refs/heads/beta) $(origin_ref refs/heads/main)"
refuses "usage: release-tag.sh" bash scripts/release-tag.sh alpha
refuses "usage: release-tag.sh" bash scripts/release-tag.sh ""
bash scripts/release-tag.sh main >/dev/null   # main tree still holds v0.1.0
eq "$(origin_ref refs/tags/v0.1.1)" "(absent)" "release main before promote published v0.1.1"
eq "$(origin_ref refs/tags/v0.1.1-beta.1)" "(absent)" "stray beta tag"
eq "$(origin_ref refs/heads/main)" "$(git rev-parse origin/main)" "release moved main"
eq "$before_heads" "$(origin_ref refs/heads/alpha) $(origin_ref refs/heads/beta) $(origin_ref refs/heads/main)" "release moved a branch"

step "promote alpha -> beta: code only, no tags, main untouched"
tags_before="$(origin_tags)"
bash scripts/promote.sh beta >/dev/null
eq "$(origin_ref refs/heads/beta)" "$RELEASE_COMMIT" "origin/beta after promote"
eq "$(origin_ref refs/heads/main)" "$(git -C "$MAIN" rev-parse HEAD)" "promote beta moved main"
eq "$(git -C "$BETA" rev-parse HEAD)" "$RELEASE_COMMIT" "beta worktree synced"
eq "$(origin_tags)" "$tags_before" "promote created or pushed a tag"
refuses "unknown target" bash scripts/promote.sh sideways

step "release guards: dirty, drifted, and wrong-branch trees are refused"
touch "$BETA/stray" && git -C "$BETA" add stray
refuses "uncommitted changes" bash scripts/release-tag.sh beta
git -C "$BETA" reset -q --hard
git -C "$BETA" commit -q --allow-empty -m "feat: local only"
refuses "does not match origin/beta" bash scripts/release-tag.sh beta
git -C "$BETA" reset -q --hard "$RELEASE_COMMIT"
git -C "$MAIN" switch -q -c detour
refuses "is not on 'main'" bash scripts/release-tag.sh main
git -C "$MAIN" switch -q main && git -C "$MAIN" branch -q -D detour
eq "$(origin_tags)" "$tags_before" "a refused release still pushed a tag"

step "release beta: vX.Y.Z-beta.N at beta HEAD, branches untouched"
heads="$(origin_ref refs/heads/alpha) $(origin_ref refs/heads/beta) $(origin_ref refs/heads/main)"
bash scripts/release-tag.sh beta >/dev/null
BETA_TAG="v0.1.1-beta.1"
eq "$(origin_ref "refs/tags/$BETA_TAG^{commit}")" "$RELEASE_COMMIT" "beta tag target on origin"
eq "$heads" "$(origin_ref refs/heads/alpha) $(origin_ref refs/heads/beta) $(origin_ref refs/heads/main)" "release beta moved a branch"
eq "$(origin_ref refs/tags/v0.1.1)" "(absent)" "release beta published the stable tag"

step "promote beta -> main (from the beta tree): code only, skill-surface gate runs"
tags_before="$(origin_tags)"
(cd "$BETA" && bash scripts/promote.sh main >/dev/null)
eq "$(origin_ref refs/heads/main)" "$RELEASE_COMMIT" "origin/main after promote"
eq "$(origin_tags)" "$tags_before" "promote main created or pushed a tag"
eq "$(origin_ref refs/tags/v0.1.1)" "(absent)" "promote main published the stable tag"

step "release main: stable tag vX.Y.Z pins the promoted commit"
bash scripts/release-tag.sh main >/dev/null
STABLE_TAG="v0.1.1"
eq "$(origin_ref "refs/tags/$STABLE_TAG^{commit}")" "$RELEASE_COMMIT" "stable tag target on origin"
for b in alpha beta main; do eq "$(origin_ref "refs/heads/$b")" "$RELEASE_COMMIT" "origin/$b at release commit"; done
bash scripts/release-tag.sh main >/dev/null   # idempotent re-run
eq "$(origin_ref "refs/tags/$STABLE_TAG^{commit}")" "$RELEASE_COMMIT" "stable tag after idempotent re-release"
git tag -d "$STABLE_TAG" >/dev/null && git -C "$MAIN" tag "$STABLE_TAG" "$(git -C "$MAIN" rev-parse HEAD~1)"
refuses "already identifies another commit" bash scripts/release-tag.sh main
git -C "$MAIN" tag -d "$STABLE_TAG" >/dev/null && git fetch -q origin "refs/tags/$STABLE_TAG:refs/tags/$STABLE_TAG"

step "tag set and ordering on origin"
expected="v0.1.0 $ALPHA_TAG $BETA_TAG $STABLE_TAG "
eq "$(origin_tags)" "$(printf '%s\n' $expected | sort | tr '\n' ' ')" "origin tags"
# versionsort.suffix makes git order a prerelease before its final release.
ordered="$(git -c versionsort.suffix=-alpha -c versionsort.suffix=-beta --git-dir="$ORIGIN" tag -l 'v0.1.1*' --sort=v:refname | tr '\n' ' ')"
eq "$ordered" "$ALPHA_TAG $BETA_TAG $STABLE_TAG " "semver order alpha < beta < stable"
[[ -z "$(ls -A "$HOME" | grep -i plexi || true)" ]] || die "a ~/.plexi* profile was created under the temp HOME"

# ── updater / channel acceptance on the tags just cut ────────────────────────

step "updater: channel acceptance of the cut tags (plexi-distribution policy)"
cd "$REPO"
PLEXI_ROUND_TRIP_ALPHA_TAG="$ALPHA_TAG" PLEXI_ROUND_TRIP_BETA_TAG="$BETA_TAG" PLEXI_ROUND_TRIP_STABLE_TAG="$STABLE_TAG" \
    CARGO_TARGET_DIR="$REAL_TARGET" \
    cargo test --quiet -p plexi-distribution --test release_round_trip -- --ignored

echo
echo "release round trip OK: $ALPHA_TAG -> $BETA_TAG -> $STABLE_TAG (isolated; nothing published)"
