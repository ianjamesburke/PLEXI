#!/usr/bin/env bash
# Asserts every non-hidden top-level CLI command is mentioned in at least one
# docs page under website/src/content/docs/ (excluding cli.md, which is
# auto-generated and covers everything by definition).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DOCS_DIR="$REPO_ROOT/website/src/content/docs"

# Commands that intentionally have no standalone docs page. Each must have a
# comment explaining where the command is documented instead.
SKIP=(
  "account"      # marketplace login, no standalone docs yet
  "agent"        # agent definitions, no standalone docs yet
  "ai"           # AI config, no standalone docs yet
  "completions"  # internal shell integration, documented in getting-started
  "context"      # context scoping, no standalone docs yet
  "demo"         # self-documenting interactive tutorial
  "doctor"       # diagnostic tool, no standalone docs yet
  "events"       # developer API, covered in sdk-emitter
  "note"         # alias for notes, covered in quick-note
  "notify"       # developer API, covered in sdk-emitter
  "registry"     # internal developer tool
  "routine"      # scheduled commands, no standalone docs yet
  "run"          # workspace commands, no standalone docs yet
  "uninstall"    # single command, covered in getting-started
  "update"       # single command, covered in getting-started
  "workspace"    # covered in getting-started
)

is_skipped() {
  local cmd="$1"
  for s in "${SKIP[@]}"; do
    [[ "$cmd" == "$s" ]] && return 0
  done
  return 1
}

cli_md="$DOCS_DIR/cli.md"
if [[ ! -f "$cli_md" ]]; then
  echo "ERROR: $cli_md not found. Run 'just gen-cli-docs' first."
  exit 1
fi

commands=$(grep '^## ' "$cli_md" | sed 's/## `plexi //' | sed 's/`.*//' | sort)

missing=0
for cmd in $commands; do
  if is_skipped "$cmd"; then
    continue
  fi
  # Search all docs pages except cli.md for any mention of the command
  found=0
  for doc in "$DOCS_DIR"/*.md; do
    [[ "$(basename "$doc")" == "cli.md" ]] && continue
    if grep -qi "plexi $cmd\|plexi-$cmd\|\`$cmd\`\| $cmd " "$doc" 2>/dev/null; then
      found=1
      break
    fi
  done
  if [[ "$found" -eq 0 ]]; then
    echo "UNDOCUMENTED: plexi $cmd has no docs page mentioning it"
    missing=1
  fi
done

if [[ "$missing" -eq 1 ]]; then
  echo ""
  echo "ERROR: Some CLI commands are not covered by any docs page."
  echo "Add documentation or add the command to the SKIP list with justification."
  exit 1
fi

echo "All CLI commands have docs coverage."
