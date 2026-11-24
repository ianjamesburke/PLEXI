#!/usr/bin/env bash
# Scan the supplied executable as bytes, including strings embedded from docs.
set -eu
if [[ $# != 1 || ! -f "$1" || ! -s "$1" || ! -r "$1" ]]; then
    echo "usage: check-release-hook-names.sh <readable nonempty binary>" >&2
    exit 2
fi

result=0
for name in PLEXI_KEYCHAIN_PATH PLEXI_KEYCHAIN_PASSWORD PLEXI_OPENROUTER_BASE_URL PLEXI_PICKER_SCRIPT; do
    if LC_ALL=C grep -aFq -e "$name" -- "$1"; then
        echo "FAIL: $1 contains $name" >&2
        result=1
    elif [[ $? != 1 ]]; then
        echo "could not scan $1" >&2
        exit 2
    fi
done
if [[ $result == 0 ]]; then
    echo "PASS: no test hook names in $1"
fi
exit "$result"
