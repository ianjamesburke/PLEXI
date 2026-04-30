#!/usr/bin/env bash
# migrate-config.sh — Append missing config sections to a Plexi config.toml.
#
# Usage: migrate-config.sh <config-path>
# Example: migrate-config.sh ~/.plexi-alpha/config.toml
#
# Idempotent: running twice never duplicates sections.
# Only appends — never modifies or removes existing values.

set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: migrate-config.sh <config-path>" >&2
  exit 1
fi

CONFIG="$1"

if [[ ! -f "$CONFIG" ]]; then
  echo "config: no config.toml found at $CONFIG — skipping migration"
  exit 0
fi

ADDED=0

# Check for [ai] section (also accepts legacy [iq] from before #429 rename).
# Users with [iq] already have equivalent config — don't duplicate.
if ! grep -q '^\[ai\]' "$CONFIG" && ! grep -q '^\[iq\]' "$CONFIG"; then
  cat >> "$CONFIG" << 'BLOCK'

# Added by installer — Plexi AI backend config
# Set OPENROUTER_API_KEY in your environment to use OpenRouter
[ai]
backend = "openrouter"

[ai.openrouter]
api_key_env = "OPENROUTER_API_KEY"
model_low    = "google/gemini-2.0-flash-001"
model_medium = "anthropic/claude-sonnet-4-6"
model_high   = "anthropic/claude-opus-4-7"

[ai.ollama]
host = "http://localhost:11434"
model_low    = "llama3.2:3b"
model_medium = "llama3.3:70b"
model_high   = "qwq:32b"
BLOCK
  echo "config: added [ai] section to $CONFIG"
  ADDED=$((ADDED + 1))
fi

# Add more section checks here as new required sections are introduced.

if [[ $ADDED -eq 0 ]]; then
  echo "config: up to date — no sections added"
fi
