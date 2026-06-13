#!/usr/bin/env bash
# Usage:
#   curl -fsSL https://plexiapp.com/install.sh | bash
#   curl -fsSL https://plexiapp.com/install.sh | bash -s -- --channel alpha
#   curl -fsSL https://plexiapp.com/install.sh | bash -s -- --channel beta
set -euo pipefail

REPO="https://github.com/ianjamesburke/PLEXI.git"
CHANNEL="main"

# Parse flags
while [[ $# -gt 0 ]]; do
  case "$1" in
    --channel|-c)
      CHANNEL="${2:-}"
      if [[ -z "$CHANNEL" ]]; then
        echo "Error: --channel requires a value (main, alpha, beta)."
        exit 1
      fi
      shift 2
      ;;
    --channel=*|-c=*)
      CHANNEL="${1#*=}"
      shift
      ;;
    --help|-h)
      echo "Usage: install.sh [--channel main|alpha|beta]"
      echo ""
      echo "  --channel, -c   Which release channel to install (default: main)"
      echo "                  main   — stable release"
      echo "                  beta   — staging / pre-release"
      echo "                  alpha  — active development"
      exit 0
      ;;
    *)
      echo "Error: unknown argument '$1'. Run with --help for usage."
      exit 1
      ;;
  esac
done

# Validate channel
case "$CHANNEL" in
  main|alpha|beta) ;;
  *)
    echo "Error: invalid channel '$CHANNEL'. Must be one of: main, alpha, beta."
    exit 1
    ;;
esac

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

# If we're not inside the repo, clone the requested branch to a temp dir
if [ ! -f "Cargo.toml" ] || ! grep -q 'name = "plexi"' Cargo.toml 2>/dev/null; then
  TMP=$(mktemp -d)
  trap 'rm -rf "$TMP"' EXIT
  echo "Cloning Plexi ($CHANNEL)..."
  if ! git clone --depth=1 --branch "$CHANNEL" "$REPO" "$TMP/PLEXI"; then
    echo "Error: git clone failed for branch '$CHANNEL'."
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

# Delegate to the full installer which handles per-channel binary naming,
# symlinks, SDK deployment, config seeding, completions, and skills.
bash scripts/install.sh "$CHANNEL"
