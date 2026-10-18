#!/usr/bin/env bash
# Repository tools query installation ownership through the native library.
set -euo pipefail
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec bash "$repo/scripts/cargo-with-lease.sh" cargo run --quiet --manifest-path "$repo/Cargo.toml" --release -p plexi-distribution --bin plexi-installer -- "$@"

