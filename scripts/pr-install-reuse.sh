#!/usr/bin/env bash
# pr-install-reuse.sh — decide whether `just pr-install` may skip the release build.
#
# Usage:
#   pr-install-reuse.sh check  <worktree> <head_oid> <channel> [binary]
#       stdout `reuse <head_oid>` + exit 0, or `build <reason>` + exit 1.
#   pr-install-reuse.sh stamp  <worktree> <head_oid> <channel> [binary]
#       record that <binary> was built for <channel> at <head_oid> (run after a
#       successful install).
#   pr-install-reuse.sh self-test
#
# A binary is reused only when ALL hold (never mtime, never branch name):
#   1. the worktree is clean (tracked and untracked) and HEAD == <head_oid>, so
#      the build identity is fully determined by the commit;
#   2. `<binary> --build-info` reports source_commit == <head_oid> (the sha the
#      build embeds via build.rs);
#   3. a stamp written by a previous install records the same channel, head, and
#      the binary's sha256 — the binary carries a compiled-in PR-channel marker
#      (PLEXI_BUILD_TEST_CHANNEL) that --build-info does not expose, so a stamp
#      is the only proof it was built for this channel and not by a plain
#      `just build`.
# Any missing or mismatching piece means: build as before.
set -euo pipefail

sha256_of() { shasum -a 256 "$1" | awk '{print $1}'; }

default_binary() {
    local wt="$1" target
    target=$(cd "$wt" && cargo metadata --format-version=1 --no-deps 2>/dev/null \
        | python3 -c 'import sys,json; print(json.load(sys.stdin)["target_directory"])') || return 1
    printf '%s/release/plexi\n' "$target"
}

resolve_binary() {
    local wt="$1" binary="${2:-}"
    if [ -z "$binary" ]; then
        binary=$(default_binary "$wt") || return 1
    fi
    printf '%s\n' "$binary"
}

do_check() {
    local wt="$1" head="$2" channel="$3" binary
    binary=$(resolve_binary "$wt" "${4:-}") || { echo "build cannot resolve target dir"; return 1; }
    [ -x "$binary" ] || { echo "build no release binary at $binary"; return 1; }

    git -C "$wt" update-index -q --refresh 2>/dev/null || true
    [ "$(git -C "$wt" rev-parse HEAD 2>/dev/null)" = "$head" ] \
        || { echo "build worktree HEAD is not the PR head"; return 1; }
    [ -z "$(git -C "$wt" status --porcelain 2>/dev/null)" ] \
        || { echo "build worktree is dirty"; return 1; }

    local info embedded
    info=$("$binary" --build-info 2>/dev/null) || { echo "build binary --build-info failed"; return 1; }
    embedded=$(printf '%s' "$info" | python3 -c 'import sys,json; print(json.load(sys.stdin).get("source_commit",""))' 2>/dev/null) \
        || { echo "build binary stamp unreadable"; return 1; }
    [ -n "$embedded" ] || { echo "build binary has no embedded sha"; return 1; }
    [ "$embedded" = "$head" ] \
        || { echo "build binary embeds ${embedded:0:8}, PR head is ${head:0:8}"; return 1; }

    local stamp
    stamp="$(dirname "$binary")/.pr-install-stamp"
    [ -f "$stamp" ] || { echo "build no pr-install stamp for binary"; return 1; }
    grep -Fqx "channel=$channel" "$stamp" || { echo "build stamp is for another channel"; return 1; }
    grep -Fqx "head=$head" "$stamp" || { echo "build stamp is for another head"; return 1; }
    grep -Fqx "sha256=$(sha256_of "$binary")" "$stamp" \
        || { echo "build binary changed since it was stamped"; return 1; }

    echo "reuse $head"
}

do_stamp() {
    local wt="$1" head="$2" channel="$3" binary
    binary=$(resolve_binary "$wt" "${4:-}") || { echo "ERROR: cannot resolve target dir" >&2; return 1; }
    [ -x "$binary" ] || { echo "ERROR: no release binary at $binary" >&2; return 1; }
    printf 'channel=%s\nhead=%s\nsha256=%s\n' "$channel" "$head" "$(sha256_of "$binary")" \
        > "$(dirname "$binary")/.pr-install-stamp"
}

# --- self-test -------------------------------------------------------------
# Fake worktree + fake binary: a stale (sha-mismatched) binary must NOT be reused.
fake_binary() { # path sha
    cat > "$1" <<EOF
#!/usr/bin/env bash
echo '{"source_commit":"$2","build_id":"$2-x"}'
EOF
    chmod +x "$1"
}

self_test() {
    local TMP wt head other bin fails=0
    TMP=$(mktemp -d)
    wt="$TMP/wt"
    mkdir -p "$wt"
    git -C "$wt" init -q
    git -C "$wt" -c user.email=t@t -c user.name=t commit -q --allow-empty -m one
    head=$(git -C "$wt" rev-parse HEAD)
    other=$(printf '%040d' 7)
    bin="$TMP/plexi"

    expect() { # label want(reuse|build) args...
        local label="$1" want="$2" out
        shift 2
        out=$(do_check "$@" 2>&1) || true
        if [ "${out%% *}" != "$want" ]; then
            echo "FAIL $label: wanted '$want', got '$out'" >&2
            fails=$((fails + 1))
        else
            echo "ok   $label ($out)"
        fi
    }

    fake_binary "$bin" "$head"
    expect "no stamp is not reused" build "$wt" "$head" pr-1 "$bin"
    do_stamp "$wt" "$head" pr-1 "$bin"
    expect "stamped matching binary is reused" reuse "$wt" "$head" pr-1 "$bin"
    expect "other channel is not reused" build "$wt" "$head" pr-2 "$bin"

    fake_binary "$bin" "$other"
    expect "sha-mismatched stale binary is NOT reused" build "$wt" "$head" pr-1 "$bin"
    do_stamp "$wt" "$head" pr-1 "$bin"
    expect "stale binary stays rejected even when re-stamped" build "$wt" "$head" pr-1 "$bin"

    fake_binary "$bin" "$head"
    do_stamp "$wt" "$head" pr-1 "$bin"
    echo "# tampered" >> "$bin"
    expect "binary changed after stamp is not reused" build "$wt" "$head" pr-1 "$bin"

    fake_binary "$bin" "$head"
    do_stamp "$wt" "$head" pr-1 "$bin"
    : > "$wt/untracked.txt"
    expect "dirty worktree is not reused" build "$wt" "$head" pr-1 "$bin"
    # shellcheck disable=SC2115
    mv "$wt/untracked.txt" "$TMP/untracked.txt"
    expect "clean again is reused" reuse "$wt" "$head" pr-1 "$bin"

    expect "worktree not at PR head is not reused" build "$wt" "$other" pr-1 "$bin"
    expect "missing binary is not reused" build "$wt" "$head" pr-1 "$TMP/absent"

    if [ "$fails" -ne 0 ]; then
        echo "pr-install-reuse self-test: $fails failure(s)" >&2
        return 1
    fi
    echo "pr-install-reuse self-test passed"
}

case "${1:-}" in
    check) shift; do_check "${1:?worktree}" "${2:?head}" "${3:?channel}" "${4:-}" ;;
    stamp) shift; do_stamp "${1:?worktree}" "${2:?head}" "${3:?channel}" "${4:-}" ;;
    self-test) self_test ;;
    *) echo "usage: $0 {check|stamp|self-test} ..." >&2; exit 2 ;;
esac
