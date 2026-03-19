#!/usr/bin/env bash
set -e

# Install cargo-bundle if not already installed
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
