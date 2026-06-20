#!/usr/bin/env bash
# Asserts every docs page verified_version matches Cargo.toml [package].version.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CARGO_VERSION=$(grep -m1 '^version' "$REPO_ROOT/Cargo.toml" | sed 's/.*"\(.*\)".*/\1/')

if [[ -z "$CARGO_VERSION" ]]; then
  echo "ERROR: could not extract version from Cargo.toml"
  exit 1
fi

echo "Cargo.toml version: $CARGO_VERSION"

stale=0
for f in "$REPO_ROOT"/website/src/content/docs/*.md; do
  page_version=$(grep -m1 '^verified_version:' "$f" | sed 's/.*"\(.*\)".*/\1/')
  if [[ -z "$page_version" ]]; then
    echo "WARN: $f has no verified_version frontmatter"
    continue
  fi
  if [[ "$page_version" != "$CARGO_VERSION" ]]; then
    echo "STALE: $(basename "$f") verified_version=$page_version (expected $CARGO_VERSION)"
    stale=1
  fi
done

if [[ "$stale" -eq 1 ]]; then
  echo ""
  echo "ERROR: Some docs pages have a stale verified_version."
  echo "Update the verified_version frontmatter to \"$CARGO_VERSION\" after verifying accuracy."
  exit 1
fi

echo "All docs pages match Cargo.toml version $CARGO_VERSION."
