#!/usr/bin/env bash
# Repository tools query installation ownership through the native library.
set -euo pipefail
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec python3 "$repo/scripts/distribution-tool.py" "$@"
