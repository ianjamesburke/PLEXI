#!/usr/bin/env bash
set -uo pipefail

scene_file="$1"
channel="$2"
out_dir="$3"
owner_file="/tmp/plexi-scene-owner-$$"

if [[ "$channel" == "main" ]]; then
    plexi_bin="plexi"
else
    plexi_bin="plexi-$channel"
fi

cleanup_owned_host() {
    if [[ -f "$owner_file" ]]; then
        "$plexi_bin" host stop || true
        rm -f "$owner_file"
    fi
}

# Rust Drop handles normal completion and panic. This trap is the outer guard
# for SIGINT/SIGTERM/HUP: only a runner-created ownership marker authorizes it
# to stop the channel, so an explicitly attached host is never touched.
trap cleanup_owned_host EXIT INT TERM HUP

PLEXI_SCENE="$scene_file" \
PLEXI_SCENE_OUT="$out_dir" \
PLEXI_SCENE_NO_SHOTS=1 \
PLEXI_SCENE_BACKEND=live \
PLEXI_SCENE_CHANNEL="$channel" \
PLEXI_SCENE_OWNER_FILE="$owner_file" \
cargo test --bin plexi scene_single -- --ignored --exact scenes::tests::scene_single --nocapture
