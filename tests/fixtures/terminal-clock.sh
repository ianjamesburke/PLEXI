#!/bin/sh
printf '\033[?25l\033[2J\033[H'
printf '\033[38;2;26;29;40;48;2;226;174;99m  \357\200\227 02:37  \033[0m\r\n'
printf 'CLOCK_READY\r\n'
while IFS= read -r line; do :; done
