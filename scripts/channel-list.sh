#!/usr/bin/env bash
# List managed channels and canonical destinations, including custom roots.
set -euo pipefail
exec bash "$(dirname "${BASH_SOURCE[0]}")/distribution-tool.sh" list
