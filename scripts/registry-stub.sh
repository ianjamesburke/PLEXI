#!/usr/bin/env bash
set -euo pipefail

PORT="${1:-8787}"

cargo run --locked -- registry stub --port "$PORT"
