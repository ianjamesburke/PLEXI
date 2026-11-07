#!/bin/sh
printf '\033[?25l\033[2J\033[H'
if [ "${1:-}" = cursor ]; then
    printf '\033[38;2;26;29;40;48;2;226;174;99m  \357\200\227\033[48;2;40;60;130m 02:37  \033[0m\r\n'
else
    printf '\033[38;2;26;29;40;48;2;226;174;99m  \357\200\227 02:37  \033[0m\r\n'
fi
printf 'CLOCK_READY\r\n'
if [ "${1:-}" = cursor ]; then printf '\033[1;3H\033[?25h'; fi
while IFS= read -r line; do :; done
