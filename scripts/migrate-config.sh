#!/usr/bin/env bash
# Additive config migration: ensures required top-level TOML sections exist.
# Usage: migrate-config.sh <config-path> [section...] e.g. "[ai]" "[notifications]"
# Never modifies existing values — only appends missing sections.
set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "usage: migrate-config.sh <config-path> [section...]" >&2
  exit 1
fi

config="$1"
shift
sections=("$@")

if [[ ! -f "$config" ]]; then
  echo "config: $config not found, skipping migration" >&2
  exit 0
fi

added=()

for section in "${sections[@]}"; do
  # Match the section header as a whole line (handles "[ai]" not matching "[ai.openrouter]")
  pattern="^\[${section:1:-1}\]"
  if ! grep -qE "$pattern" "$config"; then
    printf '\n# Added by installer — see docs for available options\n%s\n' "$section" >> "$config"
    added+=("$section")
  fi
done

if [[ ${#added[@]} -eq 0 ]]; then
  echo "config: all sections present"
else
  echo "config: added missing sections: ${added[*]}"
fi
