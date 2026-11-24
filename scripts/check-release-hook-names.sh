#!/usr/bin/env bash
# Scan the supplied executable as bytes, including strings embedded from docs.
set -eu
if [[ $# != 1 ]]; then
    echo "usage: check-release-hook-names.sh <readable nonempty binary>" >&2
    exit 2
fi

exec python3 "$(dirname "$0")/require_test_hook_binary.py" --release -- "$1"
