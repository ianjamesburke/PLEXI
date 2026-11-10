#!/usr/bin/env bash
# V1-11 acceptance lives in scripts/agents-dashboard-e2e.sh.
exec "$(cd "$(dirname "$0")" && pwd)/agents-dashboard-e2e.sh" "$@"
