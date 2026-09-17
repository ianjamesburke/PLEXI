#!/bin/sh
# Real PTY probe. Run in a Plexi pane; press, repeat and release j to finish.
set -eu
stty raw -echo
trap 'printf "\033[<u\033[?1049l"; stty sane' EXIT

query() {
    printf '\033[?u'
    expected=$(printf '\033[?%su' "$1")
    reply=$(dd bs=1 count="${#expected}" 2>/dev/null)
    if [ "$reply" != "$expected" ]; then
        printf 'QUERY_FAILED expected flags=%s\r\n' "$1"
        exit 1
    fi
    printf 'QUERY_OK flags=%s\r\n' "$1"
}

query 0
printf '\033[>1u'
query 1
printf '\033[?1049h'
query 0
printf '\033[>10u'
query 10
printf '\033[>3u'
query 3
printf '\033[<u'
query 10
printf '\033[<u'
query 0
printf '\033[?1049l'
query 1
printf '\033[<u'
query 0

printf '\033[?1049h\033[>10u'
query 10
printf 'NEGOTIATION_OK: query, nested push/pop, independent screen stacks\r\n'
printf 'KITTY_READY: press, repeat, release j\r\n'
bytes=$(dd bs=1 count=30 2>/dev/null)
expected=$(printf '\033[106;1:1u\033[106;1:2u\033[106;1:3u')
if [ "$bytes" = "$expected" ]; then
    printf '\033[32mKEY_EVENTS_OK: j press -> repeat -> release, no duplicate text\033[0m\r\n'
else
    printf 'KEY_EVENTS_FAILED: '
    printf '%s' "$bytes" | od -An -tx1
fi
# Keep the result visible until the pane is closed.
while IFS= read -r line; do :; done
