#!/usr/bin/env bash
# Guard wrapper for generator recipes that redirect a subprocess's stdout
# straight into a committed doc (`cargo run -p gen_x > website/.../x.md`).
# That pattern truncates the target file the instant the shell opens it for
# writing — before the wrapped command even runs — so a failed lease
# acquisition, a cargo build error, or a panicking generator silently leaves
# the committed file empty. Capture to a temp file first and only replace the
# real output if the command exits 0 AND produces non-empty output.
#
# Usage: gen-doc-safe.sh <output-file> -- <command...>
set -euo pipefail

if [[ $# -lt 2 ]]; then
    echo "usage: gen-doc-safe.sh <output-file> -- <command...>" >&2
    exit 2
fi

out="$1"
shift
if [[ "$1" != "--" ]]; then
    echo "gen-doc-safe: expected -- before command, got '$1'" >&2
    exit 2
fi
shift

tmp="$(mktemp "${out}.XXXXXX")"
trap 'rm -f "$tmp"' EXIT

"$@" > "$tmp"

if [[ ! -s "$tmp" ]]; then
    echo "ERROR: '$*' produced empty output; leaving $out untouched" >&2
    exit 1
fi

mv "$tmp" "$out"
