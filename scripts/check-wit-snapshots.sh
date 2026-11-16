#!/usr/bin/env bash
# Compare the PR changes so additions on either branch remain allowed.
set -euo pipefail

base="${1:?usage: check-wit-snapshots.sh BASE HEAD}"
head="${2:?usage: check-wit-snapshots.sh BASE HEAD}"
changed="$(git diff --name-only --no-renames --diff-filter=DMRTUXB "$base...$head" -- ':(glob)tests/wit-versions/*.sha256')"
if [[ -n "$changed" ]]; then
    echo "Existing WIT snapshots are immutable; bump the WIT package version and add a new snapshot."
    echo "$changed"
    exit 1
fi
echo "Existing WIT snapshots are unchanged."
