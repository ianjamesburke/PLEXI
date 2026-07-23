#!/usr/bin/env bash
# Usage: scripts/channel-clean-ephemeral.sh [--dry-run] [--age-days N]
# Reaps ephemeral ~/.plexi-* profiles: 16-hex-hash channels (minted by external
# drive-host tooling via PLEXI_CHANNEL=<hex>) and ad-hoc test leftovers
# (baseline-*, stint*, *test*). Only dirs untouched for N days (default 7) are
# reaped, via channel-clean.sh so bins/bundles/completions go too.
# Never touches the real channels: main (~/.plexi), alpha, beta, pr-*, rc-*, src.
# Periodic-run candidate: a launchd job in ~/dotfiles/launchd (not created here).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

dry_run=0
age_days=7
while [[ $# -gt 0 ]]; do
    case "$1" in
        --dry-run) dry_run=1; shift ;;
        --age-days) age_days="${2:?--age-days needs a value}"; shift 2 ;;
        *) echo "error: unknown argument '$1' (expected --dry-run or --age-days N)"; exit 1 ;;
    esac
done
[[ "$age_days" =~ ^[0-9]+$ ]] || { echo "error: --age-days must be a number, got '$age_days'"; exit 1; }

now=$(date +%s)
min_age_secs=$(( age_days * 86400 ))
total_kb=0
reaped=0

for profile in "$HOME"/.plexi-*/; do
    [[ -d "$profile" ]] || continue
    channel=$(basename "$profile" | sed 's/^\.plexi-//')

    # Hard protect list: real channels are never ephemeral, whatever else matches.
    case "$channel" in
        alpha|beta|main|src|pr-*|rc-*) continue ;;
    esac

    # Reap only known-ephemeral shapes: 16-hex hashes and ad-hoc test names.
    if [[ ! "$channel" =~ ^[0-9a-f]{16}$ ]]; then
        case "$channel" in
            baseline-*|stint*|*test*) ;;
            *) continue ;;
        esac
    fi

    # Age by the newest mtime in the profile's top two levels, not the dir
    # itself: writing plexi.log does not bump the directory mtime, and an
    # active channel must never look idle.
    mtime=$(find "$profile" -maxdepth 2 -exec stat -f %m {} + 2>/dev/null | sort -rn | head -1)
    [[ -n "$mtime" ]] || mtime=$(stat -f %m "$profile")
    age_secs=$(( now - mtime ))
    if (( age_secs < min_age_secs )); then
        echo "Skipping $channel (modified $(( age_secs / 86400 ))d ago, threshold ${age_days}d)"
        continue
    fi

    kb=$(du -sk "$profile" | cut -f1)
    total_kb=$(( total_kb + kb ))
    reaped=$(( reaped + 1 ))
    if (( dry_run )); then
        echo "Would reap $channel ($(( kb / 1024 )) MB, idle $(( age_secs / 86400 ))d)"
    else
        echo "Reaping $channel ($(( kb / 1024 )) MB, idle $(( age_secs / 86400 ))d)"
        "$SCRIPT_DIR/channel-clean.sh" "$channel"
    fi
done

if (( reaped == 0 )); then
    echo "Nothing to reap"
elif (( dry_run )); then
    echo "Dry run: $reaped profile(s), $(( total_kb / 1024 )) MB reclaimable"
else
    echo "Reaped $reaped profile(s), reclaimed $(( total_kb / 1024 )) MB"
fi
