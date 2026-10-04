#!/usr/bin/env bash
# Repository tools query installation ownership through the native library.
set -euo pipefail
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec cargo run --quiet --manifest-path "$repo/Cargo.toml" --release -p plexi-distribution --bin plexi-installer -- "$@"
