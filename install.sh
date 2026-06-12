#!/usr/bin/env bash
set -euo pipefail

REPO="https://github.com/ianjamesburke/PLEXI.git"

# Print a clear error message and the failing line on any unexpected exit.
_on_error() {
  local exit_code=$?
  local line=$1
  echo ""
  echo "ERROR: install failed (exit $exit_code) at line $line"
  echo ""
  echo "Common causes:"
  echo "  • Rust / cargo not installed — https://rustup.rs"
  echo "  • cargo-bundle install failed — try: cargo install cargo-bundle"
  echo "  • git clone failed — check your network connection"
  echo "  • cargo build failed — check the output above for compiler errors"
  echo ""
  echo "For help: https://github.com/ianjamesburke/PLEXI/issues"
}
trap '_on_error $LINENO' ERR

# macOS only
if [[ "$(uname)" != "Darwin" ]]; then
  echo "Error: Plexi is macOS-only. This installer does not support $(uname)."
  exit 1
fi

# Require git
if ! command -v git &>/dev/null; then
  echo "Error: 'git' is required but not found."
  echo "  Install Xcode Command Line Tools: xcode-select --install"
  exit 1
fi

# Require cargo — offer to install rustup if missing
if ! command -v cargo &>/dev/null; then
  echo "Rust is not installed. Plexi requires Rust to build."
  echo ""
  read -r -p "Install Rust now via rustup? [y/N] " _answer </dev/tty
  case "$_answer" in
    [yY][eE][sS]|[yY])
      echo "Installing Rust..."
      curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
      # Source the new cargo env so the rest of this script can use it
      # shellcheck source=/dev/null
      source "$HOME/.cargo/env"
      ;;
    *)
      echo ""
      echo "Skipping Rust install. Re-run this script after installing Rust:"
      echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
      exit 1
      ;;
  esac
fi

# If we're not inside the repo, clone it to a temp dir and build from there
if [ ! -f "Cargo.toml" ] || ! grep -q 'name = "plexi"' Cargo.toml 2>/dev/null; then
  TMP=$(mktemp -d)
  trap 'rm -rf "$TMP"' EXIT
  echo "Cloning Plexi..."
  if ! git clone --depth=1 "$REPO" "$TMP/PLEXI"; then
    echo "Error: git clone failed."
    echo "  Check your network connection and that $REPO is accessible."
    exit 1
  fi
  cd "$TMP/PLEXI"
fi

# Install cargo-bundle if needed
if ! command -v cargo-bundle &>/dev/null; then
  echo "Installing cargo-bundle..."
  if ! cargo install cargo-bundle; then
    echo "Error: 'cargo install cargo-bundle' failed."
    echo "  Check the output above for details."
    exit 1
  fi
fi

echo "Building Plexi.app (this may take a few minutes on first build)..."
if ! cargo bundle --release; then
  echo "Error: 'cargo bundle --release' failed."
  echo "  Check the compiler output above for details."
  exit 1
fi

APP_SRC=$(find target/release/bundle/osx -maxdepth 1 -name "*.app" 2>/dev/null | head -1)
APP_DEST="/Applications/Plexi.app"

if [ -z "$APP_SRC" ] || [ ! -d "$APP_SRC" ]; then
  echo "Error: bundle not found in target/release/bundle/osx/"
  echo "  'cargo bundle --release' appeared to succeed but produced no .app bundle."
  exit 1
fi

echo "Copying to /Applications..."
rm -rf "$APP_DEST"
cp -r "$APP_SRC" "$APP_DEST"

echo ""
echo "Done — Plexi.app installed to /Applications"
