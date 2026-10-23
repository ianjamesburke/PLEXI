#!/usr/bin/env bash
# pr-install-seed-profile.sh — seed an isolated PR profile from alpha.
#
# Usage:
#   pr-install-seed-profile.sh seed <pr-number>
#   pr-install-seed-profile.sh self-test
#
# Agent and team definitions are useful test fixtures, but a PR profile must
# remain isolated. Copy only files alpha already has and never replace a file
# which was created or customized in the PR profile.
set -euo pipefail

seed() {
    local number="$1"
    local alpha_profile="$HOME/.plexi-alpha"
    local pr_profile="$HOME/.plexi-pr-$number"
    local seeded=()
    local kind

    if [ ! -d "$alpha_profile" ]; then
        echo "pr-install: note: alpha profile is missing; skipped agent/team seed"
        return
    fi

    for kind in agents teams; do
        if [ -d "$alpha_profile/$kind" ]; then
            mkdir -p "$pr_profile/$kind"
            cp -R -n "$alpha_profile/$kind/." "$pr_profile/$kind/"
            seeded+=("$kind")
        fi
    done

    if [ "${#seeded[@]}" -eq 0 ]; then
        echo "pr-install: note: alpha profile has no agents/ or teams/ to seed"
    else
        echo "pr-install: seeded PR profile from alpha (no-clobber): ${seeded[*]}"
    fi
}

self_test() {
    local tmp home out fails=0
    tmp=$(mktemp -d)
    home="$tmp/home"
    mkdir -p "$home/.plexi-alpha/agents/nested" "$home/.plexi-alpha/teams" \
        "$home/.plexi-pr-9/agents" "$home/.plexi-pr-9/teams"
    printf 'alpha researcher\n' > "$home/.plexi-alpha/agents/researcher.toml"
    printf 'alpha nested\n' > "$home/.plexi-alpha/agents/nested/worker.toml"
    printf 'alpha team\n' > "$home/.plexi-alpha/teams/research.toml"
    printf 'PR researcher\n' > "$home/.plexi-pr-9/agents/researcher.toml"
    printf 'PR team\n' > "$home/.plexi-pr-9/teams/custom.toml"

    out=$(HOME="$home" seed 9)
    case "$out" in
        *"seeded PR profile from alpha (no-clobber): agents teams"*) ;;
        *) echo "FAIL seed reports copied kinds: $out" >&2; fails=$((fails + 1)) ;;
    esac
    [ "$(<"$home/.plexi-pr-9/agents/researcher.toml")" = "PR researcher" ] \
        || { echo "FAIL existing PR agent was overwritten" >&2; fails=$((fails + 1)); }
    [ "$(<"$home/.plexi-pr-9/agents/nested/worker.toml")" = "alpha nested" ] \
        || { echo "FAIL nested alpha agent was not copied" >&2; fails=$((fails + 1)); }
    [ "$(<"$home/.plexi-pr-9/teams/research.toml")" = "alpha team" ] \
        || { echo "FAIL alpha team was not copied" >&2; fails=$((fails + 1)); }
    [ "$(<"$home/.plexi-pr-9/teams/custom.toml")" = "PR team" ] \
        || { echo "FAIL existing PR team was overwritten" >&2; fails=$((fails + 1)); }
    [ "$(<"$home/.plexi-alpha/agents/researcher.toml")" = "alpha researcher" ] \
        || { echo "FAIL alpha profile was modified" >&2; fails=$((fails + 1)); }

    out=$(HOME="$tmp/missing-home" seed 10)
    case "$out" in
        *"alpha profile is missing; skipped agent/team seed"*) ;;
        *) echo "FAIL missing alpha profile is not reported: $out" >&2; fails=$((fails + 1)) ;;
    esac
    [ ! -e "$tmp/missing-home/.plexi-pr-10" ] \
        || { echo "FAIL missing alpha profile created a PR profile" >&2; fails=$((fails + 1)); }

    if [ "$fails" -ne 0 ]; then
        echo "pr-install-seed-profile self-test: $fails failure(s)" >&2
        return 1
    fi
    echo "pr-install-seed-profile self-test passed"
}

case "${1:-}" in
    seed) shift; seed "${1:?pr number}" ;;
    self-test) self_test ;;
    *) echo "usage: $0 {seed|self-test} ..." >&2; exit 2 ;;
esac
