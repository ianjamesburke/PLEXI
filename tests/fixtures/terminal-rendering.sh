#!/bin/sh
# Static ANSI fixture for the real PTY renderer and installed-host review.
# Keep input open so the host can resize/capture without a shell prompt.
set -eu
printf '\033[?25l\033[0;38;2;174;184;204;48;2;8;15;23m\033[2J\033[H'
printf '  Terminal rendering / nooise-style controls\r\n\r\n'
printf '\033[1;33m  [Pads]   Perc   Bass   Kick   Tonal   Clap   Arp   Lead   Master\033[0;38;2;174;184;204;48;2;8;15;23m\r\n\r\n'
bar() {
    printf '  %-17s ' "$1"
    i=0
    while [ "$i" -lt 36 ]; do
        if [ "$i" -lt "$2" ]; then printf '█'; else printf '░'; fi
        i=$((i + 1))
    done
    printf '  %s\r\n\r\n' "$3"
}
printf '\033[1;38;2;117;223;255m'
bar '> Level' 25 '70%'
printf '\033[0;38;2;174;184;204;48;2;8;15;23m'
bar 'Attack' 19 '6.00 s'
bar 'Release' 25 '8.00 s'
bar 'Type' 0 'Warm'
bar 'Chord Length' 12 '16 beats'
bar 'Stereo Width' 29 '80%'
bar 'Detune' 18 '50%'
bar 'Reverb' 14 '40%'
printf '  Regular  \033[1mBold\033[22m  \033[2mFaint\033[1m + Bold\033[0;38;2;174;184;204;48;2;8;15;23m\r\n\r\n'
printf '  Shades: ░░░░░░  ▒▒▒▒▒▒  ▓▓▓▓▓▓  ██████\r\n\r\n'
printf '  \033[36mANSI cyan  \033[1mBold cyan  \033[22;38;5;14mIndexed 14  \033[7mInverse\033[27m\r\n\r\n'
printf '\033[0;38;2;174;184;204;48;2;8;15;23m  RENDER_FIXTURE_READY\r\n'
while IFS= read -r line; do :; done
