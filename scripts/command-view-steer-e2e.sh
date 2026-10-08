#!/usr/bin/env bash
# V1-11 acceptance lives in scripts/command-view-e2e.sh.
exec "$(cd "$(dirname "$0")" && pwd)/command-view-e2e.sh" "$@"
