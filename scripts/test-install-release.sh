#!/usr/bin/env bash
set -euo pipefail
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo"
case "$(uname -s)" in Darwin) os=macos ;; Linux) os=linux ;; *) echo 'Use test-distribution.py on Windows.' >&2; exit 1 ;; esac
case "$(uname -m)" in arm64|aarch64) arch=arm64 ;; *) arch=x64 ;; esac
bash scripts/cargo-with-lease.sh cargo build --release -p plexi-distribution --bin plexi-installer
uv run --no-project python scripts/test-distribution.py --installer target/release/plexi-installer --platform "$os-$arch"
