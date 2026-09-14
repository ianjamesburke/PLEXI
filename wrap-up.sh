#!/usr/bin/env bash
# Usage: ./wrap-up.sh
# Opens the one-shot session handoff (RESUME.md at the main checkout root) in a
# named Plexi pane. Written by the wrap-up skill; rule: AGENTS.md → Session Resume.
set -euo pipefail

common_dir="$(git rev-parse --path-format=absolute --git-common-dir)" || { echo "error: run inside the PLEXI repo or a worktree of it" >&2; exit 1; }
root="$(dirname "$common_dir")"
resume="$root/RESUME.md"

[[ -f "$resume" ]] || { echo "error: no handoff at $resume (write it with the wrap-up skill first)" >&2; exit 1; }
command -v glow >/dev/null 2>&1 || { echo "error: glow not on PATH (brew install glow)" >&2; exit 1; }
command -v plexi >/dev/null 2>&1 || { echo "error: plexi CLI not on PATH" >&2; exit 1; }

plexi pane new "glow -p '$resume'" -n resume
