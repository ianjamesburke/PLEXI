#!/usr/bin/env bash
set -e

REPO="https://github.com/ianjamesburke/PLEXI.git"

# If we're not inside the repo, clone it to a temp dir and build from there
if [ ! -f "Cargo.toml" ] || ! grep -q 'name = "plexi"' Cargo.toml 2>/dev/null; then
  TMP=$(mktemp -d)
  trap 'rm -rf "$TMP"' EXIT
  echo "Cloning Plexi..."
  git clone --depth=1 "$REPO" "$TMP/PLEXI"
  cd "$TMP/PLEXI"
fi

# Install cargo-bundle if needed
if ! command -v cargo-bundle &>/dev/null; then
  echo "Installing cargo-bundle..."
  cargo install cargo-bundle
fi

echo "Building Plexi.app..."
cargo bundle --release

APP_SRC="target/release/bundle/osx/Plexi.app"
APP_DEST="/Applications/Plexi.app"

if [ ! -d "$APP_SRC" ]; then
  echo "Error: bundle not found at $APP_SRC"
  exit 1
fi

echo "Copying to /Applications..."
rm -rf "$APP_DEST"
cp -r "$APP_SRC" "$APP_DEST"

echo "Done — Plexi.app installed to /Applications"
