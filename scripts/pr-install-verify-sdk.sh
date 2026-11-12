#!/usr/bin/env bash
# pr-install-verify-sdk.sh — prove a PR install carries the PR head's Python SDK.
#
# Usage:
#   pr-install-verify-sdk.sh verify <pr-number> <worktree> <head_oid>
#       exit 0 and print one provenance line on stdout; exit 1 with a named
#       error (`SDK_MISMATCH`) on stderr when an installed SDK differs from
#       <worktree>/sdk/python/plexi_sdk.
#   pr-install-verify-sdk.sh self-test
#
# Checked copies (the profile SDK is only extracted on first host launch, keyed
# by the build id, so it is checked only when its stamp already names this head):
#   bundle   ~/Applications/Plexi PR<N>.app/Contents/Resources/sdk/plexi_sdk  (macOS)
#   profile  ~/.plexi-pr-<N>/sdk/plexi_sdk
# Byte-identical means same file set and same content, ignoring the build-time
# ignore list of scripts/package.py (__pycache__, *.pyc, .venv, .DS_Store).
set -euo pipefail

tree_digest() {
    local dir="$1"
    (
        cd "$dir"
        find . -type f ! -path '*/__pycache__/*' ! -name '*.pyc' ! -path '*/.venv/*' ! -name .DS_Store -print0 \
            | LC_ALL=C sort -z \
            | xargs -0 shasum -a 256
    ) | shasum -a 256 | awk '{print $1}'
}

do_verify() {
    local number="$1" wt="$2" head="$3"
    local src="$wt/sdk/python/plexi_sdk"
    local bundle="$HOME/Applications/Plexi PR${number}.app/Contents/Resources/sdk/plexi_sdk"
    local profile="$HOME/.plexi-pr-${number}/sdk"
    [ -d "$src" ] || { echo "SDK_MISMATCH: source tree has no sdk/python/plexi_sdk: $src" >&2; return 1; }
    local want bundle_state profile_state
    want=$(tree_digest "$src")

    if [ -d "$bundle" ]; then
        if [ "$(tree_digest "$bundle")" = "$want" ]; then
            bundle_state=match
        else
            echo "SDK_MISMATCH: installed bundle SDK differs from $src (head ${head:0:8}): $bundle" >&2
            return 1
        fi
    else
        bundle_state=absent
    fi

    if [ -d "$profile/plexi_sdk" ] && [ -f "$profile/.sdk_version" ] \
        && [[ "$(tr -d '[:space:]' < "$profile/.sdk_version")" == "$head"* ]]; then
        if [ "$(tree_digest "$profile/plexi_sdk")" = "$want" ]; then
            profile_state=match
        else
            echo "SDK_MISMATCH: profile SDK is stamped for ${head:0:8} but differs from $src: $profile/plexi_sdk" >&2
            return 1
        fi
    else
        profile_state=pending-first-launch
    fi

    if [ "$bundle_state" = absent ] && [ "$profile_state" != match ]; then
        if [ "${PLEXI_VERIFY_SDK_OS:-$(uname -s)}" = Darwin ]; then
            echo "SDK_MISMATCH: no installed SDK could be checked (bundle absent, profile not extracted for this head)" >&2
            return 1
        fi
        # Non-macOS bundle layout is not modelled here; say so instead of guessing.
        bundle_state=unchecked
    fi

    printf 'sdk_from=%s sdk_head=%s sdk_digest=%s bundle=%s profile=%s\n' \
        "$src" "${head:0:8}" "${want:0:12}" "$bundle_state" "$profile_state"
}

self_test() {
    local TMP fails=0 head out
    TMP=$(mktemp -d)
    head=$(printf '%040d' 3)
    mkdir -p "$TMP/wt/sdk/python/plexi_sdk" "$TMP/home/Applications/Plexi PR9.app/Contents/Resources/sdk/plexi_sdk"
    echo "v1" > "$TMP/wt/sdk/python/plexi_sdk/mod.py"
    echo "v1" > "$TMP/home/Applications/Plexi PR9.app/Contents/Resources/sdk/plexi_sdk/mod.py"
    export HOME="$TMP/home"

    expect() { # label want(ok|fail)
        local label="$1" want="$2" rc=0
        out=$(do_verify 9 "$TMP/wt" "$head" 2>&1) || rc=$?
        if { [ "$want" = ok ] && [ "$rc" -ne 0 ]; } || { [ "$want" = fail ] && [ "$rc" -eq 0 ]; }; then
            echo "FAIL $label: $out" >&2
            fails=$((fails + 1))
        else
            echo "ok   $label"
        fi
    }

    expect "matching bundle passes" ok
    # SDK-only change at the PR head, bundle still pre-PR: must fail, not pass silently.
    echo "v2" > "$TMP/wt/sdk/python/plexi_sdk/mod.py"
    expect "SDK-only change missing from bundle fails" fail
    case "$out" in *SDK_MISMATCH*) ;; *) echo "FAIL error is not named SDK_MISMATCH: $out" >&2; fails=$((fails + 1)) ;; esac
    echo "v2" > "$HOME/Applications/Plexi PR9.app/Contents/Resources/sdk/plexi_sdk/mod.py"
    expect "SDK-only change delivered to bundle passes" ok
    echo "extra" > "$HOME/Applications/Plexi PR9.app/Contents/Resources/sdk/plexi_sdk/extra.py"
    expect "extra file in bundle fails" fail
    mv "$HOME/Applications/Plexi PR9.app/Contents/Resources/sdk/plexi_sdk/extra.py" "$TMP/extra.py"

    mkdir -p "$HOME/.plexi-pr-9/sdk/plexi_sdk"
    echo "old" > "$HOME/.plexi-pr-9/sdk/plexi_sdk/mod.py"
    echo "old-build-id" > "$HOME/.plexi-pr-9/sdk/.sdk_version"
    expect "stale profile not yet re-extracted is pending, not an error" ok
    echo "${head}-abc" > "$HOME/.plexi-pr-9/sdk/.sdk_version"
    expect "profile stamped for this head with stale SDK fails" fail
    echo "v2" > "$HOME/.plexi-pr-9/sdk/plexi_sdk/mod.py"
    expect "profile stamped for this head with matching SDK passes" ok

    mv "$HOME/Applications/Plexi PR9.app" "$TMP/moved.app"
    echo "old-build-id" > "$HOME/.plexi-pr-9/sdk/.sdk_version"
    PLEXI_VERIFY_SDK_OS=Darwin expect "macOS with no checkable SDK fails" fail
    PLEXI_VERIFY_SDK_OS=Linux expect "non-macOS with no checkable SDK is reported unchecked" ok

    if [ "$fails" -ne 0 ]; then
        echo "pr-install-verify-sdk self-test: $fails failure(s)" >&2
        return 1
    fi
    echo "pr-install-verify-sdk self-test passed"
}

case "${1:-}" in
    verify) shift; do_verify "${1:?pr number}" "${2:?worktree}" "${3:?head}" ;;
    self-test) self_test ;;
    *) echo "usage: $0 {verify|self-test} ..." >&2; exit 2 ;;
esac
